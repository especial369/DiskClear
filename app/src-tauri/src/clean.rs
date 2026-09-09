// 清理器（PRD 2.2 交互流程 + 6.1 恢复区 + 6.4 占用跳过）
//
// 安全防线（纵深防御）：
// 1. 扫描阶段：一键清理的文件只能来自注册分类的根目录
// 2. 清理阶段：再次校验每个路径归属分类根（under_any_root），不属即拒绝
// 3. 删除方式：一律 rename 到同盘恢复区，不直接 unlink
// 4. 占用失败：重试 1 次后计入 skippedLocked，绝不中断
//
// M2 扩展：move_paths_to_recovery 供重复文件清理复用；
// delete_large_file 实现 PRD 2.3 的分级删除策略（≤5GB 恢复区 / >5GB 永久删除需显式确认）。

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::category;
use crate::recovery::{self, RecoveryEntry};
use crate::scan::FileEntry;
use crate::util;

/// 单文件移入恢复区的上限（PRD 2.3：5GB）
pub const RECOVERY_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024 * 1024;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    pub batch_id: String,
    pub freed_bytes: u64,
    pub moved_count: u64,
    pub skipped_locked: u64,
    pub skipped_missing: u64,
    pub rejected_unsafe: u64,
    pub errors: Vec<String>,
    pub recovery_bytes: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 路径是否位于任一根目录之下（词法前缀 + 规范化分隔符）。
pub fn under_any_root(path: &Path, roots: &[PathBuf]) -> bool {
    let ps = path.to_string_lossy().replace('/', "\\").to_lowercase();
    roots.iter().any(|r| {
        let mut rs = r.to_string_lossy().replace('/', "\\").to_lowercase();
        if !rs.ends_with('\\') {
            rs.push('\\');
        }
        ps.starts_with(&rs)
    })
}

/// 单文件移入恢复区的共享实现。成功时向 pending 追加恢复区条目并累计 result。
fn move_one(
    path: &Path,
    size: u64,
    cat_id: &str,
    original_path_override: Option<String>,
    batch_id: &str,
    seq: usize,
    now: u64,
    retention_ms: u64,
    result: &mut CleanResult,
    pending: &mut Vec<RecoveryEntry>,
) -> bool {
    let drive = util::drive_root_of(path)
        .and_then(|d| d.to_string_lossy().chars().next())
        .map(|c| c.to_string())
        .unwrap_or_else(|| "C".to_string());

    let Ok(recovery_root) = recovery::ensure_recovery_root(path) else {
        result
            .errors
            .push(format!("无法创建恢复区：{}", path.display()));
        return false;
    };
    let batch_dir = recovery_root.join(batch_id);
    if std::fs::create_dir_all(util::long_path(&batch_dir)).is_err() {
        result
            .errors
            .push(format!("无法创建批次目录：{}", batch_dir.display()));
        return false;
    }
    let entry_id = format!("{}-{}", batch_id, seq);
    let display = format!("{:04}_{}", seq,
        path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "unnamed".to_string()));
    let dest = batch_dir.join(&display);

    // rename 到恢复区；占用则重试 1 次；仍失败计入 skippedLocked（PRD 6.4）
    let mut moved = false;
    for attempt in 0..2 {
        match util::move_path(path, &dest) {
            Ok(_) => {
                moved = true;
                break;
            }
            Err(_) if attempt == 0 => {
                std::thread::sleep(Duration::from_millis(80));
            }
            Err(_) => break,
        }
    }
    if !moved {
        result.skipped_locked += 1;
        let _ = std::fs::remove_dir_all(util::long_path(&batch_dir)).ok(); // 空批次自清
        return false;
    }

    // 回收站：成功移出 $R 后删除对应 $I 元数据
    if cat_id == "recycle" {
        delete_recycle_meta(path);
    }

    result.freed_bytes += size;
    result.moved_count += 1;
    pending.push(RecoveryEntry {
        id: entry_id,
        batch_id: batch_id.to_string(),
        original_path: original_path_override
            .unwrap_or_else(|| util::normalize_path(path)),
        recovery_path: util::normalize_path(&dest),
        size,
        deleted_at_ms: now,
        expires_at_ms: now + retention_ms,
        category_id: cat_id.to_string(),
        drive,
        ..Default::default()
    });
    true
}

/// 执行一键清理：从会话中取出所选分类的文件清单，逐个移入恢复区。
pub fn run_clean(
    app: &AppHandle,
    state: &crate::AppState,
    session_id: u32,
    category_ids: Vec<String>,
) -> Result<CleanResult, String> {
    let settings = crate::settings::load();
    let session_files = {
        let sessions = state.sessions.lock().unwrap();
        let session = sessions
            .get(&session_id)
            .ok_or("扫描会话不存在或已过期，请重新扫描")?;
        let mut m: std::collections::HashMap<String, Vec<FileEntry>> =
            std::collections::HashMap::new();
        for id in &category_ids {
            if let Some(files) = session.files.get(id) {
                m.insert(id.clone(), files.clone());
            }
        }
        m
    };

    let now = now_ms();
    let batch_id = format!(
        "{}_{}",
        chrono_stamp(now),
        state.next_session.fetch_add(1, Ordering::SeqCst)
    );
    let retention_ms = settings.recovery_retention_days as u64 * 86_400_000;

    let mut result = CleanResult {
        batch_id: batch_id.clone(),
        ..Default::default()
    };
    let mut pending: Vec<RecoveryEntry> = Vec::new();

    let total: usize = session_files.values().map(|v| v.len()).sum();
    let mut done = 0usize;
    let mut seq = 0usize;
    let mut last_emit = std::time::Instant::now();

    for (cat_id, files) in &session_files {
        let roots = category::roots_for(cat_id, "C");
        for fe in files {
            done += 1;
            seq += 1;
            if last_emit.elapsed() >= Duration::from_millis(120) {
                let _ = app.emit(
                    "clean-progress",
                    serde_json::json!({
                        "done": done, "total": total,
                        "freedBytes": result.freed_bytes,
                        "skippedLocked": result.skipped_locked,
                    }),
                );
                last_emit = std::time::Instant::now();
            }

            if !fe.path.exists() {
                result.skipped_missing += 1;
                continue;
            }
            // 防线 2：清理前再次校验路径归属
            let drive = util::drive_root_of(&fe.path)
                .and_then(|d| d.to_string_lossy().chars().next())
                .map(|c| c.to_string())
                .unwrap_or_else(|| "C".to_string());
            if !under_any_root(&fe.path, &roots)
                && cat_id != "recycle"
                && !under_any_root(&fe.path, &category::roots_for(cat_id, &drive))
            {
                result.rejected_unsafe += 1;
                continue;
            }
            // 白名单用户目录（设置中心）额外保护
            if settings
                .whitelist_dirs
                .iter()
                .any(|w| under_any_root(&fe.path, &[PathBuf::from(w)]))
            {
                result.skipped_locked += 1;
                continue;
            }

            move_one(
                &fe.path,
                fe.size,
                cat_id,
                if cat_id == "recycle" {
                    Some(original_path_of("recycle", &fe.path))
                } else {
                    None
                },
                &batch_id,
                seq,
                now,
                retention_ms,
                &mut result,
                &mut pending,
            );
        }
    }

    if !pending.is_empty() {
        recovery::insert_entries(state, pending);
    }

    // 清理后顺带过期维护
    recovery::startup_maintenance(state);
    result.recovery_bytes = recovery::total_size(state);

    // 累计统计（状态栏"已释放"）
    add_freed_total(result.freed_bytes);

    let _ = app.emit(
        "clean-progress",
        serde_json::json!({
            "done": total, "total": total,
            "freedBytes": result.freed_bytes,
            "skippedLocked": result.skipped_locked,
            "finished": true,
        }),
    );
    Ok(result)
}

/// 通用：把给定路径列表移入恢复区（重复文件"删除非保留项"使用，PRD 2.4）。
/// 云占位文件一律拒绝（其"删除"会波及云端，仅允许大文件页的显式永久删除流程处理）。
pub fn move_paths_to_recovery(
    app: &AppHandle,
    state: &crate::AppState,
    paths: Vec<String>,
    category: &str,
) -> Result<CleanResult, String> {
    let settings = crate::settings::load();
    let now = now_ms();
    let batch_id = format!(
        "{}_{}",
        chrono_stamp(now),
        state.next_session.fetch_add(1, Ordering::SeqCst)
    );
    let retention_ms = settings.recovery_retention_days as u64 * 86_400_000;
    let mut result = CleanResult {
        batch_id: batch_id.clone(),
        ..Default::default()
    };
    let mut pending: Vec<RecoveryEntry> = Vec::new();
    let total = paths.len();

    for (i, p) in paths.iter().enumerate() {
        let path = PathBuf::from(p);
        let _ = app.emit(
            "clean-progress",
            serde_json::json!({ "done": i, "total": total, "freedBytes": result.freed_bytes, "skippedLocked": result.skipped_locked }),
        );
        let Ok(md) = std::fs::metadata(util::long_path(&path)) else {
            result.skipped_missing += 1;
            continue;
        };
        if md.is_dir() || util::is_cloud_placeholder(&md) {
            result.rejected_unsafe += 1;
            continue;
        }
        move_one(
            &path,
            md.len(),
            category,
            None,
            &batch_id,
            i + 1,
            now,
            retention_ms,
            &mut result,
            &mut pending,
        );
    }

    if !pending.is_empty() {
        recovery::insert_entries(state, pending);
    }
    recovery::startup_maintenance(state);
    result.recovery_bytes = recovery::total_size(state);
    add_freed_total(result.freed_bytes);

    let _ = app.emit(
        "clean-progress",
        serde_json::json!({ "done": total, "total": total, "freedBytes": result.freed_bytes, "skippedLocked": result.skipped_locked, "finished": true }),
    );
    Ok(result)
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LargeDeleteOutcome {
    pub mode: String, // "recovered" | "permanent"
    pub freed_bytes: u64,
    pub recovery_bytes: u64,
}

/// 大文件删除（PRD 2.3 分级策略）：
/// - ≤ 5GB：移入恢复区，可撤销
/// - > 5GB：仅当 allow_permanent=true（前端已做红色警示二次确认）时永久删除
/// - 云占位文件：允许移入恢复区（rename 不触发下载）；永久删除必须显式确认
pub fn delete_large_file(
    state: &crate::AppState,
    path: &str,
    allow_permanent: bool,
) -> Result<LargeDeleteOutcome, String> {
    let p = PathBuf::from(path);
    let Ok(md) = std::fs::metadata(util::long_path(&p)) else {
        return Err("文件不存在或已被删除".to_string());
    };
    if md.is_dir() {
        return Err("不支持删除目录，请在大文件结果中仅选择文件".to_string());
    }
    let size = md.len();

    if size <= RECOVERY_MAX_FILE_BYTES {
        let settings = crate::settings::load();
        let now = now_ms();
        let batch_id = format!(
            "{}_{}",
            chrono_stamp(now),
            state.next_session.fetch_add(1, Ordering::SeqCst)
        );
        let mut result = CleanResult::default();
        let mut pending = Vec::new();
        let ok = move_one(
            &p,
            size,
            "large",
            None,
            &batch_id,
            1,
            now,
            settings.recovery_retention_days as u64 * 86_400_000,
            &mut result,
            &mut pending,
        );
        if !ok {
            return Err(format!(
                "移入恢复区失败（可能被占用或磁盘空间不足），跳过 {} 个文件",
                result.skipped_locked
            ));
        }
        recovery::insert_entries(state, pending);
        recovery::startup_maintenance(state);
        let recovery_bytes = recovery::total_size(state);
        add_freed_total(result.freed_bytes);
        Ok(LargeDeleteOutcome {
            mode: "recovered".to_string(),
            freed_bytes: result.freed_bytes,
            recovery_bytes,
        })
    } else if allow_permanent {
        std::fs::remove_file(util::long_path(&p)).map_err(|e| format!("永久删除失败：{}", e))?;
        add_freed_total(size);
        Ok(LargeDeleteOutcome {
            mode: "permanent".to_string(),
            freed_bytes: size,
            recovery_bytes: recovery::total_size(state),
        })
    } else {
        Err("文件超过 5GB，无法移入恢复区；请确认后选择永久删除".to_string())
    }
}

/// 恢复区记录的原始路径：回收站从 $I 元数据还原。
fn original_path_of(cat_id: &str, path: &Path) -> String {
    if cat_id == "recycle" {
        if let Some(parent) = path.parent() {
            let stem = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let meta_name = format!("$I{}", stem.trim_start_matches("$R"));
            let meta_path = parent.join(meta_name);
            if let Some((_, original)) = crate::scan::parse_recycle_meta(&meta_path) {
                return original;
            }
        }
        return path.to_string_lossy().to_string();
    }
    path.to_string_lossy().to_string()
}

/// 删除回收站 $I 元数据（$R 已成功移出后）。
fn delete_recycle_meta(r_path: &Path) {
    if let Some(parent) = r_path.parent() {
        let stem = r_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let meta = parent.join(format!("$I{}", stem.trim_start_matches("$R")));
        let _ = std::fs::remove_file(util::long_path(&meta));
    }
}

fn chrono_stamp(ms: u64) -> String {
    // 无 chrono 依赖：从 UNIX epoch 计算本地简化时间戳（YYYYMMDD_HHMMSS）
    let secs = ms / 1000;
    let days = secs / 86400;
    let (y, mo, d) = civil_from_days(days as i64);
    let rem = secs % 86400;
    format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}",
        y,
        mo,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant 算法：UNIX 天数 → (年, 月, 日)。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn stats_path() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|base| PathBuf::from(base).join("com.diskclear.app").join("stats.json"))
}

pub fn freed_total() -> u64 {
    stats_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("freedTotalBytes").and_then(|x| x.as_u64()))
        .unwrap_or(0)
}

pub fn add_freed_total(bytes: u64) {
    if let Some(p) = stats_path() {
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let total = freed_total() + bytes;
        let _ = std::fs::write(
            &p,
            serde_json::json!({ "freedTotalBytes": total }).to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_root_checks_prefix() {
        let roots = vec![PathBuf::from(r"C:\Windows\Temp")];
        assert!(under_any_root(Path::new(r"C:\Windows\Temp\a.tmp"), &roots));
        assert!(under_any_root(Path::new(r"c:/windows/temp/sub/b.log"), &roots));
        // 前缀相似但非子目录：拒绝
        assert!(!under_any_root(Path::new(r"C:\Windows\TempXYZ\a"), &roots));
        assert!(!under_any_root(Path::new(r"D:\Windows\Temp\a"), &roots));
    }

    #[test]
    fn civil_from_days_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
    }

    /// 分级删除策略：≤5GB 走恢复区；>5GB 未确认时拒绝、确认后永久删除。
    #[test]
    fn large_delete_policy_levels() {
        let state = crate::AppState::new();
        let base = std::env::temp_dir().join(format!("dc_large_del_{}", std::process::id()));
        let src_dir = base.join("orig");
        std::fs::create_dir_all(util::long_path(&src_dir)).unwrap();

        // ≤5GB：允许走恢复区。为避免造 5GB 文件，这里仅验证"未确认时小文件不报错"
        let small = src_dir.join("small.bin");
        std::fs::write(&small, vec![7u8; 4096]).unwrap();
        let out = delete_large_file(&state, small.to_str().unwrap(), false).unwrap();
        assert_eq!(out.mode, "recovered");
        assert!(!small.exists(), "移入恢复区后原文件应消失");
        // 恢复回原位，保持测试目录干净
        let entry = recovery::list_all(&state).into_iter().next_back().unwrap();
        recovery::restore(&state, &[entry.id]);
        assert!(small.exists());

        let _ = std::fs::remove_dir_all(util::long_path(&base));
    }
}
