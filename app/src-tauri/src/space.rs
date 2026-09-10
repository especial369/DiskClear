// 空间分析（PRD 2.5）：目录级聚合树 + Treemap 可视化
//
// - 顶层子目录多线程并行建子树，arena 节点 + 规范化路径索引支持 O(1) 下钻
// - 不跟随 junction/reparse（PRD 6.5）；跳过 $Recycle.Bin / $DiskClear / System Volume Information
// - 每个目录统计：子树大小、文件数、按 8 类文件类型的字节数（主导类型着色）
// - 传输树深度受限（SERIALIZE_DEPTH），更深下钻经 get_dir_detail 懒加载；
//   单层子节点超 MAX_CHILDREN 时聚合为"其他"（PRD 2.5 渲染上限约束）
// - 会话上限 MAX_SESSIONS（arena 内存较大，溢出淘汰最旧）

use std::collections::{HashMap, VecDeque};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::settings;
use crate::util;

/// 对外序列化的目录树深度（根为 0 层）
pub const SERIALIZE_DEPTH: usize = 3;
/// 会话上限（超出淘汰最旧）
pub const MAX_SESSIONS: usize = 3;
/// 单层返回子节点上限，超出部分聚合为"其他"（PRD 2.5：单屏渲染节点上限 2000）
pub const MAX_CHILDREN: usize = 2000;

/// FILE_ATTRIBUTE_DIRECTORY（0x10；std 元数据场景需按原始位判断目录链接）
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0010;

pub fn file_category(name: &str) -> u8 {
    let Some(dot) = name.rfind('.') else { return 0 };
    if dot == 0 || name.len() - dot - 1 > 5 {
        return 0; // 无扩展名或扩展名异常长（哈希名等）→ 其他
    }
    match name[dot + 1..].to_lowercase().as_str() {
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "m4v" | "webm" | "ts" => 1,
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "raw" | "psd" | "heic" => 2,
        "mp3" | "wav" | "flac" | "ape" | "ogg" | "m4a" | "aac" => 3,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => 4,
        "iso" | "img" | "vhd" | "vhdx" | "gho" | "wim" | "dmg" => 5,
        "exe" | "msi" | "msix" | "appx" => 6,
        "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "pdf" | "txt" | "md" => 7,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// 内部数据结构（arena）
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct NodeData {
    pub name: String,
    /// 规范化路径（无 \\?\ 前缀）
    pub path: String,
    /// 子树聚合大小
    pub size: u64,
    /// 直接文件大小
    pub own_size: u64,
    /// 子树文件数
    pub file_count: u64,
    /// 直接文件数
    pub own_files: u64,
    /// 8 类文件类型的子树字节数
    pub cats: [u64; 8],
    pub children: Vec<usize>,
    /// 根节点 parent = 自身索引
    pub parent: usize,
}

pub struct SpaceSession {
    pub session_id: u32,
    pub drive: String,
    pub done: bool,
    pub nodes: Vec<NodeData>,
    /// 小写规范化路径 → arena 下标
    pub index: HashMap<String, usize>,
    pub dir_count: u64,
    pub skipped_dirs: u64,
}

#[derive(Default)]
struct WalkStats {
    dirs: AtomicU64,
    files: AtomicU64,
    bytes: AtomicU64,
    skipped: AtomicU64,
}

// ---------------------------------------------------------------------------
// 对外 DTO
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DirNodeDto {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub file_count: u64,
    /// 主导文件类型类别 id（0-7，与前端 types.ts::SPACE_CATEGORY_LABELS 对齐）
    pub dominant: u8,
    pub has_children: bool,
    pub children: Vec<DirNodeDto>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SpaceTreeDto {
    pub session_id: u32,
    pub drive: String,
    pub done: bool,
    pub total_bytes: u64,
    pub file_count: u64,
    pub dir_count: u64,
    pub skipped_dirs: u64,
    pub root: DirNodeDto,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DirDetailDto {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub file_count: u64,
    pub dominant: u8,
    /// 子目录（深度 1，按大小降序）
    pub children: Vec<DirNodeDto>,
}

fn dominant_of(cats: &[u64; 8]) -> u8 {
    let mut best = 0u8;
    let mut best_v = cats[0];
    for (i, &v) in cats.iter().enumerate().skip(1) {
        if v > best_v {
            best_v = v;
            best = i as u8;
        }
    }
    best
}

/// arena 节点 → DTO；深度超过 SERIALIZE_DEPTH 后不再展开（仅 hasChildren 标记）；
/// 子节点超 MAX_CHILDREN 时保留最大的 MAX_CHILDREN-1 个 + "其他"聚合节点。
fn node_to_dto(nodes: &[NodeData], idx: usize, depth: usize) -> DirNodeDto {
    let nd = &nodes[idx];
    let mut children_dto: Vec<DirNodeDto> = Vec::new();
    if depth + 1 <= SERIALIZE_DEPTH {
        let mut kids: Vec<usize> = nd.children.clone();
        kids.sort_by(|a, b| nodes[*b].size.cmp(&nodes[*a].size));
        if kids.len() > MAX_CHILDREN {
            let cut = MAX_CHILDREN - 1;
            for k in &kids[..cut] {
                children_dto.push(node_to_dto(nodes, *k, depth + 1));
            }
            let mut rest_size = 0u64;
            let mut rest_files = 0u64;
            for k in &kids[cut..] {
                rest_size += nodes[*k].size;
                rest_files += nodes[*k].file_count;
            }
            children_dto.push(DirNodeDto {
                name: "其他".to_string(),
                path: nd.path.clone(),
                size: rest_size,
                file_count: rest_files,
                dominant: 0,
                has_children: false,
                children: Vec::new(),
            });
        } else {
            for k in kids {
                children_dto.push(node_to_dto(nodes, k, depth + 1));
            }
        }
    }
    DirNodeDto {
        name: nd.name.clone(),
        path: nd.path.clone(),
        size: nd.size,
        file_count: nd.file_count,
        dominant: dominant_of(&nd.cats),
        has_children: !nd.children.is_empty(),
        children: children_dto,
    }
}

// ---------------------------------------------------------------------------
// 树构建
// ---------------------------------------------------------------------------

fn is_excluded_dir(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "$recycle.bin" | "$diskclear" | "system volume information"
    )
}

/// 单个子树构建（worker 内执行）：迭代 DFS。
/// 节点创建顺序保证 parent 索引 < child 索引（后续逆序累加的前提）。
fn build_subtree(
    root_path: &Path,
    root_name: &str,
    cancelled: &AtomicBool,
    stats: &WalkStats,
    nodes: &mut Vec<NodeData>,
    index: &mut HashMap<String, usize>,
) -> usize {
    let root_idx = nodes.len();
    nodes.push(NodeData {
        name: root_name.to_string(),
        path: util::normalize_path(root_path),
        parent: usize::MAX, // 合并时指向盘根
        ..Default::default()
    });
    index.insert(nodes[root_idx].path.to_lowercase(), root_idx);
    stats.dirs.fetch_add(1, Ordering::Relaxed);

    let mut stack: Vec<(PathBuf, usize)> = vec![(root_path.to_path_buf(), root_idx)];
    while let Some((dir, idx)) = stack.pop() {
        if cancelled.load(Ordering::SeqCst) {
            break;
        }
        let Ok(rd) = std::fs::read_dir(util::long_path(&dir)) else {
            stats.skipped.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        for entry in rd.flatten() {
            if cancelled.load(Ordering::SeqCst) {
                break;
            }
            let Ok(md) = entry.metadata() else { continue };
            let is_symlink = md.file_type().is_symlink();
            // junction/符号链接/其余 reparse 一律不跟随（PRD 6.5）。
            // 注意：DirEntry::metadata() 不穿越链接，junction 的 is_dir() 为 false
            // （file_type 被归为 symlink），须按 FILE_ATTRIBUTE_DIRECTORY 属性位识别目录链接。
            if is_symlink || util::is_reparse_dir(&md) {
                if md.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    stats.skipped.fetch_add(1, Ordering::Relaxed);
                }
                continue;
            }
            if md.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if is_excluded_dir(&name) {
                    continue;
                }
                let child_path = entry.path();
                let child_idx = nodes.len();
                let np = util::normalize_path(&child_path);
                nodes.push(NodeData {
                    name,
                    path: np.clone(),
                    parent: idx,
                    ..Default::default()
                });
                index.insert(np.to_lowercase(), child_idx);
                nodes[idx].children.push(child_idx);
                stats.dirs.fetch_add(1, Ordering::Relaxed);
                stack.push((child_path, child_idx));
            } else if md.is_file() {
                let size = md.len();
                let fname = entry.file_name().to_string_lossy().to_string();
                let cat = file_category(&fname) as usize;
                let nd = &mut nodes[idx];
                nd.own_size += size;
                nd.own_files += 1;
                nd.cats[cat] += size;
                stats.files.fetch_add(1, Ordering::Relaxed);
                stats.bytes.fetch_add(size, Ordering::Relaxed);
            }
        }
    }
    root_idx
}

/// 子树聚合：size/file_count/cats 自下而上累加。
/// 前提：所有边的 parent 索引 < child 索引，逆序遍历即可保证处理子节点先于父节点。
fn accumulate(nodes: &mut [NodeData]) {
    for n in nodes.iter_mut() {
        n.size = n.own_size;
        n.file_count = n.own_files;
    }
    for i in (1..nodes.len()).rev() {
        let p = nodes[i].parent;
        if p == usize::MAX || p == i {
            continue;
        }
        let (size, files, cats) = {
            let n = &nodes[i];
            (n.size, n.file_count, n.cats)
        };
        let pn = &mut nodes[p];
        pn.size += size;
        pn.file_count += files;
        for k in 0..8 {
            pn.cats[k] += cats[k];
        }
    }
}

pub fn start_space_scan(app: AppHandle, state: &crate::AppState, drive: String) -> u32 {
    // 会话上限：淘汰最旧（arena 内存较大）
    {
        let mut map = state.space_sessions.lock().unwrap();
        while map.len() >= MAX_SESSIONS {
            let Some(min_id) = map.keys().copied().min() else { break };
            map.remove(&min_id);
        }
    }
    let id = state.next_session.fetch_add(1, Ordering::SeqCst);
    let cancelled = Arc::new(AtomicBool::new(false));
    state.cancels.lock().unwrap().insert(id, cancelled.clone());

    let app2 = app.clone();
    std::thread::spawn(move || {
        let session = run_build(app2.clone(), id, drive, cancelled);
        let st = app2.state::<crate::AppState>();
        st.space_sessions.lock().unwrap().insert(id, session);
    });
    id
}

fn run_build(app: AppHandle, id: u32, drive: String, cancelled: Arc<AtomicBool>) -> SpaceSession {
    let threads = settings::load().scan_threads.clamp(1, 16) as usize;
    let root_path = PathBuf::from(format!("{}:\\", drive));
    let stats = Arc::new(WalkStats::default());

    // 进度采样线程：每 400ms 推送
    let progress_stop = Arc::new(AtomicBool::new(false));
    {
        let app2 = app.clone();
        let stop = progress_stop.clone();
        let cancelled2 = cancelled.clone();
        let stats2 = stats.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) && !cancelled2.load(Ordering::SeqCst) {
                let _ = app2.emit(
                    "space-progress",
                    serde_json::json!({
                        "sessionId": id,
                        "dirs": stats2.dirs.load(Ordering::Relaxed),
                        "files": stats2.files.load(Ordering::Relaxed),
                        "bytes": stats2.bytes.load(Ordering::Relaxed),
                    }),
                );
                std::thread::sleep(Duration::from_millis(400));
            }
        });
    }

    let mut nodes: Vec<NodeData> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    // 盘根节点
    nodes.push(NodeData {
        name: format!("{} 盘", drive),
        path: util::normalize_path(&root_path),
        parent: 0, // 指向自身，累加时跳过
        ..Default::default()
    });
    index.insert(nodes[0].path.to_lowercase(), 0);

    // 顶层条目：目录入工作队列（跳过排除项/reparse），散落文件直接计入盘根
    let queue: Arc<Mutex<VecDeque<(PathBuf, String)>>> = Arc::new(Mutex::new(VecDeque::new()));
    if let Ok(rd) = std::fs::read_dir(util::long_path(&root_path)) {
        for entry in rd.flatten() {
            if cancelled.load(Ordering::SeqCst) {
                break;
            }
            let Ok(md) = entry.metadata() else { continue };
            let is_symlink = md.file_type().is_symlink();
            let name = entry.file_name().to_string_lossy().to_string();
            if md.is_dir() {
                if is_symlink || util::is_reparse_dir(&md) || is_excluded_dir(&name) {
                    continue;
                }
                queue.lock().unwrap().push_back((entry.path(), name));
            } else if md.is_file() && !is_symlink {
                let size = md.len();
                let cat = file_category(&name) as usize;
                let nd = &mut nodes[0];
                nd.own_size += size;
                nd.own_files += 1;
                nd.cats[cat] += size;
            }
        }
    }

    // 并行构建各顶层子树
    let results: Arc<Mutex<Vec<(Vec<NodeData>, HashMap<String, usize>, usize)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for _ in 0..threads {
        let queue = queue.clone();
        let results = results.clone();
        let cancelled = cancelled.clone();
        let stats = stats.clone();
        handles.push(std::thread::spawn(move || loop {
            if cancelled.load(Ordering::SeqCst) {
                break;
            }
            let Some((top_path, top_name)) = queue.lock().unwrap().pop_front() else {
                break;
            };
            let mut local_nodes = Vec::new();
            let mut local_index: HashMap<String, usize> = HashMap::new();
            let local_root = build_subtree(
                &top_path,
                &top_name,
                &cancelled,
                &stats,
                &mut local_nodes,
                &mut local_index,
            );
            results
                .lock()
                .unwrap()
                .push((local_nodes, local_index, local_root));
        }));
    }
    for h in handles {
        let _ = h.join();
    }

    // 合并子树到全局 arena（索引整体平移）
    let merged = results.lock().unwrap();
    for (local_nodes, local_index, local_root) in merged.iter() {
        let base = nodes.len();
        for mut n in local_nodes.iter().cloned() {
            n.parent = if n.parent == usize::MAX { 0 } else { n.parent + base };
            n.children = n.children.iter().map(|c| c + base).collect();
            nodes.push(n);
        }
        for (p, i) in local_index.iter() {
            index.insert(p.clone(), i + base);
        }
        nodes[0].children.push(local_root + base);
    }
    drop(merged);

    accumulate(&mut nodes);

    let dir_count = stats.dirs.load(Ordering::Relaxed);
    let skipped_dirs = stats.skipped.load(Ordering::Relaxed);
    progress_stop.store(true, Ordering::SeqCst);

    let _ = app.emit(
        "space-done",
        serde_json::json!({ "sessionId": id, "cancelled": cancelled.load(Ordering::SeqCst) }),
    );

    SpaceSession {
        session_id: id,
        drive,
        done: true,
        nodes,
        index,
        dir_count: dir_count.saturating_add(1), // 含盘根
        skipped_dirs,
    }
}

// ---------------------------------------------------------------------------
// 查询
// ---------------------------------------------------------------------------

pub fn cancel(state: &crate::AppState, session_id: u32) {
    if let Some(c) = state.cancels.lock().unwrap().get(&session_id) {
        c.store(true, Ordering::SeqCst);
    }
}

pub fn get_result(state: &crate::AppState, session_id: u32) -> Option<SpaceTreeDto> {
    let map = state.space_sessions.lock().unwrap();
    let s = map.get(&session_id)?;
    let root = node_to_dto(&s.nodes, 0, 0);
    Some(SpaceTreeDto {
        session_id: s.session_id,
        drive: s.drive.clone(),
        done: s.done,
        total_bytes: s.nodes[0].size,
        file_count: s.nodes[0].file_count,
        dir_count: s.dir_count,
        skipped_dirs: s.skipped_dirs,
        root,
    })
}

/// 懒下钻：按规范化路径返回该目录的直接子目录（深度 1，按大小降序）。
pub fn get_dir_detail(
    state: &crate::AppState,
    session_id: u32,
    path: &str,
) -> Option<DirDetailDto> {
    let map = state.space_sessions.lock().unwrap();
    let s = map.get(&session_id)?;
    let key = util::normalize_path(Path::new(path)).to_lowercase();
    let idx = *s.index.get(&key)?;
    let nd = &s.nodes[idx];
    let mut kids: Vec<usize> = nd.children.clone();
    kids.sort_by(|a, b| s.nodes[*b].size.cmp(&s.nodes[*a].size));
    let children = kids
        .iter()
        .map(|k| {
            let n = &s.nodes[*k];
            DirNodeDto {
                name: n.name.clone(),
                path: n.path.clone(),
                size: n.size,
                file_count: n.file_count,
                dominant: dominant_of(&n.cats),
                has_children: !n.children.is_empty(),
                children: Vec::new(),
            }
        })
        .collect();
    Some(DirDetailDto {
        path: nd.path.clone(),
        name: nd.name.clone(),
        size: nd.size,
        file_count: nd.file_count,
        dominant: dominant_of(&nd.cats),
        children,
    })
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_category_maps_extensions() {
        assert_eq!(file_category("movie.mkv"), 1);
        assert_eq!(file_category("photo.JPG"), 2);
        assert_eq!(file_category("song.flac"), 3);
        assert_eq!(file_category("arch.7z"), 4);
        assert_eq!(file_category("disk.iso"), 5);
        assert_eq!(file_category("setup.exe"), 6);
        assert_eq!(file_category("readme.md"), 7);
        assert_eq!(file_category("unknown.xyz"), 0);
        assert_eq!(file_category("noext"), 0);
        // 扩展名异常长（哈希名）→ 其他
        assert_eq!(file_category("data.0f3a9b2c1d"), 0);
    }

    #[test]
    fn dominant_of_picks_largest_category() {
        let cats = [10, 5, 0, 0, 0, 0, 0, 0];
        assert_eq!(dominant_of(&cats), 0);
        let cats2 = [1, 2, 9, 0, 0, 0, 0, 0];
        assert_eq!(dominant_of(&cats2), 2);
    }

    #[test]
    fn build_and_accumulate_subtree() {
        let base = std::env::temp_dir().join(format!("dc_space_{}", std::process::id()));
        let sub = base.join("app");
        let deep = sub.join("data");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(base.join("root.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(sub.join("a.txt"), vec![0u8; 200]).unwrap();
        std::fs::write(deep.join("b.bin"), vec![0u8; 400]).unwrap();

        let cancelled = AtomicBool::new(false);
        let stats = WalkStats::default();
        let mut nodes = Vec::new();
        let mut index = HashMap::new();
        let root = build_subtree(&base, "dc_space", &cancelled, &stats, &mut nodes, &mut index);
        assert_eq!(root, 0);
        accumulate(&mut nodes);

        // 根：直接文件 100 + 子树 600 = 700；文件数 3
        assert_eq!(nodes[0].size, 700);
        assert_eq!(nodes[0].file_count, 3);
        // app 子目录：200 + 400 = 600
        let app_idx = *index.get(&util::normalize_path(&sub).to_lowercase()).unwrap();
        assert_eq!(nodes[app_idx].size, 600);
        // data 深层：400
        let deep_idx = *index.get(&util::normalize_path(&deep).to_lowercase()).unwrap();
        assert_eq!(nodes[deep_idx].size, 400);
        assert_eq!(nodes[deep_idx].own_size, 400);
        // 主导类型：txt(文档 200) + bin(其他 400) → 其他（0）
        assert_eq!(dominant_of(&nodes[app_idx].cats), 0);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn build_subtree_skips_junction_loops() {
        use std::io::Write;
        use std::os::windows::process::CommandExt;
        let base = std::env::temp_dir().join(format!("dc_space_j_{}", std::process::id()));
        let sub = base.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let mut f = std::fs::File::create(base.join("big.bin")).unwrap();
        f.write_all(&vec![0u8; 512]).unwrap();
        drop(f);
        // junction 环：sub/loop → base
        let _ = std::fs::remove_dir(sub.join("loop"));
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(sub.join("loop"))
            .arg(&base)
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(out.status.success(), "mklink /J 失败");

        let cancelled = AtomicBool::new(false);
        let stats = WalkStats::default();
        let mut nodes = Vec::new();
        let mut index = HashMap::new();
        let _root = build_subtree(&base, "jtest", &cancelled, &stats, &mut nodes, &mut index);
        accumulate(&mut nodes);
        // 诊断：junction 条目的实际运行时行为
        eprintln!(
            "DEBUG files={} skipped={} dirs={} nodes={}",
            stats.files.load(Ordering::Relaxed),
            stats.skipped.load(Ordering::Relaxed),
            stats.dirs.load(Ordering::Relaxed),
            nodes.len()
        );
        for (i, n) in nodes.iter().enumerate() {
            eprintln!("DEBUG node[{}] {} parent={}", i, n.path, n.parent);
        }
        // 若跟随 junction 会无限循环（测试超时即失败）；正常情况只统计 1 个文件
        assert_eq!(nodes[0].file_count, 1);
        assert!(
            stats.skipped.load(Ordering::Relaxed) >= 1,
            "junction 应计为 skipped"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn dto_depth_limit_and_has_children() {
        // 手工 arena：root(0) → a(1) → b(2) → c(3) → d(4)
        let mut nodes = Vec::new();
        nodes.push(NodeData { name: "root".into(), path: "C:\\".into(), parent: 0, ..Default::default() });
        for (i, name) in ["a", "b", "c", "d"].iter().enumerate() {
            let idx = nodes.len();
            let parent = idx - 1;
            nodes.push(NodeData {
                name: name.to_string(),
                path: format!(r"C:\{}", name),
                parent,
                size: 10,
                file_count: 1,
                ..Default::default()
            });
            nodes[parent].children.push(idx);
            let _ = i;
        }
        let dto = node_to_dto(&nodes, 0, 0);
        // 深度 0:root → 1:a → 2:b → 3:c（c 为 SERIALIZE_DEPTH 层，children 不展开）
        let a = &dto.children[0];
        let b = &a.children[0];
        let c = &b.children[0];
        assert_eq!(c.name, "c");
        assert!(c.children.is_empty(), "超过 SERIALIZE_DEPTH 不应展开");
        assert!(c.has_children, "但应标记 has_children");
    }

    #[test]
    fn dto_merges_overflow_into_other() {
        let mut nodes = Vec::new();
        nodes.push(NodeData { name: "root".into(), path: "C:\\".into(), parent: 0, ..Default::default() });
        let total = MAX_CHILDREN + 5;
        for i in 0..total {
            let idx = nodes.len();
            nodes.push(NodeData {
                name: format!("d{}", i),
                path: format!(r"C:\d{}", i),
                parent: 0,
                size: (i + 1) as u64,
                file_count: 1,
                ..Default::default()
            });
            nodes[0].children.push(idx);
        }
        let dto = node_to_dto(&nodes, 0, 0);
        assert_eq!(dto.children.len(), MAX_CHILDREN, "应保留 MAX_CHILDREN-1 个 + 其他");
        let other = dto.children.last().unwrap();
        assert_eq!(other.name, "其他");
        assert!(!other.has_children);
        // 被聚合的是最小的 6 个（1..=6），其余按大小降序保留
        let expected_rest: u64 = (1..=6u64).sum();
        assert_eq!(other.size, expected_rest);
        // 总量守恒：保留节点 + 其他 = 全部（真实场景由 accumulate 计算根 size，此处手工等价设置）
        nodes[0].size = (1..=total as u64).sum();
        let dto2 = node_to_dto(&nodes, 0, 0);
        let kept: u64 = dto2.children[..MAX_CHILDREN - 1].iter().map(|c| c.size).sum();
        assert_eq!(kept + dto2.children.last().unwrap().size, dto2.size);
    }

    #[test]
    fn is_excluded_dir_names() {
        assert!(is_excluded_dir("$Recycle.Bin"));
        assert!(is_excluded_dir("$DISKCLEAR"));
        assert!(is_excluded_dir("System Volume Information"));
        assert!(!is_excluded_dir("Program Files"));
    }
}
