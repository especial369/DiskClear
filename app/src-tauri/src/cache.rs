// 重复文件哈希缓存（PRD 2.4：大小+修改时间未变则复用哈希，避免全量重读）
//
// 存储：appdata/dupe_cache.json；键为路径，值为 {size, mtimeMs, hash}。
// 容量上限 10 万条，超限整体重置（重建成本低）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 100_000;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CacheVal {
    pub size: u64,
    pub mtime_ms: u64,
    pub hash: String,
}

#[derive(Default)]
pub struct HashCache {
    map: Mutex<Option<HashMap<String, CacheVal>>>,
    dirty: Mutex<bool>,
}

fn cache_path() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|base| PathBuf::from(base).join("com.diskclear.app").join("dupe_cache.json"))
}

impl HashCache {
    fn ensure_loaded(&self) {
        let mut guard = self.map.lock().unwrap();
        if guard.is_some() {
            return;
        }
        let map = cache_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<HashMap<String, CacheVal>>(&s).ok())
            .unwrap_or_default();
        *guard = Some(map);
    }

    /// 命中条件：size 与 mtime 均未变化（PRD 2.4）。
    pub fn get(&self, path: &str, size: u64, mtime_ms: u64) -> Option<String> {
        self.ensure_loaded();
        let guard = self.map.lock().unwrap();
        let map = guard.as_ref()?;
        let v = map.get(path)?;
        if v.size == size && v.mtime_ms == mtime_ms {
            Some(v.hash.clone())
        } else {
            None
        }
    }

    pub fn put(&self, path: &str, size: u64, mtime_ms: u64, hash: String) {
        self.ensure_loaded();
        {
            let mut guard = self.map.lock().unwrap();
            let map = guard.as_mut().unwrap();
            if map.len() >= MAX_ENTRIES && !map.contains_key(path) {
                map.clear(); // 超限重置
            }
            map.insert(
                path.to_string(),
                CacheVal {
                    size,
                    mtime_ms,
                    hash,
                },
            );
        }
        *self.dirty.lock().unwrap() = true;
    }

    pub fn save_if_dirty(&self) {
        let dirty = *self.dirty.lock().unwrap();
        if !dirty {
            return;
        }
        let guard = self.map.lock().unwrap();
        if let (Some(map), Some(p)) = (guard.as_ref(), cache_path()) {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string(map) {
                let _ = std::fs::write(p, json);
            }
        }
        *self.dirty.lock().unwrap() = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_requires_same_size_and_mtime() {
        let c = HashCache::default();
        c.put(r"C:\a.bin", 100, 500, "deadbeef".into());
        assert_eq!(c.get(r"C:\a.bin", 100, 500).as_deref(), Some("deadbeef"));
        assert_eq!(c.get(r"C:\a.bin", 101, 500), None); // 大小变了
        assert_eq!(c.get(r"C:\a.bin", 100, 501), None); // 修改时间变了
        assert_eq!(c.get(r"C:\other.bin", 100, 500), None);
    }
}
