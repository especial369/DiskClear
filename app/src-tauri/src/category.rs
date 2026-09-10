// 清理分类定义（PRD 2.2 扫描项清单）
//
// 安全模型（PRD 6.2）：所有可清理路径必须落在注册的分类根之下；
// 分类根本身即白名单的"例外清单"（如 C:\Windows\Temp、Explorer\thumbcache_*.db）。
// 清理器在删除前会再次校验路径归属（见 clean.rs::under_any_root）。

use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Low,
    Medium,
}

#[derive(Clone, Copy, PartialEq)]
pub enum NameMatch {
    /// 目录内全部文件
    Any,
    /// 文件名前缀匹配且以 .db 结尾（缩略图/图标缓存）
    CacheDb(&'static [&'static str]),
}

pub struct CategoryDef {
    pub id: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub risk: Risk,
    pub needs_admin: bool,
    pub name_match: NameMatch,
}

pub const CATEGORIES: &[CategoryDef] = &[
    CategoryDef {
        id: "temp",
        name: "系统临时文件",
        desc: "%TEMP% 与 C:\\Windows\\Temp 中的临时文件",
        risk: Risk::Low,
        needs_admin: false,
        name_match: NameMatch::Any,
    },
    CategoryDef {
        id: "recycle",
        name: "回收站",
        desc: "所选盘符回收站中的已删除文件（移入恢复区，仍可恢复）",
        risk: Risk::Low,
        needs_admin: false,
        name_match: NameMatch::Any,
    },
    CategoryDef {
        id: "browser",
        name: "浏览器缓存",
        desc: "Chrome / Edge / Firefox 的网页缓存",
        risk: Risk::Low,
        needs_admin: false,
        name_match: NameMatch::Any,
    },
    CategoryDef {
        id: "update",
        name: "Windows 更新缓存",
        desc: "C:\\Windows\\SoftwareDistribution\\Download 更新下载缓存",
        risk: Risk::Medium,
        needs_admin: true,
        name_match: NameMatch::Any,
    },
    CategoryDef {
        id: "logs",
        name: "系统日志",
        desc: "C:\\Windows\\Logs 系统日志文件",
        risk: Risk::Low,
        needs_admin: false,
        name_match: NameMatch::Any,
    },
    CategoryDef {
        id: "thumbnail",
        name: "缩略图缓存",
        desc: "资源管理器缩略图与图标缓存（重建后自动生成）",
        risk: Risk::Low,
        needs_admin: false,
        name_match: NameMatch::CacheDb(&["thumbcache_", "iconcache_"]),
    },
];

fn env_path(var: &str, fallback: &str) -> PathBuf {
    std::env::var(var)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(fallback))
}

/// 浏览器缓存目录展开：Chrome/Edge 按配置枚举 Profile，Firefox 枚举 cache2。
fn browser_cache_roots() -> Vec<PathBuf> {
    let local = env_path("LOCALAPPDATA", r"C:\Users\Public\AppData\Local");
    let mut roots = Vec::new();
    for browser in ["Google\\Chrome", "Microsoft\\Edge"] {
        let user_data = local.join(browser).join("User Data");
        let Ok(rd) = std::fs::read_dir(&user_data) else {
            continue;
        };
        for profile in rd.flatten() {
            let name = profile.file_name().to_string_lossy().to_lowercase();
            // Profile 目录：Default、Profile 1、Guest Profile 等；排除系统子目录
            if !profile.path().is_dir()
                || matches!(
                    name.as_str(),
                    "crashpad" | "browsermetrics" | "safes browsing" | "extensions" | "system profile"
                )
            {
                continue;
            }
            let is_profile = name == "default" || name.starts_with("profile ") || name.ends_with(" profile");
            if !is_profile {
                continue;
            }
            for cache in ["Cache", "Cache\\Cache_Data", "Code Cache", "GPUCache"] {
                roots.push(profile.path().join(cache));
            }
        }
    }
    // Firefox
    let ff = local.join("Mozilla\\Firefox\\Profiles");
    if let Ok(rd) = std::fs::read_dir(&ff) {
        for profile in rd.flatten() {
            if profile.path().is_dir() {
                roots.push(profile.path().join("cache2"));
            }
        }
    }
    roots
}

/// 解析分类的扫描根目录。`drive` 为所选盘符字母（如 "C"）。
///
/// 盘符隔离：系统级分类（临时文件 / 浏览器缓存 / 更新缓存 / 系统日志 / 缩略图缓存）
/// 的目录物理上只存在于系统盘（`%TEMP%`、`C:\Windows\*`、`%LOCALAPPDATA%`），仅在所选盘
/// 为系统盘时扫描；选非系统盘时这些分类返回空，避免把 C 盘系统垃圾混入其他盘结果。
/// 回收站随所选盘符扫描。
pub fn roots_for(id: &str, drive: &str) -> Vec<PathBuf> {
    let local = env_path("LOCALAPPDATA", r"C:\Users\Public\AppData\Local");
    let sys_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    let is_system_drive = drive.eq_ignore_ascii_case(sys_drive.trim_end_matches(':'));
    match id {
        "temp" if is_system_drive => vec![
            env_path("TEMP", r"C:\Windows\Temp"),
            PathBuf::from(r"C:\Windows\Temp"),
        ],
        "recycle" => vec![PathBuf::from(format!("{}:\\$Recycle.Bin", drive))],
        "browser" if is_system_drive => browser_cache_roots(),
        "update" if is_system_drive => vec![PathBuf::from(r"C:\Windows\SoftwareDistribution\Download")],
        "logs" if is_system_drive => vec![PathBuf::from(r"C:\Windows\Logs")],
        "thumbnail" if is_system_drive => vec![local.join(r"Microsoft\Windows\Explorer")],
        _ => Vec::new(),
    }
}

/// 文件名是否匹配分类规则（缩略图类只清 thumbcache_/iconcache_ 的 .db）。
pub fn matches_name(cat: &CategoryDef, file_name: &str) -> bool {
    let lower = file_name.to_lowercase();
    match cat.name_match {
        NameMatch::Any => true,
        NameMatch::CacheDb(prefixes) => {
            lower.ends_with(".db") && prefixes.iter().any(|p| lower.starts_with(p))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_matcher_only_cache_db() {
        let cat = CategoryDef {
            id: "thumbnail",
            name: "缩略图缓存",
            desc: "",
            risk: Risk::Low,
            needs_admin: false,
            name_match: NameMatch::CacheDb(&["thumbcache_", "iconcache_"]),
        };
        assert!(matches_name(&cat, "thumbcache_1024.db"));
        assert!(matches_name(&cat, "iconcache_768.db"));
        assert!(!matches_name(&cat, "other.db"));
        assert!(!matches_name(&cat, "thumbstore.idx"));
    }

    #[test]
    fn recycle_root_uses_selected_drive() {
        let roots = roots_for("recycle", "D");
        assert_eq!(roots[0], PathBuf::from(r"D:\$Recycle.Bin"));
    }
}
