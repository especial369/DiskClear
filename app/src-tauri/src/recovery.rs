// 恢复区（PRD 6.1，v1.1 按盘符分布方案）
//
// - 位置：<被清理文件所在盘>:\$DiskClear\Recovery\<批次id>\（同盘 rename 零拷贝）
//   C 盘无盘根写权限时回退 %LOCALAPPDATA%\DiskClear\Recovery\
// - 每个恢复区根一份 index.json；恢复区目录设隐藏+系统属性
// - 自愈：目录被外部删除时重建；索引缺失时从磁盘孤儿目录重建条目
// - 过期：按保留天数（默认 7 天）清除

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::util;

const DIR_NAME: &str = r"$DiskClear\Recovery";
const FALLBACK: &str = r"DiskClear\Recovery";
pub const INDEX_FILE: &str = "index.json";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct RecoveryEntry {
    pub id: String,
    pub batch_id: String,
    pub original_path: String,
    pub recovery_path: String,
    pub size: u64,
    pub deleted_at_ms: u64,
    pub expires_at_ms: u64,
    pub category_id: String,
    pub drive: String,
}

impl Default for RecoveryEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            batch_id: String::new(),
            original_path: String::new(),
            recovery_path: String::new(),
            size: 0,
            deleted_at_ms: 0,
            expires_at_ms: 0,
            category_id: String::new(),
            drive: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct RecoveryIndex {
    pub entries: Vec<RecoveryEntry>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 为给定文件路径确定其恢复区根目录（按盘符分布）。
/// 目标恢复区文件系统为 FAT32 时返回 (root, fs_name)，供 >4GB 兜底策略判断。
pub fn ensure_recovery_root(for_file: &Path) -> std::io::Result<PathBuf> {
    let drive_root = util::drive_root_of(for_file)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "非本地盘路径"))?;
    let recovery = drive_root.join(DIR_NAME);
    if create_recovery_dir(&recovery).is_ok() {
        return Ok(recovery);
    }
    // 回退：%LOCALAPPDATA%\DiskClear\Recovery（PRD 6.1）
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    if !local.is_empty() {
        let fallback = PathBuf::from(&local).join(FALLBACK);
        if create_recovery_dir(&fallback).is_ok() {
            return Ok(fallback);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "无法创建恢复区目录",
    ))
}

fn create_recovery_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(util::long_path(dir))?;
    util::set_hidden_system(dir);
    Ok(())
}

fn index_path(root: &Path) -> PathBuf {
    root.join(INDEX_FILE)
}

pub fn load_index(root: &Path) -> RecoveryIndex {
    match std::fs::read_to_string(util::long_path(&index_path(root))) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => RecoveryIndex::default(),
    }
}

pub fn save_index(root: &Path, index: &RecoveryIndex) -> std::io::Result<()> {
    std::fs::create_dir_all(util::long_path(root))?;
    let tmp = index_path(root).with_extension("json.tmp");
    std::fs::write(util::long_path(&tmp), serde_json::to_string_pretty(index)?)?;
    std::fs::rename(util::long_path(&tmp), util::long_path(&index_path(root)))?;
    Ok(())
}

/// 所有已存在恢复区根（各盘 + 回退目录）。
pub fn existing_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for d in util::list_drives() {
        let p = PathBuf::from(format!("{}:\\", d.letter)).join(DIR_NAME);
        if p.is_dir() {
            roots.push(p);
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local).join(FALLBACK);
        if p.is_dir() {
            roots.push(p);
        }
    }
    roots
}

/// 启动维护：过期清理 + 孤儿自愈（PRD 6.1）。返回清理的过期条目数。
pub fn startup_maintenance(state: &crate::AppState) -> usize {
    let _guard = state.recovery_lock.lock().unwrap();
    let settings = crate::settings::load();
    let mut expired_removed = 0;
    for root in existing_roots() {
        expired_removed += maintenance_for_root(&root, settings.recovery_retention_days as u64 * 86_400_000);
    }
    expired_removed
}

/// 单个恢复区根的维护逻辑（可测试）：过期清除 + 孤儿批次重建 + 空批次清理。
pub fn maintenance_for_root(root: &Path, retention_ms: u64) -> usize {
    let mut index = load_index(root);
    // 1. 过期清理
    let now = now_ms();
    let (keep, expired): (Vec<_>, Vec<_>) = index
        .entries
        .iter()
        .cloned()
        .partition(|e| e.expires_at_ms > now);
    let expired_removed = expired.len();
    for e in &expired {
        let _ = std::fs::remove_file(util::long_path(Path::new(&e.recovery_path)));
        let _ = std::fs::remove_dir_all(util::long_path(Path::new(&e.recovery_path)));
    }
    // 2. 自愈：磁盘上存在但索引未引用的孤儿批次 → 重建条目（原始路径未知）。
    //    known = 索引中各条目的所属批次目录（规范化后比较，PRD 6.1 自愈不得误伤正常批次）
    let known: std::collections::HashSet<String> = keep
        .iter()
        .filter_map(|e| {
            Path::new(&e.recovery_path)
                .parent()
                .map(|p| util::normalize_path(p))
        })
        .collect();
    let mut healed = keep;
    for batch_dir in orphan_batches(root, &known) {
        for (path, size) in files_in_batch(&batch_dir) {
            let batch_name = Path::new(&path)
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let drive = util::drive_root_of(Path::new(&path))
                .and_then(|d| d.to_string_lossy().chars().next().map(|c| c.to_ascii_uppercase().to_string()))
                .unwrap_or_default();
            healed.push(RecoveryEntry {
                id: format!("heal-{}", now_ms() ^ healed.len() as u64),
                batch_id: batch_name,
                original_path: "未知（索引缺失，已自愈登记）".to_string(),
                size,
                recovery_path: path,
                deleted_at_ms: now,
                expires_at_ms: now + retention_ms,
                category_id: "unknown".to_string(),
                drive,
                ..Default::default()
            });
        }
    }
    index.entries = healed;
    let _ = save_index(root, &index);
    // 3. 清掉空批次目录
    remove_empty_batches(root);
    expired_removed
}

fn orphan_batches(root: &Path, known: &std::collections::HashSet<String>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(util::long_path(root)) else {
        return out;
    };
    for entry in rd.flatten() {
        if entry.path().is_dir() {
            let sp = util::normalize_path(&entry.path());
            if !sp.is_empty() && !known.contains(&sp) {
                out.push(entry.path());
            }
        }
    }
    out
}

/// 遍历批次内全部文件，返回 (规范化路径, 大小)。
fn files_in_batch(batch: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let Ok(md) = std::fs::metadata(util::long_path(batch)) else {
        return out;
    };
    if md.is_file() {
        return vec![(util::normalize_path(batch), md.len())];
    }
    let mut stack = vec![batch.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(util::long_path(&dir)) else {
            continue;
        };
        for entry in rd.flatten() {
            let Ok(m) = entry.metadata() else { continue };
            if m.is_dir() {
                stack.push(entry.path());
            } else {
                out.push((util::normalize_path(&entry.path()), m.len()));
            }
        }
    }
    out
}

fn remove_empty_batches(root: &Path) {
    let Ok(rd) = std::fs::read_dir(util::long_path(root)) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() && std::fs::read_dir(util::long_path(&p)).map(|mut i| i.next().is_none()).unwrap_or(false) {
            let _ = std::fs::remove_dir(util::long_path(&p));
        }
    }
}

/// 汇总所有恢复区的总大小（状态栏展示）。
pub fn total_size(state: &crate::AppState) -> u64 {
    let _guard = state.recovery_lock.lock().unwrap();
    existing_roots()
        .iter()
        .map(|r| load_index(r).entries.iter().map(|e| e.size).sum::<u64>())
        .sum()
}

/// 全部恢复区条目（跨盘合并，按删除时间倒序）。
pub fn list_all(state: &crate::AppState) -> Vec<RecoveryEntry> {
    let _guard = state.recovery_lock.lock().unwrap();
    let mut all: Vec<RecoveryEntry> = existing_roots()
        .iter()
        .flat_map(|r| load_index(r).entries)
        .collect();
    all.sort_by(|a, b| b.deleted_at_ms.cmp(&a.deleted_at_ms));
    all
}

pub fn insert_entries(state: &crate::AppState, entries: Vec<RecoveryEntry>) {
    let _guard = state.recovery_lock.lock().unwrap();
    // 按恢复路径所属根分组写入各自索引
    let roots = existing_roots();
    let mut by_root: std::collections::HashMap<PathBuf, Vec<RecoveryEntry>> =
        std::collections::HashMap::new();
    for e in entries {
        let erp = PathBuf::from(&e.recovery_path);
        let root = roots
            .iter()
            .find(|r| erp.starts_with(r))
            .cloned()
            .unwrap_or_else(|| {
                util::drive_root_of(&erp)
                    .map(|d| d.join(DIR_NAME))
                    .unwrap_or(erp.clone())
            });
        by_root.entry(root).or_default().push(e);
    }
    for (root, entries) in by_root {
        let mut index = load_index(&root);
        index.entries.extend(entries);
        let _ = save_index(&root, &index);
    }
}

/// 恢复条目到原路径；返回成功数。父目录不存在时自动创建。
pub fn restore(state: &crate::AppState, entry_ids: &[String]) -> usize {
    let _guard = state.recovery_lock.lock().unwrap();
    restore_in_roots(existing_roots(), entry_ids)
}

pub fn restore_in_roots(roots: Vec<PathBuf>, entry_ids: &[String]) -> usize {
    let mut restored = 0;
    for root in roots {
        let mut index = load_index(&root);
        let mut changed = false;
        for e in index.entries.iter_mut() {
            if entry_ids.contains(&e.id) && Path::new(&e.recovery_path).exists() {
                let dest = PathBuf::from(&e.original_path);
                if dest.exists() {
                    // 原路径已被占用：不覆盖，改存为恢复副本
                    let stem = dest.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    let ext = dest.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
                    let parent = dest.parent().unwrap_or(Path::new("C:\\"));
                    let alt = parent.join(format!("{} (恢复){}", stem, ext));
                    let alt = if alt.exists() {
                        parent.join(format!("{} (恢复-{}){}", stem, now_ms(), ext))
                    } else {
                        alt
                    };
                    if util::move_path(Path::new(&e.recovery_path), &alt).is_ok() {
                        e.recovery_path = String::new();
                        restored += 1;
                        changed = true;
                    }
                } else if let Some(parent) = dest.parent() {
                    if std::fs::create_dir_all(util::long_path(parent)).is_ok()
                        && util::move_path(Path::new(&e.recovery_path), &dest).is_ok()
                    {
                        e.recovery_path = String::new();
                        restored += 1;
                        changed = true;
                    }
                }
            }
        }
        if changed {
            index.entries.retain(|e| !e.recovery_path.is_empty());
            let _ = save_index(&root, &index);
            remove_empty_batches(&root);
        }
    }
    restored
}

/// 永久删除恢复区条目（用户显式操作或过期）；返回成功数。
pub fn purge(state: &crate::AppState, entry_ids: &[String]) -> usize {
    let _guard = state.recovery_lock.lock().unwrap();
    purge_in_roots(existing_roots(), entry_ids)
}

pub fn purge_in_roots(roots: Vec<PathBuf>, entry_ids: &[String]) -> usize {
    let mut purged = 0;
    for root in roots {
        let mut index = load_index(&root);
        let ids: std::collections::HashSet<&String> = entry_ids.iter().collect();
        let removed: Vec<RecoveryEntry> = index
            .entries
            .iter()
            .filter(|e| ids.contains(&e.id))
            .cloned()
            .collect();
        if removed.is_empty() {
            continue;
        }
        index.entries.retain(|e| !ids.contains(&e.id));
        let _ = save_index(&root, &index);
        for e in removed {
            let p = Path::new(&e.recovery_path);
            let _ = std::fs::remove_file(util::long_path(p));
            let _ = std::fs::remove_dir_all(util::long_path(p));
            purged += 1;
        }
        remove_empty_batches(&root);
    }
    purged
}

/// 清空全部恢复区。
pub fn purge_all(state: &crate::AppState) -> usize {
    let ids: Vec<String> = list_all(state).into_iter().map(|e| e.id).collect();
    purge(state, &ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entry(id: &str, batch: &str, original: &Path, recovery: &Path) -> RecoveryEntry {
        RecoveryEntry {
            id: id.to_string(),
            batch_id: batch.to_string(),
            original_path: original.to_string_lossy().to_string(),
            recovery_path: recovery.to_string_lossy().to_string(),
            size: 100,
            deleted_at_ms: now_ms(),
            expires_at_ms: now_ms() + 86_400_000,
            category_id: "temp".to_string(),
            drive: "C".to_string(),
            ..Default::default()
        }
    }

    /// 完整闭环：登记 → 恢复回原位 → 索引收敛；再登记 → 永久删除 → 文件与索引消失。
    #[test]
    fn recovery_roundtrip_restore_and_purge() {
        let base = std::env::temp_dir().join(format!("dc_recovery_test_{}", std::process::id()));
        let root = base.join("Recovery");
        let orig_dir = base.join("orig");
        std::fs::create_dir_all(util::long_path(&orig_dir)).unwrap();
        std::fs::create_dir_all(util::long_path(&root)).unwrap();

        // ---- 恢复闭环 ----
        let src1 = orig_dir.join("a.txt");
        std::fs::write(&src1, "hello").unwrap();
        let batch1 = root.join("batch1");
        std::fs::create_dir_all(util::long_path(&batch1)).unwrap();
        let rec1 = batch1.join("0001_a.txt");
        std::fs::rename(util::long_path(&src1), util::long_path(&rec1)).unwrap();
        save_index(
            &root,
            &RecoveryIndex {
                entries: vec![test_entry("e1", "batch1", &src1, &rec1)],
            },
        )
        .unwrap();

        assert!(!src1.exists());
        let n = restore_in_roots(vec![root.clone()], &["e1".to_string()]);
        assert_eq!(n, 1);
        assert!(src1.exists(), "恢复后原路径应存在");
        assert!(!rec1.exists());
        // 空批次目录应被清理，索引应收敛为空
        assert!(!batch1.exists());
        assert!(load_index(&root).entries.is_empty());

        // ---- 永久删除闭环 ----
        let src2 = orig_dir.join("b.txt");
        std::fs::write(&src2, "world").unwrap();
        let batch2 = root.join("batch2");
        std::fs::create_dir_all(util::long_path(&batch2)).unwrap();
        let rec2 = batch2.join("0001_b.txt");
        std::fs::rename(util::long_path(&src2), util::long_path(&rec2)).unwrap();
        save_index(
            &root,
            &RecoveryIndex {
                entries: vec![test_entry("e2", "batch2", &src2, &rec2)],
            },
        )
        .unwrap();

        let n = purge_in_roots(vec![root.clone()], &["e2".to_string()]);
        assert_eq!(n, 1);
        assert!(!rec2.exists(), "永久删除后文件应消失");
        assert!(!batch2.exists());
        assert!(load_index(&root).entries.is_empty());

        let _ = std::fs::remove_dir_all(util::long_path(&base));
    }

    /// 自愈：索引丢失但磁盘有孤儿文件 → 索引重建登记（原始路径未知）。
    #[test]
    fn orphan_self_heal_rebuilds_entries() {
        let base = std::env::temp_dir().join(format!("dc_heal_test_{}", std::process::id()));
        let root = base.join("Recovery");
        let batch = root.join("batchX");
        std::fs::create_dir_all(util::long_path(&batch)).unwrap();
        std::fs::write(batch.join("0001_junk.tmp"), "x").unwrap();

        let state = crate::AppState::new();
        // existing_roots() 扫真实盘，不含临时目录 —— 直接测孤儿收集逻辑
        let orphans = orphan_batches(&root, &std::collections::HashSet::new());
        assert_eq!(orphans.len(), 1);
        let files = files_in_batch(&orphans[0]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1, 1);
        let _ = &state;
        let _ = std::fs::remove_dir_all(util::long_path(&base));
    }

    /// 回归：索引完整时，维护不得把正常批次误判为孤儿（历史上 known 存文件路径、
    /// 孤儿比较用目录路径且带 \\?\ 前缀，导致每次维护都重复登记）。
    #[test]
    fn maintenance_must_not_duplicate_known_batches() {
        let base = std::env::temp_dir().join(format!("dc_maint_test_{}", std::process::id()));
        let root = base.join("Recovery");
        let batch = root.join("batchY");
        std::fs::create_dir_all(util::long_path(&batch)).unwrap();
        let rec_file = batch.join("0001_x.bin");
        std::fs::write(&rec_file, "data").unwrap();

        let entry = test_entry("e-ok", "batchY", Path::new(r"C:\orig\x.bin"), &rec_file);
        save_index(&root, &RecoveryIndex { entries: vec![entry] }).unwrap();

        let removed = maintenance_for_root(&root, 86_400_000);
        assert_eq!(removed, 0, "未过期的条目不应被清除");
        let after = load_index(&root);
        assert_eq!(after.entries.len(), 1, "正常批次被误判为孤儿并重复登记");
        assert_eq!(after.entries[0].id, "e-ok");
        // 连续两次维护结果稳定（幂等）
        assert_eq!(maintenance_for_root(&root, 86_400_000), 0);
        assert_eq!(load_index(&root).entries.len(), 1);
        let _ = std::fs::remove_dir_all(util::long_path(&base));
    }

    /// 过期条目：文件与索引记录一并清除，批次目录随空清理。
    #[test]
    fn maintenance_removes_expired_entries() {
        let base = std::env::temp_dir().join(format!("dc_exp_test_{}", std::process::id()));
        let root = base.join("Recovery");
        let batch = root.join("batchZ");
        std::fs::create_dir_all(util::long_path(&batch)).unwrap();
        let rec_file = batch.join("0001_old.bin");
        std::fs::write(&rec_file, "old").unwrap();

        let mut entry = test_entry("e-old", "batchZ", Path::new(r"C:\orig\old.bin"), &rec_file);
        entry.expires_at_ms = now_ms() - 1000; // 已过期
        save_index(&root, &RecoveryIndex { entries: vec![entry] }).unwrap();

        let removed = maintenance_for_root(&root, 86_400_000);
        assert_eq!(removed, 1);
        assert!(!rec_file.exists(), "过期文件应被物理删除");
        assert!(!batch.exists(), "空批次目录应被清理");
        assert!(load_index(&root).entries.is_empty());
        let _ = std::fs::remove_dir_all(util::long_path(&base));
    }
}
