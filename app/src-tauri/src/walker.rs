// 并行目录遍历器（PRD 5.3 / 2.7 扫描线程数 / 6.5 边界场景）
//
// - 多线程工作队列：pending 计数 = 已发现未完成目录数，队列空且 pending=0 时终止（无竞态）
// - 不跟随 junction/符号链接目录；文件级符号链接跳过（PRD 6.5）
// - 标记 OneDrive 云占位文件（PRD 6.5）
// - 阈值过滤在 worker 内完成，避免全盘小文件明细进入内存（PRD 5.3 聚合策略）
// - 进度：原子计数器 files_seen / bytes_seen，由调用方采样

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use crate::util;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkFile {
    pub path: String,
    pub size: u64,
    pub modified_ms: u64,
    pub is_cloud: bool,
}

pub enum WalkMsg {
    File(WalkFile),
    Done,
}

#[derive(Clone, Default)]
pub struct WalkCounters {
    pub files_seen: Arc<AtomicU64>,
    pub bytes_seen: Arc<AtomicU64>,
    pub skipped_dirs: Arc<AtomicU64>,
}

fn mtime_ms(md: &std::fs::Metadata) -> u64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 并行遍历给定的根目录，返回 (消息接收端, 工作线程句柄, 计数器)。
/// 只产出 size >= threshold 的普通文件；调用方收满 `threads` 个 Done 后结束。
pub fn walk_parallel(
    roots: Vec<PathBuf>,
    threads: usize,
    threshold: u64,
    cancelled: Arc<AtomicBool>,
) -> (Receiver<WalkMsg>, Vec<std::thread::JoinHandle<()>>, WalkCounters) {
    let threads = threads.clamp(1, 16);
    let queue: Arc<Mutex<VecDeque<PathBuf>>> = Arc::new(Mutex::new(VecDeque::new()));
    let pending = Arc::new(AtomicUsize::new(roots.len()));
    let counters = WalkCounters::default();
    let (tx, rx) = channel::<WalkMsg>();

    {
        let mut q = queue.lock().unwrap();
        for r in roots {
            q.push_back(r);
        }
    }

    let mut handles = Vec::with_capacity(threads);
    for _ in 0..threads {
        let queue = queue.clone();
        let pending = pending.clone();
        let cancelled = cancelled.clone();
        let counters = counters.clone();
        let tx = tx.clone();
        handles.push(std::thread::spawn(move || {
            let mut local: Vec<WalkMsg> = Vec::with_capacity(64);
            loop {
                if cancelled.load(Ordering::SeqCst) {
                    break;
                }
                let dir = queue.lock().unwrap().pop_front();
                match dir {
                    Some(dir) => {
                        match std::fs::read_dir(util::long_path(&dir)) {
                            Ok(rd) => {
                                for entry in rd.flatten() {
                                    if cancelled.load(Ordering::SeqCst) {
                                        break;
                                    }
                                    let Ok(md) = entry.metadata() else { continue };
                                    let is_dir = md.is_dir();
                                    let is_symlink = md.file_type().is_symlink();
                                    if is_dir {
                                        if !is_symlink && !util::is_reparse_dir(&md) {
                                            pending.fetch_add(1, Ordering::SeqCst);
                                            queue.lock().unwrap().push_back(entry.path());
                                        }
                                    } else if md.is_file() && !is_symlink {
                                        let size = md.len();
                                        counters.files_seen.fetch_add(1, Ordering::Relaxed);
                                        counters.bytes_seen.fetch_add(size, Ordering::Relaxed);
                                        if size >= threshold {
                                            let wf = WalkFile {
                                                path: util::normalize_path(&entry.path()),
                                                size,
                                                modified_ms: mtime_ms(&md),
                                                is_cloud: util::is_cloud_placeholder(&md),
                                            };
                                            local.push(WalkMsg::File(wf));
                                            if local.len() >= 128 {
                                                for m in local.drain(..) {
                                                    let _ = tx.send(m);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Err(_) => {
                                counters.skipped_dirs.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        pending.fetch_sub(1, Ordering::SeqCst);
                    }
                    None => {
                        if pending.load(Ordering::SeqCst) == 0 {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
            }
            for m in local.drain(..) {
                let _ = tx.send(m);
            }
            let _ = tx.send(WalkMsg::Done);
        }));
    }

    (rx, handles, counters)
}

/// 收集遍历结果直到所有 worker 完成（或取消）。max_files 限制明细条数（PRD 5.3 上限策略）。
pub fn collect(
    rx: Receiver<WalkMsg>,
    handles: Vec<std::thread::JoinHandle<()>>,
    cancelled: &AtomicBool,
    max_files: usize,
) -> (Vec<WalkFile>, bool) {
    let mut files = Vec::new();
    let mut truncated = false;
    let mut done = 0usize;
    while done < handles.len() {
        match rx.recv_timeout(Duration::from_millis(120)) {
            Ok(WalkMsg::File(f)) => {
                if files.len() < max_files {
                    files.push(f);
                } else {
                    truncated = true;
                }
            }
            Ok(WalkMsg::Done) => done += 1,
            Err(_) => {
                if cancelled.load(Ordering::SeqCst) {
                    break;
                }
            }
        }
    }
    for h in handles {
        let _ = h.join();
    }
    (files, truncated)
}

/// FNV-1a 64 位：首 4KB 预筛哈希（非加密用途，速度优先）。
pub fn fnv64a(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 分块读取计算文件 MD5（PRD 2.4：单块 1MB）。失败返回 None。
pub fn md5_of_file(path: &str) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(util::long_path(path)).ok()?;
    let mut ctx = md5::Context::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        ctx.consume(&buf[..n]);
    }
    Some(format!("{:x}", ctx.compute()))
}

/// 读取文件首 4KB（预筛用）。失败返回 None。
pub fn read_head(path: &str, len: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(util::long_path(path)).ok()?;
    let mut buf = vec![0u8; len];
    let mut read = 0usize;
    while read < len {
        let n = f.read(&mut buf[read..]).ok()?;
        if n == 0 {
            break;
        }
        read += n;
    }
    buf.truncate(read);
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv64a_known_vectors() {
        // FNV-1a 64 标准测试向量
        assert_eq!(fnv64a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv64a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv64a(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn md5_known_vector() {
        let dir = std::env::temp_dir().join(format!("dc_md5_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("abc.txt");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            md5_of_file(p.to_str().unwrap()).as_deref(),
            Some("900150983cd24fb0d6963f7d28e17f72")
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn walk_parallel_finds_files_and_skips_junction_loops() {
        use std::collections::HashSet;
        use std::io::Write;
        use std::os::windows::process::CommandExt;
        let base = std::env::temp_dir().join(format!("dc_walk_test_{}", std::process::id()));
        let sub = base.join("sub").join("deep");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(base.join("big1.bin"), vec![0u8; 4096]).unwrap();
        std::fs::write(sub.join("big2.bin"), vec![1u8; 2048]).unwrap();
        let mut small = std::fs::File::create(base.join("small.txt")).unwrap();
        small.write_all(b"tiny").unwrap();
        drop(small);

        // junction 环：sub/loop → base（若不跳过 reparse 会无限循环）
        let _ = std::fs::remove_dir(sub.join("loop"));
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(sub.join("loop"))
            .arg(&base)
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(out.status.success(), "mklink /J 失败：{:?}", String::from_utf8_lossy(&out.stderr));

        let cancelled = Arc::new(AtomicBool::new(false));
        let (rx, handles, counters) = walk_parallel(
            vec![base.clone()],
            4,
            1024, // 只收 ≥1KB
            cancelled,
        );
        let (files, truncated) = collect(rx, handles, &AtomicBool::new(false), 1000);

        let names: HashSet<String> = files
            .iter()
            .map(|f| {
                std::path::Path::new(&f.path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert!(names.contains("big1.bin"));
        assert!(names.contains("big2.bin"));
        assert!(!names.contains("small.txt"), "低于阈值文件应被过滤");
        assert!(!truncated);
        assert!(counters.files_seen.load(Ordering::Relaxed) >= 3);
        let _ = std::fs::remove_dir_all(&base);
    }
}
