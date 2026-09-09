// 大文件查找（PRD 2.3 / 5.3）
//
// - 全盘并行扫描超过阈值的文件，按大小降序，明细上限 2 万条（超出置 truncated）
// - 明细字段：路径、大小、修改时间、云占位标记
// - 忽略列表（settings.ignored_files）中的路径直接跳过

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::settings;
use crate::util;
use crate::walker::{self, WalkFile};

const MAX_FILES: usize = 20_000;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub path: String,
    pub size: u64,
    pub modified_ms: u64,
    pub is_cloud: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LargeScanSession {
    pub session_id: u32,
    pub drive: String,
    pub files: Vec<LargeFile>,
    pub total_bytes: u64,
    pub truncated: bool,
    pub skipped_dirs: u64,
    pub done: bool,
}

pub fn start_large_scan(
    app: AppHandle,
    state: &crate::AppState,
    drive: String,
    threshold_mb: u64,
) -> u32 {
    let id = state.next_session.fetch_add(1, Ordering::SeqCst);
    let cancelled = Arc::new(AtomicBool::new(false));
    state.cancels.lock().unwrap().insert(id, cancelled.clone());

    let app2 = app.clone();
    let cancelled2 = cancelled.clone();
    let settings = settings::load();
    let threads = settings.scan_threads;
    let ignored: HashSet<String> = settings
        .ignored_files
        .iter()
        .map(|p| util::normalize_path(std::path::Path::new(p)).to_lowercase())
        .collect();

    std::thread::spawn(move || {
        let session = run_scan(app2.clone(), id, drive, threshold_mb, threads, ignored, cancelled2);
        let st = app2.state::<crate::AppState>();
        st.big_sessions.lock().unwrap().insert(id, session);
    });
    id
}

fn run_scan(
    app: AppHandle,
    id: u32,
    drive: String,
    threshold_mb: u64,
    threads: u32,
    ignored: HashSet<String>,
    cancelled: Arc<AtomicBool>,
) -> LargeScanSession {
    let roots = vec![std::path::PathBuf::from(format!("{}:\\", drive))];
    let threshold = threshold_mb.saturating_mul(1024 * 1024);

    let (rx, handles, counters) =
        walker::walk_parallel(roots, threads as usize, threshold, cancelled.clone());

    // 进度采样：每 300ms 推送已遍历文件数与字节数
    let progress_stop = Arc::new(AtomicBool::new(false));
    let progress_app = app.clone();
    let progress_stop2 = progress_stop.clone();
    let progress_cancelled = cancelled.clone();
    let progress_thread = std::thread::spawn(move || {
        while !progress_stop2.load(Ordering::SeqCst) && !progress_cancelled.load(Ordering::SeqCst) {
            let _ = progress_app.emit(
                "large-progress",
                serde_json::json!({
                    "sessionId": id,
                    "files": counters.files_seen.load(Ordering::Relaxed),
                    "bytes": counters.bytes_seen.load(Ordering::Relaxed),
                }),
            );
            std::thread::sleep(Duration::from_millis(300));
        }
    });

    let (mut files, truncated) = walker::collect(rx, handles, &cancelled, MAX_FILES);
    let skipped_dirs = counters.skipped_dirs.load(Ordering::Relaxed);
    progress_stop.store(true, Ordering::SeqCst);
    let _ = progress_thread.join();

    // 忽略列表过滤 + 降序排序 + 截断
    files.retain(|f| !ignored.contains(&f.path.to_lowercase()));
    files.sort_by(|a, b| b.size.cmp(&a.size));
    let mut truncated = truncated;
    if files.len() > MAX_FILES {
        files.truncate(MAX_FILES);
        truncated = true;
    }
    let total_bytes = files.iter().map(|f| f.size).sum();

    let _ = app.emit(
        "large-done",
        serde_json::json!({ "sessionId": id, "cancelled": cancelled.load(Ordering::SeqCst) }),
    );

    LargeScanSession {
        session_id: id,
        drive,
        total_bytes,
        truncated,
        skipped_dirs,
        done: true,
        files: files.into_iter().map(to_large_file).collect(),
    }
}

fn to_large_file(w: WalkFile) -> LargeFile {
    LargeFile {
        path: w.path,
        size: w.size,
        modified_ms: w.modified_ms,
        is_cloud: w.is_cloud,
    }
}

pub fn cancel(state: &crate::AppState, session_id: u32) {
    if let Some(c) = state.cancels.lock().unwrap().get(&session_id) {
        c.store(true, Ordering::SeqCst);
    }
}

pub fn get_result(state: &crate::AppState, session_id: u32) -> Option<LargeScanSession> {
    state
        .big_sessions
        .lock()
        .unwrap()
        .get(&session_id)
        .cloned()
}
