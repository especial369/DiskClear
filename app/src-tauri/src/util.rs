// 基础工具：长路径、盘符、权限/进程/服务检测、文件属性（对应 PRD 6.3 / 6.5）
#![allow(clippy::result_unit_err)]

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows_sys::Win32::Storage::FileSystem as fs;

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// FILE_ATTRIBUTE_* 原始值（windows-sys 未按名导出部分常量到 std 元数据场景）
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

// GetDriveTypeW 返回值（windows-sys 未导出，值为 Win32 API 固定定义）
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

/// 为绝对路径加 `\\?\` 前缀以支持超过 260 字符的长路径（PRD 6.5）。
/// UNC 路径转为 `\\?\UNC\...`。相对路径原样返回（本应用所有 fs 操作均为绝对路径）。
pub fn long_path<P: AsRef<Path>>(p: P) -> PathBuf {
    let p = p.as_ref();
    let s = p.to_string_lossy();
    if s.starts_with(r"\\?\") {
        p.to_path_buf()
    } else if s.starts_with(r"\\") {
        PathBuf::from(format!(r"\\?\UNC\{}", &s[2..]))
    } else if p.is_absolute() {
        PathBuf::from(format!(r"\\?\{}", s))
    } else {
        p.to_path_buf()
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// 规范化路径字符串：去掉 `\\?\` / `\\?\UNC\` 前缀（仅用于存储与比较，fs 操作仍走 long_path）。
pub fn normalize_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\");
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s
    }
}

fn drive_type_name(t: u32) -> &'static str {
    match t {
        DRIVE_REMOVABLE => "removable",
        DRIVE_FIXED => "fixed",
        DRIVE_REMOTE => "remote",
        DRIVE_CDROM => "cdrom",
        DRIVE_RAMDISK => "ramdisk",
        _ => "unknown",
    }
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    pub letter: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
    pub fs_name: String,
    pub drive_type: String,
}

/// 枚举本地逻辑盘符及容量/文件系统（GetLogicalDrives + GetDiskFreeSpaceExW）。
pub fn list_drives() -> Vec<DriveInfo> {
    let mut out = Vec::new();
    let mask = unsafe { fs::GetLogicalDrives() };
    if mask == 0 {
        return out;
    }
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{}:\\", letter);
        let wroot = wide(&root);
        let mut total: u64 = 0;
        let mut free: u64 = 0;
        let ok = unsafe {
            fs::GetDiskFreeSpaceExW(wroot.as_ptr(), std::ptr::null_mut(), &mut total, &mut free)
        };
        if ok == 0 || total == 0 {
            continue; // 光驱空盘、未就绪设备等
        }
        // 跳过网络盘与光驱（清理工具不适用）
        let dtype = unsafe { fs::GetDriveTypeW(wroot.as_ptr()) };
        if dtype == DRIVE_REMOTE || dtype == DRIVE_CDROM {
            continue;
        }
        let mut fs_buf = [0u16; 32];
        let fs_name = unsafe {
            let ok = fs::GetVolumeInformationW(
                wroot.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs_buf.as_mut_ptr(),
                fs_buf.len() as u32,
            );
            if ok != 0 {
                let end = fs_buf.iter().position(|&c| c == 0).unwrap_or(0);
                String::from_utf16_lossy(&fs_buf[..end])
            } else {
                String::new()
            }
        };
        out.push(DriveInfo {
            letter: letter.to_string(),
            total_bytes: total,
            free_bytes: free,
            used_bytes: total.saturating_sub(free),
            fs_name,
            drive_type: drive_type_name(dtype).to_string(),
        });
    }
    out
}

/// 盘符根目录（"C:\\"），输入非法时返回 None。
pub fn drive_root_of(path: &Path) -> Option<PathBuf> {
    let s = path.to_string_lossy();
    let bytes = s.as_bytes();
    if bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') {
        let letter = bytes[0].to_ascii_uppercase();
        if letter.is_ascii_alphabetic() {
            return Some(PathBuf::from(format!("{}:\\", letter as char)));
        }
    }
    None
}

/// 设置目录为隐藏 + 系统属性（恢复区目录，PRD 6.1）。
pub fn set_hidden_system(path: &Path) {
    let w = wide(&path.to_string_lossy());
    unsafe {
        fs::SetFileAttributesW(
            w.as_ptr(),
            fs::FILE_ATTRIBUTE_HIDDEN | fs::FILE_ATTRIBUTE_SYSTEM,
        )
    };
}

pub fn is_reparse_dir(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// OneDrive 云占位文件识别（PRD 6.5）：内容在云端，本地仅占位。
pub fn is_cloud_placeholder(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    let a = metadata.file_attributes();
    a & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS != 0 || a & FILE_ATTRIBUTE_OFFLINE != 0
}

fn run_quiet(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout).to_string()
                + &String::from_utf8_lossy(&o.stderr)
        })
}

/// 管理员权限检测（PRD 6.3）：`net session` 仅管理员可成功。
pub fn is_admin() -> bool {
    Command::new("net")
        .args(["session"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 当前正在运行的进程名（小写，含 .exe），通过 tasklist 枚举。
fn running_process_set() -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    if let Some(out) = run_quiet("tasklist", &["/FO", "CSV", "/NH"]) {
        for line in out.lines() {
            if let Some(name) = line.split(',').next() {
                let name = name.trim_matches('"').to_lowercase();
                if name.ends_with(".exe") {
                    set.insert(name);
                }
            }
        }
    }
    set
}

/// 检测指定浏览器是否正在运行（PRD 6.4）。
pub fn running_browsers() -> Vec<String> {
    let procs = running_process_set();
    let mut out = Vec::new();
    for (exe, label) in [
        ("chrome.exe", "Chrome"),
        ("msedge.exe", "Edge"),
        ("firefox.exe", "Firefox"),
    ] {
        if procs.contains(exe) {
            out.push(label.to_string());
        }
    }
    out
}

/// wuauserv（Windows 更新服务）是否运行中（PRD 6.4）。
pub fn wuauserv_running() -> bool {
    run_quiet("sc.exe", &["query", "wuauserv"])
        .map(|out| out.to_uppercase().contains("RUNNING"))
        .unwrap_or(false)
}

/// 同盘内移动：优先 rename（零拷贝），跨设备时复制+删除兜底。
pub fn move_path(src: &Path, dst: &Path) -> io::Result<u64> {
    match std::fs::rename(long_path(src), long_path(dst)) {
        Ok(()) => Ok(0),
        Err(_) => {
            // 跨设备或 rename 失败：复制 + 删除
            if src.is_dir() {
                copy_dir_recursive(src, dst)?;
                std::fs::remove_dir_all(long_path(src))?;
                Ok(0)
            } else {
                std::fs::copy(long_path(src), long_path(dst))?;
                std::fs::remove_file(long_path(src))?;
                std::fs::metadata(long_path(dst)).map(|m| m.len()).or(Ok(0))
            }
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    std::fs::create_dir_all(long_path(dst))?;
    for entry in std::fs::read_dir(long_path(src))? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(long_path(&entry.path()), long_path(target))?;
        }
    }
    Ok(())
}

pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(long_path(&dir)) else {
            continue;
        };
        for entry in rd.flatten() {
            let Ok(md) = entry.metadata() else { continue };
            if md.is_dir() {
                stack.push(entry.path());
            } else {
                total += md.len();
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_path_prefixes_absolute() {
        assert_eq!(
            long_path(r"C:\a\b.txt").to_string_lossy(),
            r"\\?\C:\a\b.txt"
        );
        assert_eq!(
            long_path(r"\\server\share\a").to_string_lossy(),
            r"\\?\UNC\server\share\a"
        );
        assert_eq!(
            long_path(r"\\?\C:\already.txt").to_string_lossy(),
            r"\\?\C:\already.txt"
        );
    }

    #[test]
    fn drive_root_extracts_letter() {
        assert_eq!(
            drive_root_of(Path::new(r"c:\Users\x\y")),
            Some(PathBuf::from(r"C:\"))
        );
        assert_eq!(drive_root_of(Path::new("relative\\path")), None);
    }
}
