// 扫描器（PRD 2.2 / 5.3 / 6.4 / 6.5）
//
// - 迭代式遍历（栈实现，防深目录栈溢出）
// - 不跟随 reparse point（junction/符号链接），防循环与重复统计（PRD 6.5）
// - 跳过 OneDrive 云占位文件（PRD 6.5）
// - 扫描期仅聚合，文件明细驻留会话内存（M1 范围内分类均为缓存目录，量级可控；
//   全盘扫描的聚合分页属 M2 大文件模块）
// - 回收站按 $I 元数据解析原始路径

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::category::{self, CategoryDef};
use crate::util;

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CategoryResult {
    pub id: String,
    pub name: String,
    pub desc: String,
    pub risk: String,
    pub file_count: u64,
    pub total_size: u64,
    pub needs_admin: bool,
    /// true = 该分类当前不可清理（权限不足），前端禁用勾选
    pub disabled: bool,
    pub warning: Option<String>,
}

pub struct ScanSession {
    pub id: u32,
    pub drive: String,
    pub files: HashMap<String, Vec<FileEntry>>,
    pub results: Vec<CategoryResult>,
    pub cancelled: Arc<AtomicBool>,
    pub done: bool,
}

const EVENT_PROGRESS: &str = "scan-progress";
const EVENT_DONE: &str = "scan-done";

/// 启动一次扫描：立即返回会话 id，后台线程执行并通过事件汇报进度。
pub fn start_scan(app: AppHandle, state: &crate::AppState, drive: String) -> u32 {
    let id = state
        .next_session
        .fetch_add(1, Ordering::SeqCst);
    let cancelled = Arc::new(AtomicBool::new(false));

    // 仅保留最近 3 个会话
    {
        let mut sessions = state.sessions.lock().unwrap();
        while sessions.len() >= 3 {
            let oldest = sessions.keys().copied().min().unwrap();
            sessions.remove(&oldest);
        }
    }
    state.cancels.lock().unwrap().insert(id, cancelled.clone());

    let app2 = app.clone();
    let cancelled2 = cancelled.clone();
    std::thread::spawn(move || {
        let session = run_scan(app2.clone(), id, drive, cancelled2);
        let state = app2.state::<crate::AppState>();
        let mut sessions = state.sessions.lock().unwrap();
        sessions.insert(id, session);
    });
    id
}

pub fn cancel_scan(state: &crate::AppState, session: u32) {
    if let Some(c) = state.cancels.lock().unwrap().get(&session) {
        c.store(true, Ordering::SeqCst);
    }
}

fn run_scan(app: AppHandle, id: u32, drive: String, cancelled: Arc<AtomicBool>) -> ScanSession {
    let is_admin = util::is_admin();
    let browsers = util::running_browsers();
    let wuauserv = util::wuauserv_running();

    let mut files: HashMap<String, Vec<FileEntry>> = HashMap::new();
    let mut results: Vec<CategoryResult> = Vec::new();
    let total = category::CATEGORIES.len();

    for (idx, cat) in category::CATEGORIES.iter().enumerate() {
        if cancelled.load(Ordering::SeqCst) {
            break;
        }
        let mut entries: Vec<FileEntry> = Vec::new();
        let roots = category::roots_for(cat.id, &drive);

        // 盘符隔离：该分类在所选盘无扫描根（如非系统盘的系统级垃圾）则跳过，不纳入结果，
        // 避免把系统盘 C 的垃圾混入其他盘的扫描结果。
        if roots.is_empty() {
            continue;
        }

        if cat.id == "recycle" {
            scan_recycle(&roots, &mut entries);
        } else {
            for root in &roots {
                walk(root, cat, &mut entries, &cancelled);
            }
        }

        let (warning, disabled) = warnings_for(cat, is_admin, &browsers, wuauserv);
        results.push(CategoryResult {
            id: cat.id.to_string(),
            name: cat.name.to_string(),
            desc: cat.desc.to_string(),
            risk: format!("{:?}", cat.risk).to_lowercase(),
            file_count: entries.len() as u64,
            total_size: entries.iter().map(|e| e.size).sum(),
            needs_admin: cat.needs_admin,
            disabled,
            warning,
        });
        files.insert(cat.id.to_string(), entries);

        let _ = app.emit(
            EVENT_PROGRESS,
            serde_json::json!({
                "sessionId": id,
                "categoryIndex": idx + 1,
                "totalCategories": total,
                "categoryName": cat.name,
                "foundFiles": results.iter().map(|r| r.file_count).sum::<u64>(),
                "foundSize": results.iter().map(|r| r.total_size).sum::<u64>(),
                "cancelled": false,
            }),
        );
    }

    let _ = app.emit(
        EVENT_DONE,
        serde_json::json!({ "sessionId": id, "cancelled": cancelled.load(Ordering::SeqCst) }),
    );

    ScanSession {
        id,
        drive,
        files,
        results,
        cancelled,
        done: true,
    }
}

fn warnings_for(
    cat: &CategoryDef,
    is_admin: bool,
    browsers: &[String],
    wuauserv: bool,
) -> (Option<String>, bool) {
    match cat.id {
        "browser" => {
            if browsers.is_empty() {
                (None, false)
            } else {
                (
                    Some(format!(
                        "检测到浏览器正在运行（{}），运行中被锁定的缓存文件将自动跳过",
                        browsers.join("、")
                    )),
                    false,
                )
            }
        }
        "update" => {
            if !is_admin {
                (
                    Some("需要管理员权限才能清理 Windows 更新缓存".to_string()),
                    true,
                )
            } else if wuauserv {
                (
                    Some("Windows 更新服务运行中，被占用的文件将自动跳过".to_string()),
                    false,
                )
            } else {
                (None, false)
            }
        }
        "logs" | "temp" => {
            if !is_admin && cat.id == "logs" {
                (
                    Some("非管理员模式下部分系统日志无法删除，将自动跳过".to_string()),
                    false,
                )
            } else {
                (None, false)
            }
        }
        _ => (None, false),
    }
}

/// 迭代式目录遍历。返回 false 的取消信号由闭包内控制（这里通过外部 flag）。
fn walk(root: &Path, cat: &CategoryDef, out: &mut Vec<FileEntry>, cancelled: &AtomicBool) {
    if !root.exists() {
        return;
    }
    let mut stack = vec![root.to_path_buf()];
    let mut last_yield = Instant::now();
    while let Some(dir) = stack.pop() {
        if cancelled.load(Ordering::SeqCst) {
            return;
        }
        let Ok(rd) = std::fs::read_dir(util::long_path(&dir)) else {
            continue; // 无权限/已消失：跳过并继续（PRD 6.3）
        };
        for entry in rd.flatten() {
            if cancelled.load(Ordering::SeqCst) {
                return;
            }
            // 高频取消检测 + 让出 CPU
            if last_yield.elapsed() > Duration::from_millis(50) {
                std::thread::yield_now();
                last_yield = Instant::now();
            }
            let Ok(md) = entry.metadata() else { continue };
            let path = entry.path();
            if md.is_dir() {
                // 不跟随 junction/符号链接（PRD 6.5）
                if !util::is_reparse_dir(&md) {
                    stack.push(path);
                }
            } else if md.is_file() {
                // 云占位文件不纳入清理（PRD 6.5）
                if util::is_cloud_placeholder(&md) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if category::matches_name(cat, &name) {
                    out.push(FileEntry {
                        path,
                        size: md.len(),
                    });
                }
            }
        }
    }
}

/// 回收站扫描：遍历 <drive>:\$Recycle.Bin\<SID>\$Ixxxxx 元数据文件，
/// 解析原始路径与大小，映射到 $Rxxxxx 实体（文件或目录）。
fn scan_recycle(roots: &[PathBuf], out: &mut Vec<FileEntry>) {
    let Some(root) = roots.first() else { return };
    let Ok(sids) = std::fs::read_dir(util::long_path(root)) else {
        return;
    };
    for sid in sids.flatten() {
        if !sid.path().is_dir() {
            continue;
        }
        let Ok(items) = std::fs::read_dir(util::long_path(&sid.path())) else {
            continue; // 其他用户的 SID 目录无权限，跳过
        };
        for item in items.flatten() {
            let name = item.file_name().to_string_lossy().to_string();
            if !name.starts_with("$I") {
                continue;
            }
            let r_path = sid.path().join(format!(
                "$R{}",
                name.trim_start_matches("$I")
            ));
            if !r_path.exists() {
                continue;
            }
            let (size, _original) = match parse_recycle_meta(&item.path()) {
                Some((size, original)) => (size, Some(original)),
                None => (
                    if r_path.is_dir() {
                        util::dir_size(&r_path)
                    } else {
                        item.metadata().map(|m| m.len()).unwrap_or(0)
                    },
                    None,
                ),
            };
            out.push(FileEntry { path: r_path, size });
        }
    }
}

/// 解析回收站 $I 元数据：返回 (原始大小, 原始路径)。
/// v1（Win8 及以下）：8B 版本 + 8B 大小 + 8B 时间 + 520B 定长 UTF-16 路径
/// v2+（Win10+）：8B 版本 + 8B 大小 + 8B 时间 + 4B 名长 + 变长 UTF-16 路径
pub fn parse_recycle_meta(path: &Path) -> Option<(u64, String)> {
    let bytes = std::fs::read(util::long_path(path)).ok()?;
    if bytes.len() < 24 {
        return None;
    }
    let version = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
    let size = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    let original = if version == 1 {
        if bytes.len() < 24 + 520 {
            return None;
        }
        decode_utf16_nul(&bytes[24..24 + 520])?
    } else if (2..=4).contains(&version) {
        let name_len = u32::from_le_bytes(bytes[24..28].try_into().ok()?) as usize;
        let end = 28 + name_len * 2;
        if bytes.len() < end {
            return None;
        }
        decode_utf16_nul(&bytes[28..end])?
    } else {
        return None;
    };
    Some((size, original))
}

fn decode_utf16_nul(bytes: &[u8]) -> Option<String> {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16(&units).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_meta_v(version: u64, size: u64, original: &str) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&version.to_le_bytes());
        b.extend_from_slice(&size.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes()); // filetime
        let units: Vec<u16> = original.encode_utf16().collect();
        if version == 1 {
            let mut fixed = units.clone();
            fixed.resize(260, 0); // 520 字节
            for u in fixed {
                b.extend_from_slice(&u.to_le_bytes());
            }
        } else {
            b.extend_from_slice(&(units.len() as u32).to_le_bytes());
            for u in units {
                b.extend_from_slice(&u.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn parse_recycle_meta_v2() {
        let dir = std::env::temp_dir().join("dc_test_meta");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("$Itest1");
        std::fs::write(&p, build_meta_v(2, 12345, r"D:\docs\报告.docx")).unwrap();
        let (size, original) = parse_recycle_meta(&p).unwrap();
        assert_eq!(size, 12345);
        assert_eq!(original, r"D:\docs\报告.docx");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parse_recycle_meta_v1() {
        let dir = std::env::temp_dir().join("dc_test_meta");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("$Itest2");
        std::fs::write(&p, build_meta_v(1, 999, r"C:\a.txt")).unwrap();
        let (size, original) = parse_recycle_meta(&p).unwrap();
        assert_eq!(size, 999);
        assert_eq!(original, r"C:\a.txt");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parse_recycle_meta_garbage() {
        let dir = std::env::temp_dir().join("dc_test_meta");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("$Itest3");
        std::fs::write(&p, [0u8; 10]).unwrap();
        assert!(parse_recycle_meta(&p).is_none());
        assert!(parse_recycle_meta(Path::new(r"C:\no\such\file")).is_none());
        let _ = std::fs::remove_file(&p);
    }
}
