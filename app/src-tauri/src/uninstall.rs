// 应用卸载（PRD 2.6 v1 简单版）
//
// - 列表：读注册表 Uninstall 键（HKLM 64 位视图 / HKLM WOW6432Node / HKCU）
//   + UWP 应用（Appx 模型，经 PowerShell Get-AppxPackage，沿用 tasklist/sc 子进程模式）
// - 卸载：解析 UninstallString（引号/未引号/MSI 归一化 /I→/X）并调起程序自带卸载器，
//   后台线程等待其退出后 emit "uninstall-exited"；UWP 经 Remove-AppxPackage
// - 残留：仅目录级，扫描 ProgramFiles / ProgramFiles(x86) / ProgramData / LOCALAPPDATA
//   四根（PRD 2.6），按程序名/发布者规范化启发式匹配；用户勾选确认后经恢复区清理
//   （clean.rs::move_residue_dirs_to_recovery，双重防线校验）
// - 注册表残留扫描与清理明确不在 v1 范围（PRD 2.6 排除项，v2 规划）

use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use windows_sys::Win32::System::Registry as reg;

use crate::clean::under_any_root;
use crate::util;

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    /// 唯一标识：注册表来源为 "<键路径>\<子键名>"，UWP 为 "uwp:<PackageFullName>"
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    /// YYYY-MM-DD（无法解析时保留原始串；UWP 为空）
    pub install_date: String,
    /// 字节；未知为 0
    pub estimated_size: u64,
    pub uninstall_string: String,
    pub install_location: String,
    pub is_uwp: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResidueDir {
    pub path: String,
    pub size: u64,
    /// 匹配依据：名称匹配 / 发布者匹配 / 注册表安装位置
    pub reason: String,
}

/// 前端回传的待清理残留（size 为扫描时测得的目录大小）
#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResidueInput {
    pub path: String,
    pub size: u64,
}

// ---------------------------------------------------------------------------
// 注册表读取（windows-sys 裸 API，无 winreg 依赖）
// ---------------------------------------------------------------------------

fn reg_open(root: reg::HKEY, path: &str) -> Option<reg::HKEY> {
    let mut out: reg::HKEY = unsafe { std::mem::zeroed() };
    let wpath = util::wide(path);
    let st = unsafe {
        reg::RegOpenKeyExW(root, wpath.as_ptr(), 0, reg::KEY_READ, &mut out)
    };
    if st == 0 {
        Some(out)
    } else {
        None
    }
}

fn reg_close(hk: reg::HKEY) {
    unsafe { reg::RegCloseKey(hk) };
}

fn reg_subkeys(hk: reg::HKEY) -> Vec<String> {
    let mut out = Vec::new();
    let mut idx = 0u32;
    loop {
        let mut name = [0u16; 256];
        let mut len = 256u32;
        let st = unsafe {
            reg::RegEnumKeyExW(
                hk,
                idx,
                name.as_mut_ptr(),
                &mut len,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if st != 0 || len == 0 {
            break; // ERROR_NO_MORE_ITEMS 或错误
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
        idx += 1;
        if idx > 8192 {
            break; // 防御性上限
        }
    }
    out
}

fn reg_value_raw(hk: reg::HKEY, name: &str) -> Option<(u32, Vec<u8>)> {
    let wname = util::wide(name);
    let mut ty: u32 = 0;
    let mut size: u32 = 0;
    let st = unsafe {
        reg::RegQueryValueExW(
            hk,
            wname.as_ptr(),
            std::ptr::null(),
            &mut ty,
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if st != 0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    let st = unsafe {
        reg::RegQueryValueExW(
            hk,
            wname.as_ptr(),
            std::ptr::null(),
            &mut ty,
            buf.as_mut_ptr(),
            &mut size,
        )
    };
    if st != 0 {
        return None;
    }
    buf.truncate(size as usize);
    Some((ty, buf))
}

fn bytes_to_wstring(buf: &[u8]) -> Vec<u16> {
    buf.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn wstring_to_string(mut w: Vec<u16>) -> String {
    while w.last() == Some(&0) {
        w.pop();
    }
    String::from_utf16_lossy(&w)
}

fn reg_string(hk: reg::HKEY, name: &str) -> Option<String> {
    let (ty, buf) = reg_value_raw(hk, name)?;
    match ty {
        reg::REG_SZ | reg::REG_EXPAND_SZ => Some(wstring_to_string(bytes_to_wstring(&buf))),
        _ => None,
    }
}

fn reg_dword(hk: reg::HKEY, name: &str) -> Option<u32> {
    let (ty, buf) = reg_value_raw(hk, name)?;
    if ty == reg::REG_DWORD && buf.len() >= 4 {
        Some(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]))
    } else {
        None
    }
}

/// 展开 %VAR%（std::env::var）；未定义的变量保持原样。
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => {
                let var = &after[..j];
                if var.is_empty() {
                    out.push_str("%%");
                } else if let Ok(v) = std::env::var(var) {
                    out.push_str(&v);
                } else {
                    out.push_str(&format!("%{}%", var));
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// "20250103" → "2025-01-03"；无法解析时原样返回。
fn format_install_date(raw: &str) -> String {
    let t = raw.trim();
    if t.len() == 8 && t.bytes().all(|b| b.is_ascii_digit()) {
        format!("{}-{}-{}", &t[0..4], &t[4..6], &t[6..8])
    } else {
        t.to_string()
    }
}

// ---------------------------------------------------------------------------
// 已安装程序列表
// ---------------------------------------------------------------------------

/// HKLM 64 位视图 + HKLM WOW6432Node（32 位程序）+ HKCU。
/// 用函数返回（而非 static）以规避 HKEY 句柄类型的 Sync 限制。
fn uninstall_sources() -> Vec<(reg::HKEY, &'static str)> {
    vec![
        (
            reg::HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            reg::HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            reg::HKEY_CURRENT_USER,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
    ]
}

/// 本应用自身不进入卸载列表（不可卸载自己）。
fn is_self(name: &str) -> bool {
    name.to_lowercase().contains("diskclear") || name.contains("盘清")
}

pub fn list_installed_apps() -> Vec<InstalledApp> {
    let mut out: Vec<InstalledApp> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (root, path) in uninstall_sources() {
        let Some(hk) = reg_open(root, path) else { continue };
        for sub in reg_subkeys(hk) {
            let Some(hsub) = reg_open(hk, &sub) else { continue };
            // SystemComponent=1：系统组件/热修复，控制面板不显示，跳过
            if reg_dword(hsub, "SystemComponent") == Some(1) {
                reg_close(hsub);
                continue;
            }
            let Some(name) = reg_string(hsub, "DisplayName")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
            else {
                reg_close(hsub);
                continue;
            };
            if is_self(&name) {
                reg_close(hsub);
                continue;
            }
            // KB 热修复项跳过
            if name.starts_with("KB") && name.to_lowercase().contains("update") {
                reg_close(hsub);
                continue;
            }
            // 无卸载命令的程序无法调起卸载器，跳过
            let un = reg_string(hsub, "UninstallString").unwrap_or_default();
            if un.trim().is_empty() {
                reg_close(hsub);
                continue;
            }
            let version = reg_string(hsub, "DisplayVersion").unwrap_or_default();
            let publisher = reg_string(hsub, "Publisher").unwrap_or_default();
            let install_date =
                format_install_date(&reg_string(hsub, "InstallDate").unwrap_or_default());
            let size_kb = reg_dword(hsub, "EstimatedSize").unwrap_or(0);
            let install_location = reg_string(hsub, "InstallLocation")
                .map(|s| expand_env(&s).trim().trim_end_matches('\\').to_string())
                .unwrap_or_default();
            reg_close(hsub);

            // 去重：同一程序可能在 HKCU/HKLM 双处登记（按名称+版本+卸载命令）
            let key = format!("{}|{}|{}", name.to_lowercase(), version, un);
            if !seen.insert(key) {
                continue;
            }
            out.push(InstalledApp {
                id: format!("{}\\{}", path, sub),
                name,
                version,
                publisher,
                install_date,
                estimated_size: (size_kb as u64).saturating_mul(1024),
                uninstall_string: un,
                install_location,
                is_uwp: false,
            });
        }
        reg_close(hk);
    }

    out.extend(list_uwp_apps());
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn run_quiet_ps(script: &str) -> Option<String> {
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(util::CREATE_NO_WINDOW)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
}

/// UWP（Appx）应用：经 PowerShell Get-AppxPackage 枚举主包（排除框架/资源包）。
fn list_uwp_apps() -> Vec<InstalledApp> {
    const SCRIPT: &str = "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
        Get-AppxPackage | Where-Object { -not $_.IsFramework -and -not $_.IsResourcePackage } | \
        ForEach-Object { \"N:$($_.Name)`tP:$($_.Publisher)`tL:$($_.InstallLocation)`tF:$($_.PackageFullName)`tV:$($_.Version)\" }";
    let Some(out) = run_quiet_ps(SCRIPT) else {
        return Vec::new();
    };
    let mut apps = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if !line.starts_with("N:") {
            continue;
        }
        let (mut name, mut publisher, mut loc, mut full, mut ver) =
            (String::new(), String::new(), String::new(), String::new(), String::new());
        for f in line.split('\t') {
            if let Some(v) = f.strip_prefix("N:") {
                name = v.trim().to_string();
            } else if let Some(v) = f.strip_prefix("P:") {
                publisher = v.trim().to_string();
            } else if let Some(v) = f.strip_prefix("L:") {
                loc = v.trim().to_string();
            } else if let Some(v) = f.strip_prefix("F:") {
                full = v.trim().to_string();
            } else if let Some(v) = f.strip_prefix("V:") {
                ver = v.trim().to_string();
            }
        }
        if name.is_empty() || full.is_empty() || is_self(&name) {
            continue;
        }
        apps.push(InstalledApp {
            id: format!("uwp:{}", full),
            name,
            version: ver,
            publisher,
            install_date: String::new(),
            estimated_size: 0,
            uninstall_string: String::new(),
            install_location: loc,
            is_uwp: true,
        });
    }
    apps
}

// ---------------------------------------------------------------------------
// 卸载器调起
// ---------------------------------------------------------------------------

/// 解析 UninstallString → (程序, 参数)。
/// 处理：带引号路径 / 未引号路径（逐步取前缀找真实文件）/ MsiExec（首个空格切分）。
pub fn parse_uninstall_string(s: &str) -> Option<(String, String)> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    // 带引号：""C:\...\un.exe" args"
    if let Some(inner) = t.strip_prefix('"') {
        let end = inner.find('"')?;
        let prog = inner[..end].trim();
        if prog.is_empty() {
            return None;
        }
        let args = inner[end + 1..].trim().to_string();
        return Some((prog.to_string(), args));
    }
    // MsiExec：首个空格切分
    if t.to_lowercase().contains("msiexec") {
        return match t.find(' ') {
            Some(i) => Some((
                t[..i].trim().to_string(),
                t[i + 1..].trim().to_string(),
            )),
            None => Some((t.to_string(), String::new())),
        };
    }
    // 未加引号：按空格逐步扩展前缀，取第一个"确实存在的文件"作为程序路径
    let mut end = 0usize;
    let mut first_cut: Option<usize> = None;
    while let Some(rel) = t[end..].find(' ') {
        let cut = end + rel;
        if first_cut.is_none() {
            first_cut = Some(cut);
        }
        if Path::new(&t[..cut]).is_file() {
            return Some((
                t[..cut].to_string(),
                t[cut + 1..].trim().to_string(),
            ));
        }
        end = cut + 1;
    }
    if Path::new(t).is_file() {
        return Some((t.to_string(), String::new()));
    }
    // 兜底：首个空格切分（或整串）
    match first_cut {
        Some(cut) => Some((
            t[..cut].to_string(),
            t[cut + 1..].trim().to_string(),
        )),
        None => Some((t.to_string(), String::new())),
    }
}

/// MSI 参数归一化：MsiExec 的 /I{GUID} 是"修改"，卸载必须用 /X{GUID}。
pub fn normalize_msi_args(args: &str) -> String {
    let t = args.trim();
    match t.strip_prefix("/I") {
        Some(rest) => format!("/X{}", rest),
        None => t.to_string(),
    }
}

/// 拆分卸载命令参数（支持双引号包裹的含空格参数）。
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for c in s.chars() {
        match c {
            '"' => in_quote = !in_quote,
            ' ' | '\t' if !in_quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 调起程序自带卸载器（PRD 2.6）。Win32：spawn 后由后台线程等待退出并 emit
/// "uninstall-exited"；UWP：Remove-AppxPackage（同步执行后 emit）。
/// 返回仅代表"已成功调起"，卸载是否真正完成以残留扫描为准。
pub fn launch_uninstall(
    app: &AppHandle,
    uninstall_string: &str,
    display_name: &str,
    package_full_name: &str,
    is_uwp: bool,
) -> Result<(), String> {
    let app2 = app.clone();
    let name = display_name.to_string();

    if is_uwp {
        let pkg = package_full_name.trim().to_string();
        if pkg.is_empty() {
            return Err("缺少 UWP 包标识（PackageFullName）".to_string());
        }
        std::thread::spawn(move || {
            let script = format!("Remove-AppxPackage -Package '{}'", pkg);
            let ok = Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .creation_flags(util::CREATE_NO_WINDOW)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            // PowerShell 移除包后系统注销注册表需短暂时间
            std::thread::sleep(Duration::from_millis(1200));
            let _ = app2.emit(
                "uninstall-exited",
                serde_json::json!({ "displayName": name, "ok": ok }),
            );
        });
        return Ok(());
    }

    let (prog, args) =
        parse_uninstall_string(uninstall_string).ok_or("无法解析卸载命令")?;
    let prog_l = prog.to_lowercase();
    let args = if prog_l.ends_with("msiexec.exe") || prog_l.ends_with("msiexec") {
        normalize_msi_args(&args)
    } else {
        args
    };

    let mut cmd = Command::new(&prog);
    for a in split_args(&args) {
        cmd.arg(a);
    }
    // 不设 CREATE_NO_WINDOW：卸载器需要弹窗交互
    let mut child = cmd.spawn().map_err(|e| format!("调起卸载器失败：{}", e))?;

    std::thread::spawn(move || {
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        // 宽限：部分卸载器主进程先退出、再由子进程/提权流程收尾
        std::thread::sleep(Duration::from_millis(1500));
        let _ = app2.emit(
            "uninstall-exited",
            serde_json::json!({ "displayName": name, "ok": ok }),
        );
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// 目录级残留扫描（PRD 2.6；注册表残留不在 v1 范围）
// ---------------------------------------------------------------------------

/// 卸载残留扫描根（PRD 2.6）：ProgramFiles / ProgramFiles(x86) / ProgramData / LOCALAPPDATA。
pub fn residue_roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramData", "LOCALAPPDATA"] {
        if let Ok(v) = std::env::var(var) {
            let p = PathBuf::from(v);
            if p.is_dir() && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// 文本规范化：小写 + 去除空白与标点（保留字母数字与 CJK），用于启发式匹配。
pub fn normalize_text(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

/// 发布者匹配词元：按非字母数字分词，取 ≥4 字符且非常见企业通用词的部分。
pub fn publisher_tokens(publisher: &str) -> Vec<String> {
    const GENERIC: &[&str] = &[
        "microsoft", "corporation", "corp", "inc", "ltd", "llc", "gmbh", "the",
        "software", "systems", "system", "technologies", "technology", "group",
        "limited", "international", "application", "app", "team", "labs", "studio",
        "co", "company", "network", "solutions", "organization",
    ];
    publisher
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 4)
        .map(|w| w.to_lowercase())
        .filter(|w| !GENERIC.contains(&w.as_str()))
        .collect()
}

/// 目录名匹配判定（纯函数，可测试）：
/// - 名称匹配：规范化目录名等于程序名，或程序名 ≥4 字符且被目录名包含
/// - 发布者匹配：任一发布者词元被目录名包含
pub fn dir_match_reason(dn: &str, name: &str, pub_tokens: &[String]) -> Option<&'static str> {
    if name.chars().count() >= 2 && (dn == name || (name.chars().count() >= 4 && dn.contains(name)))
    {
        return Some("名称匹配");
    }
    for t in pub_tokens {
        if dn == t.as_str() || dn.contains(t.as_str()) {
            return Some("发布者匹配");
        }
    }
    None
}

/// 残留扫描核心（roots 可注入以便测试）：仅匹配各根第一层目录，匹配即计目录整体大小。
/// 上限 50 条，按大小降序。
pub fn scan_roots_for_match(
    roots: &[PathBuf],
    display_name: &str,
    publisher: &str,
    install_location: &str,
) -> Vec<ResidueDir> {
    let name = normalize_text(display_name);
    let tokens = publisher_tokens(publisher);
    let mut out: Vec<ResidueDir> = Vec::new();
    if name.chars().count() < 2 {
        return out;
    }
    // 自我排除：永不提示删除本应用
    if name.contains("diskclear") || display_name.contains("盘清") {
        return out;
    }
    let mut seen: HashSet<String> = HashSet::new();
    const MAX_RESULTS: usize = 50;

    let push = |p: &Path, reason: &str, out: &mut Vec<ResidueDir>, seen: &mut HashSet<String>| {
        if out.len() >= MAX_RESULTS {
            return;
        }
        let np = util::normalize_path(p);
        if np.is_empty() || !seen.insert(np.to_lowercase()) {
            return;
        }
        out.push(ResidueDir {
            path: np,
            size: util::dir_size(p),
            reason: reason.to_string(),
        });
    };

    // 1. 注册表 InstallLocation 直接命中
    if !install_location.is_empty() {
        let il = PathBuf::from(install_location.trim().trim_end_matches('\\'));
        if il.is_dir() && under_any_root(&il, roots) {
            push(&il, "注册表安装位置", &mut out, &mut seen);
        }
    }

    // 2. 各根第一层目录的启发式匹配
    for root in roots {
        let Ok(rd) = std::fs::read_dir(util::long_path(root)) else {
            continue;
        };
        for entry in rd.flatten() {
            let Ok(md) = entry.metadata() else { continue };
            if !md.is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let dn = normalize_text(&dir_name);
            if dn.is_empty() {
                continue;
            }
            if let Some(reason) = dir_match_reason(&dn, &name, &tokens) {
                push(&entry.path(), reason, &mut out, &mut seen);
            }
        }
    }

    out.sort_by(|a, b| b.size.cmp(&a.size));
    out
}

pub fn scan_residues(display_name: &str, publisher: &str, install_location: &str) -> Vec<ResidueDir> {
    scan_roots_for_match(&residue_roots(), display_name, publisher, install_location)
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_uninstall_string_quoted_with_args() {
        let (p, a) = parse_uninstall_string(
            r#""C:\Program Files\App\unins000.exe" /SILENT /LANG=zh"#,
        )
        .unwrap();
        assert_eq!(p, r"C:\Program Files\App\unins000.exe");
        assert_eq!(a, "/SILENT /LANG=zh");
    }

    #[test]
    fn parse_uninstall_string_msiexec() {
        let (p, a) =
            parse_uninstall_string(r"MsiExec.exe /I{A1B2C3D4-0000-1111-2222-333344445555}")
                .unwrap();
        assert_eq!(p, "MsiExec.exe");
        assert_eq!(normalize_msi_args(&a), "/X{A1B2C3D4-0000-1111-2222-333344445555}");
        // 已是 /X 的保持不变
        assert_eq!(normalize_msi_args("/X{GUID}"), "/X{GUID}");
        assert_eq!(normalize_msi_args(" /I {GUID} "), "/X {GUID}");
    }

    #[test]
    fn parse_uninstall_string_unquoted_with_spaces() {
        // 未加引号的含空格路径：逐步取前缀，命中真实文件
        let base = std::env::temp_dir()
            .join(format!("dc_uninst_{}", std::process::id()));
        let dir = base.join("My App Tools");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("tool.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        let s = format!("{} --quiet", exe.to_str().unwrap());
        let (p, a) = parse_uninstall_string(&s).unwrap();
        assert_eq!(p, exe.to_str().unwrap());
        assert_eq!(a, "--quiet");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn parse_uninstall_string_no_space_fallback() {
        let (p, a) = parse_uninstall_string(r"C:\Nothing\Here\un.exe").unwrap();
        assert_eq!(p, r"C:\Nothing\Here\un.exe");
        assert_eq!(a, "");
        assert!(parse_uninstall_string("   ").is_none());
    }

    #[test]
    fn split_args_respects_quotes() {
        let v = split_args(r#"/SILENT /DIR="C:\Program Files\App""#);
        assert_eq!(v, vec!["/SILENT", "/DIR=C:\\Program Files\\App"]);
    }

    #[test]
    fn normalize_text_keeps_cjk_drops_punct() {
        assert_eq!(normalize_text("AVG Antivirus 2024!"), "avgantivirus2024");
        assert_eq!(normalize_text("腾讯 会议（Pro）"), "腾讯会议pro");
    }

    #[test]
    fn publisher_tokens_filter_generic_words() {
        assert_eq!(publisher_tokens("VideoLAN Organization"), vec!["videolan"]);
        assert!(publisher_tokens("Microsoft Corporation").is_empty());
        // "7-Zip" 分词后 "Zip" 仅 3 字符被过滤；"Team" 属通用词被过滤
        assert_eq!(publisher_tokens("7-Zip Development Team"), vec!["development"]);
    }

    #[test]
    fn dir_match_reason_rules() {
        // 名称 ≥4 字符：包含即命中
        assert_eq!(dir_match_reason("avgantivirus", "avg", &[]), None); // 短名需精确
        assert_eq!(dir_match_reason("avg", "avg", &[]), Some("名称匹配"));
        assert_eq!(dir_match_reason("microsoftvscode", "vscode", &[]), Some("名称匹配"));
        // 发布者词元
        assert_eq!(
            dir_match_reason("videolan", "vlc", &["videolan".to_string()]),
            Some("发布者匹配")
        );
        assert_eq!(dir_match_reason("nothing", "xyz", &[]), None);
    }

    #[test]
    fn expand_env_resolves_defined_and_keeps_unknown() {
        std::env::set_var("DC_TEST_VAR_XYZ", "C:/tmp");
        assert_eq!(expand_env("%DC_TEST_VAR_XYZ%\\a"), "C:/tmp\\a");
        assert_eq!(expand_env("%DC_NO_SUCH_VAR_QQ%\\a"), "%DC_NO_SUCH_VAR_QQ%\\a");
    }

    #[test]
    fn format_install_date_compacts() {
        assert_eq!(format_install_date("20250103"), "2025-01-03");
        assert_eq!(format_install_date("unknown"), "unknown");
    }

    /// 真实注册表连通性：ProgramFilesDir 必然存在且指向 Program Files。
    #[test]
    fn registry_read_live_smoke() {
        let hk = reg_open(reg::HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion")
            .expect("打开 CurrentVersion 失败");
        let v = reg_string(hk, "ProgramFilesDir").expect("读 ProgramFilesDir 失败");
        assert!(v.to_lowercase().starts_with(r"c:\program files"), "got: {}", v);
        reg_close(hk);
    }

    /// 残留扫描核心：临时根下按名称命中、无关目录不误报、自我排除。
    #[test]
    fn scan_roots_for_match_finds_and_excludes() {
        let base = std::env::temp_dir().join(format!("dc_residue_{}", std::process::id()));
        let root = base.join("PF");
        let hit = root.join("AVG Antivirus");
        std::fs::create_dir_all(&hit).unwrap();
        std::fs::write(hit.join("data.bin"), vec![0u8; 100]).unwrap();
        std::fs::create_dir_all(root.join("Nothing")).unwrap();

        let out = scan_roots_for_match(&[root.clone()], "AVG Antivirus", "AVG Technologies", "");
        assert_eq!(out.len(), 1, "应只命中 AVG Antivirus：{:?}", out);
        assert!(out[0].path.contains("AVG Antivirus"));
        assert_eq!(out[0].size, 100);
        assert_eq!(out[0].reason, "名称匹配");

        // 发布者词元命中
        let out2 = scan_roots_for_match(&[root.clone()], " Totally Unrelated ", "AVG Technologies", "");
        assert!(out2.is_empty(), "名称无关时不应误报：{:?}", out2);

        // 自我排除
        let out3 = scan_roots_for_match(&[root], "DiskClear 盘清", "DiskClear", "");
        assert!(out3.is_empty());

        let _ = std::fs::remove_dir_all(&base);
    }
}
