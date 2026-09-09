// Tauri IPC 命令层（前端 invoke 的全部入口）

use std::sync::atomic::Ordering;
use tauri::{AppHandle, State};
use tauri::async_runtime::spawn_blocking;

use crate::bigfiles;
use crate::clean::LargeDeleteOutcome;
use crate::dupes::{self, DupeScanSession};
use crate::recovery::RecoveryEntry;
use crate::scan::{CategoryResult, ScanSession};
use crate::settings::Settings;
use crate::util::DriveInfo;
use crate::{clean, recovery, scan, settings, util};

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EnvInfo {
    pub is_admin: bool,
    pub browsers_running: Vec<String>,
    pub wuauserv_running: bool,
    pub version: String,
}

#[tauri::command]
pub fn list_drives() -> Vec<DriveInfo> {
    util::list_drives()
}

#[tauri::command]
pub fn get_env_info() -> EnvInfo {
    EnvInfo {
        is_admin: util::is_admin(),
        browsers_running: util::running_browsers(),
        wuauserv_running: util::wuauserv_running(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[tauri::command]
pub fn start_scan(app: AppHandle, state: State<'_, crate::AppState>, drive: String) -> u32 {
    scan::start_scan(app, &state, drive)
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, crate::AppState>, session_id: u32) {
    scan::cancel_scan(&state, session_id);
}

#[tauri::command]
pub fn get_scan_result(state: State<'_, crate::AppState>, session_id: u32) -> Result<ScanResultDto, String> {
    let sessions = state.sessions.lock().unwrap();
    let s: &ScanSession = sessions.get(&session_id).ok_or("会话不存在")?;
    Ok(ScanResultDto {
        session_id: s.id,
        drive: s.drive.clone(),
        done: s.done,
        cancelled: s.cancelled.load(Ordering::SeqCst),
        results: s.results.clone(),
    })
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanResultDto {
    pub session_id: u32,
    pub drive: String,
    pub done: bool,
    pub cancelled: bool,
    pub results: Vec<CategoryResult>,
}

#[tauri::command]
pub async fn clean(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    session_id: u32,
    category_ids: Vec<String>,
) -> Result<clean::CleanResult, String> {
    // 跨线程执行耗时清理（async runtime 内使用 spawn_blocking）
    let app2 = app.clone();
    let moved_state = state.inner().clone_shared();
    spawn_blocking(move || clean::run_clean(&app2, &moved_state, session_id, category_ids))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn list_recovery(state: State<'_, crate::AppState>) -> Vec<RecoveryEntry> {
    recovery::list_all(&state)
}

#[tauri::command]
pub fn restore_entries(state: State<'_, crate::AppState>, entry_ids: Vec<String>) -> usize {
    recovery::restore(&state, &entry_ids)
}

#[tauri::command]
pub fn delete_recovery(state: State<'_, crate::AppState>, entry_ids: Vec<String>) -> usize {
    recovery::purge(&state, &entry_ids)
}

#[tauri::command]
pub fn clear_recovery(state: State<'_, crate::AppState>) -> usize {
    recovery::purge_all(&state)
}

#[tauri::command]
pub fn get_settings() -> Settings {
    settings::load()
}

#[tauri::command]
pub fn save_settings(settings: Settings) -> Result<Settings, String> {
    settings::save(&settings)?;
    Ok(settings::load())
}

#[tauri::command]
pub fn get_stats(state: State<'_, crate::AppState>) -> StatsDto {
    StatsDto {
        freed_total_bytes: clean::freed_total(),
        recovery_bytes: recovery::total_size(&state),
    }
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StatsDto {
    pub freed_total_bytes: u64,
    pub recovery_bytes: u64,
}

// ---------- M2：大文件查找 ----------

#[tauri::command]
pub fn start_large_scan(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    drive: String,
    threshold_mb: u64,
) -> u32 {
    bigfiles::start_large_scan(app, &state, drive, threshold_mb)
}

#[tauri::command]
pub fn cancel_large_scan(state: State<'_, crate::AppState>, session_id: u32) {
    bigfiles::cancel(&state, session_id);
}

#[tauri::command]
pub fn get_large_scan_result(
    state: State<'_, crate::AppState>,
    session_id: u32,
) -> Result<bigfiles::LargeScanSession, String> {
    bigfiles::get_result(&state, session_id).ok_or_else(|| "扫描会话不存在".to_string())
}

#[tauri::command]
pub async fn delete_large_file(
    state: State<'_, crate::AppState>,
    path: String,
    allow_permanent: bool,
) -> Result<LargeDeleteOutcome, String> {
    let moved_state = state.inner().clone_shared();
    spawn_blocking(move || clean::delete_large_file(&moved_state, &path, allow_permanent))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn add_ignored_file(path: String) -> Result<Settings, String> {
    let mut s = settings::load();
    if !s.ignored_files.iter().any(|p| p == &path) {
        s.ignored_files.push(path);
    }
    settings::save(&s)?;
    Ok(s)
}

#[tauri::command]
pub fn clear_ignored_files() -> Result<Settings, String> {
    let mut s = settings::load();
    s.ignored_files.clear();
    settings::save(&s)?;
    Ok(s)
}

// ---------- M2：重复文件检测 ----------

#[tauri::command]
pub fn start_dupe_scan(app: AppHandle, state: State<'_, crate::AppState>, drive: String) -> u32 {
    dupes::start_dupe_scan(app, &state, drive)
}

#[tauri::command]
pub fn cancel_dupe_scan(state: State<'_, crate::AppState>, session_id: u32) {
    dupes::cancel(&state, session_id);
}

#[tauri::command]
pub fn get_dupe_scan_result(
    state: State<'_, crate::AppState>,
    session_id: u32,
) -> Result<DupeScanSession, String> {
    dupes::get_result(&state, session_id).ok_or_else(|| "扫描会话不存在".to_string())
}

#[tauri::command]
pub async fn delete_dupe_files(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    paths: Vec<String>,
) -> Result<clean::CleanResult, String> {
    let app2 = app.clone();
    let moved_state = state.inner().clone_shared();
    spawn_blocking(move || clean::move_paths_to_recovery(&app2, &moved_state, paths, "dupes"))
        .await
        .map_err(|e| e.to_string())?
}

// ---------- M2：打开所在文件夹 ----------

#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    tauri_plugin_opener::reveal_item_in_dir(std::path::Path::new(&path))
        .map_err(|e| e.to_string())
}
