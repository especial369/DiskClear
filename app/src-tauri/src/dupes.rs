// 重复文件检测（PRD 2.4）
//
// 三级算法：大小分组（不同大小不可能重复）→ 首 4KB 预筛（FNV-1a，减少全量读盘）
// → 全量 MD5 复核（分块 1MB，带大小+修改时间缓存）。
// 推荐保留：修改时间最新优先，相同则路径最短（PRD v1.1 明确优先级）。
// 低于 1MB 的文件不参与（微小文件去重价值低、IO 成本高）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::cache::HashCache;
use crate::settings;
use crate::walker::{self, WalkFile};

const MIN_FILE_BYTES: u64 = 1024 * 1024; // 1MB
const MAX_HASHED_FILES: usize = 50_000;
const HEAD_BYTES: usize = 4096;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DupeFile {
    pub path: String,
    pub size: u64,
    pub modified_ms: u64,
    pub is_cloud: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DupeGroup {
    pub hash: String,
    pub size: u64,
    pub files: Vec<DupeFile>,
    pub keep_index: usize,
}

impl DupeGroup {
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len() as u64 - 1)
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DupeScanSession {
    pub session_id: u32,
    pub drive: String,
    pub groups: Vec<DupeGroup>,
    pub walked_files: u64,
    pub hashed_files: u64,
    pub truncated: bool,
    pub done: bool,
}

/// 推荐保留项下标：修改时间最新优先；相同则路径最短。
pub fn pick_keep(files: &[DupeFile]) -> usize {
    let mut best = 0usize;
    for i in 1..files.len() {
        let a = &files[best];
        let b = &files[i];
        let better = b.modified_ms > a.modified_ms
            || (b.modified_ms == a.modified_ms
                && b.path.len() < a.path.len());
        if better {
            best = i;
        }
    }
    best
}

struct PhaseProgress {
    app: AppHandle,
    session_id: u32,
    stop: Arc<AtomicBool>,
}

impl PhaseProgress {
    fn emit(&self, phase: &str, done: u64, total: u64, groups: usize) {
        let _ = self.app.emit(
            "dupe-progress",
            serde_json::json!({
                "sessionId": self.session_id,
                "phase": phase,
                "done": done,
                "total": total,
                "groups": groups,
            }),
        );
    }

    /// 长阶段哈希循环中周期性推送并检查取消；返回是否应终止。
    fn tick(&self, phase: &str, done: u64, total: u64, groups: usize, last: &mut std::time::Instant) -> bool {
        if last.elapsed() >= Duration::from_millis(250) {
            self.emit(phase, done, total, groups);
            *last = std::time::Instant::now();
        }
        self.stop.load(Ordering::SeqCst)
    }
}

pub fn start_dupe_scan(app: AppHandle, state: &crate::AppState, drive: String) -> u32 {
    let id = state.next_session.fetch_add(1, Ordering::SeqCst);
    let cancelled = Arc::new(AtomicBool::new(false));
    state.cancels.lock().unwrap().insert(id, cancelled.clone());

    let app2 = app.clone();
    let cancelled2 = cancelled.clone();
    let threads = settings::load().scan_threads;

    std::thread::spawn(move || {
        let session = run_scan(app2.clone(), id, drive, threads, cancelled2, &state_hash_cache(&app2));
        let st = app2.state::<crate::AppState>();
        st.dupe_sessions.lock().unwrap().insert(id, session);
    });
    id
}

fn state_hash_cache(app: &AppHandle) -> Arc<HashCache> {
    let st = app.state::<crate::AppState>();
    st.hash_cache.clone()
}

fn run_scan(
    app: AppHandle,
    id: u32,
    drive: String,
    threads: u32,
    cancelled: Arc<AtomicBool>,
    cache: &Arc<HashCache>,
) -> DupeScanSession {
    let progress = PhaseProgress {
        app: app.clone(),
        session_id: id,
        stop: cancelled.clone(),
    };

    // ---- 阶段 1：并行遍历，按大小分组 ----
    let (rx, handles, counters) = walker::walk_parallel(
        vec![std::path::PathBuf::from(format!("{}:\\", drive))],
        threads as usize,
        MIN_FILE_BYTES,
        cancelled.clone(),
    );
    let mut by_size: HashMap<u64, Vec<WalkFile>> = HashMap::new();
    let mut walked = 0u64;
    let mut done_workers = 0usize;
    let mut last_emit = std::time::Instant::now();
    while done_workers < handles.len() {
        match rx.recv_timeout(Duration::from_millis(120)) {
            Ok(walker::WalkMsg::File(f)) => {
                walked += 1;
                if f.size >= MIN_FILE_BYTES && !f.is_cloud {
                    by_size.entry(f.size).or_default().push(f);
                }
            }
            Ok(walker::WalkMsg::Done) => done_workers += 1,
            Err(_) => {
                if cancelled.load(Ordering::SeqCst) {
                    break;
                }
            }
        }
        if last_emit.elapsed() >= Duration::from_millis(250) {
            progress.emit(
                "walk",
                walked,
                counters.files_seen.load(Ordering::Relaxed),
                0,
            );
            last_emit = std::time::Instant::now();
        }
    }
    for h in handles {
        let _ = h.join();
    }
    if cancelled.load(Ordering::SeqCst) {
        return empty_session(id, drive, walked, 0, false);
    }

    // 仅保留 ≥2 个副本的大小组，按组内总占用降序（大组先处理，用户感知更好）
    let mut size_groups: Vec<(u64, Vec<WalkFile>)> = by_size
        .into_iter()
        .filter(|(_, v)| v.len() >= 2)
        .collect();
    size_groups.sort_by(|a, b| {
        let wa = a.0 * a.1.len() as u64;
        let wb = b.0 * b.1.len() as u64;
        wb.cmp(&wa)
    });
    let total_groups = size_groups.len() as u64;

    // ---- 阶段 2：首 4KB 预筛 ----
    let mut prescreened: Vec<(u64, Vec<WalkFile>)> = Vec::new();
    let mut prescreen_done = 0u64;
    last_emit = std::time::Instant::now();
    for (size, group) in size_groups {
        if cancelled.load(Ordering::SeqCst) {
            return empty_session(id, drive, walked, 0, false);
        }
        let mut sub: HashMap<u64, Vec<WalkFile>> = HashMap::new();
        for f in group {
            if let Some(head) = walker::read_head(&f.path, HEAD_BYTES) {
                sub.entry(walker::fnv64a(&head)).or_default().push(f);
            }
        }
        for (_, files) in sub {
            if files.len() >= 2 {
                prescreened.push((size, files));
            }
        }
        prescreen_done += 1;
        if progress.tick("prescreen", prescreen_done, total_groups, 0, &mut last_emit) {
            return empty_session(id, drive, walked, 0, false);
        }
    }

    // ---- 阶段 3：全量 MD5（带缓存）----
    let mut final_groups: Vec<DupeGroup> = Vec::new();
    let mut hashed = 0u64;
    let mut truncated = false;
    last_emit = std::time::Instant::now();
    'outer: for (size, files) in prescreened {
        let mut by_hash: HashMap<String, Vec<WalkFile>> = HashMap::new();
        for f in files {
            if hashed >= MAX_HASHED_FILES as u64 {
                truncated = true;
                break 'outer;
            }
            let hash = match cache.get(&f.path, f.size, f.modified_ms) {
                Some(h) => h,
                None => match walker::md5_of_file(&f.path) {
                    Some(h) => {
                        cache.put(&f.path, f.size, f.modified_ms, h.clone());
                        h
                    }
                    None => continue, // 读取失败（可能刚被删除/锁定）：跳过
                },
            };
            hashed += 1;
            by_hash.entry(hash).or_default().push(f);
            if progress.tick("hash", hashed, hashed.max(1), final_groups.len(), &mut last_emit) {
                break 'outer;
            }
        }
        for (hash, files) in by_hash {
            if files.len() >= 2 {
                let mut df: Vec<DupeFile> = files
                    .into_iter()
                    .map(|f| DupeFile {
                        path: f.path,
                        size: f.size,
                        modified_ms: f.modified_ms,
                        is_cloud: f.is_cloud,
                    })
                    .collect();
                df.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
                let keep_index = pick_keep(&df);
                final_groups.push(DupeGroup {
                    hash,
                    size,
                    files: df,
                    keep_index,
                });
            }
        }
    }

    // 可释放空间降序
    final_groups.sort_by(|a, b| b.wasted().cmp(&a.wasted()));
    cache.save_if_dirty();

    let _ = app.emit(
        "dupe-done",
        serde_json::json!({ "sessionId": id, "cancelled": cancelled.load(Ordering::SeqCst) }),
    );

    DupeScanSession {
        session_id: id,
        drive,
        walked_files: walked,
        hashed_files: hashed,
        groups: final_groups,
        truncated,
        done: true,
    }
}

fn empty_session(id: u32, drive: String, walked: u64, hashed: u64, truncated: bool) -> DupeScanSession {
    DupeScanSession {
        session_id: id,
        drive,
        groups: Vec::new(),
        walked_files: walked,
        hashed_files: hashed,
        truncated,
        done: true,
    }
}

pub fn cancel(state: &crate::AppState, session_id: u32) {
    if let Some(c) = state.cancels.lock().unwrap().get(&session_id) {
        c.store(true, Ordering::SeqCst);
    }
}

pub fn get_result(state: &crate::AppState, session_id: u32) -> Option<DupeScanSession> {
    state
        .dupe_sessions
        .lock()
        .unwrap()
        .get(&session_id)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, mtime: u64) -> DupeFile {
        DupeFile {
            path: path.to_string(),
            size: 10,
            modified_ms: mtime,
            is_cloud: false,
        }
    }

    #[test]
    fn keep_prefers_newest_mtime_then_shortest_path() {
        let files = vec![
            f(r"C:\a\very\long\path\file.txt", 100),
            f(r"C:\b\file.txt", 300), // 最新 → 保留
            f(r"C:\c\file.txt", 200),
        ];
        assert_eq!(pick_keep(&files), 1);

        let tie = vec![
            f(r"C:\aaaa\bbbb\file.txt", 100),
            f(r"C:\short.txt", 100), // 同时间路径最短 → 保留
        ];
        assert_eq!(pick_keep(&tie), 1);
    }

    #[test]
    fn wasted_is_size_times_extra_copies() {
        let g = DupeGroup {
            hash: "x".into(),
            size: 1000,
            files: vec![f("a", 1), f("b", 2), f("c", 3)],
            keep_index: 0,
        };
        assert_eq!(g.wasted(), 2000);
    }
}
