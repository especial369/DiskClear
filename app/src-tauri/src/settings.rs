// 设置持久化（PRD 2.7）：%APPDATA%\com.diskclear.app\settings.json

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 恢复区保留天数（1-30，默认 7）
    pub recovery_retention_days: u32,
    /// 白名单目录：扫描时跳过（用户自定义）
    pub whitelist_dirs: Vec<String>,
    /// FAT32 超 4GB 文件兜底策略：skip=跳过 / confirm_delete=增强确认后直接删除
    pub fat32_policy: String,
    /// 大文件阈值 MB（默认 100）
    pub large_file_threshold_mb: u64,
    /// 扫描线程数（1-8，默认 4）
    pub scan_threads: u32,
    /// 主题：system / light / dark
    pub theme: String,
    /// 大文件忽略列表（用户选择不再显示的文件路径）
    pub ignored_files: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            recovery_retention_days: 7,
            whitelist_dirs: Vec::new(),
            fat32_policy: "skip".to_string(),
            large_file_threshold_mb: 100,
            scan_threads: 4,
            theme: "system".to_string(),
            ignored_files: Vec::new(),
        }
    }
}

fn config_path() -> Option<std::path::PathBuf> {
    dirs_config().map(|d| d.join("settings.json"))
}

fn dirs_config() -> Option<std::path::PathBuf> {
    // 使用 %APPDATA%（Roaming）；避免额外目录依赖
    std::env::var("APPDATA")
        .ok()
        .map(|base| std::path::PathBuf::from(base).join("com.diskclear.app"))
}

pub fn load() -> Settings {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> Result<(), String> {
    let path = config_path().ok_or("无法定位配置目录")?;
    std::fs::create_dir_all(&path.parent().unwrap()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_roundtrip() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.recovery_retention_days, 7);
        assert_eq!(back.fat32_policy, "skip");
        assert_eq!(back.theme, "system");
    }
}
