// DiskClear 盘清 — Tauri 应用入口
mod bigfiles;
mod cache;
mod category;
mod clean;
mod commands;
mod dupes;
mod recovery;
mod scan;
mod settings;
mod util;
mod walker;

use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use tauri::Manager;

use cache::HashCache;
use scan::ScanSession;

/// 全局应用状态：扫描会话 + 恢复区写锁 + M2 大文件/重复文件会话 + 哈希缓存。
/// 内部字段全部 Arc 化，便于命令层 clone 后跨线程使用（spawn_blocking）。
#[derive(Clone)]
pub struct AppState {
    pub sessions: Arc<Mutex<HashMap<u32, ScanSession>>>,
    pub big_sessions: Arc<Mutex<HashMap<u32, bigfiles::LargeScanSession>>>,
    pub dupe_sessions: Arc<Mutex<HashMap<u32, DupeScanSessionRef>>>,
    pub next_session: Arc<AtomicU32>,
    pub cancels: Arc<Mutex<HashMap<u32, Arc<std::sync::atomic::AtomicBool>>>>,
    pub recovery_lock: Arc<Mutex<()>>,
    pub hash_cache: Arc<HashCache>,
}

/// LargeScanSession 直接 Clone 存储；DupeScanSession 亦 Clone（字段均为 Owned 数据）
pub type DupeScanSessionRef = dupes::DupeScanSession;

impl AppState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            big_sessions: Arc::new(Mutex::new(HashMap::new())),
            dupe_sessions: Arc::new(Mutex::new(HashMap::new())),
            next_session: Arc::new(AtomicU32::new(1)),
            cancels: Arc::new(Mutex::new(HashMap::new())),
            recovery_lock: Arc::new(Mutex::new(())),
            hash_cache: Arc::new(HashCache::default()),
        }
    }

    pub fn clone_shared(&self) -> Self {
        self.clone()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .setup(|app| {
            // 启动维护：恢复区过期清理 + 孤儿自愈（PRD 6.1），后台执行不阻塞启动
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let state = handle.state::<AppState>();
                recovery::startup_maintenance(&state);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_drives,
            commands::get_env_info,
            commands::start_scan,
            commands::cancel_scan,
            commands::get_scan_result,
            commands::clean,
            commands::list_recovery,
            commands::restore_entries,
            commands::delete_recovery,
            commands::clear_recovery,
            commands::get_settings,
            commands::save_settings,
            commands::get_stats,
            commands::start_large_scan,
            commands::cancel_large_scan,
            commands::get_large_scan_result,
            commands::delete_large_file,
            commands::add_ignored_file,
            commands::clear_ignored_files,
            commands::start_dupe_scan,
            commands::cancel_dupe_scan,
            commands::get_dupe_scan_result,
            commands::delete_dupe_files,
            commands::open_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
