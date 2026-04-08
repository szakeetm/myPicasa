use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::PathBuf,
};

use crate::models::CacheStats;

#[derive(Clone, Default)]
pub struct ThumbnailCache {
    cache_dir: PathBuf,
    max_bytes: usize,
    current_bytes: usize,
    persisted_bytes: usize,
    persisted_items: usize,
    order: VecDeque<String>,
    values: HashMap<String, Vec<u8>>,
}

impl ThumbnailCache {
    pub fn new(cache_dir: PathBuf, max_bytes: usize) -> Self {
        let mut cache = Self {
            cache_dir,
            max_bytes,
            ..Self::default()
        };
        let _ = fs::create_dir_all(&cache.cache_dir);
        cache.refresh_disk_stats();
        cache
    }

    pub fn get(&mut self, key: &str) -> Option<Vec<u8>> {
        if let Some(value) = self.values.get(key).cloned() {
            self.touch(key);
            return Some(value);
        }

        let path = self.path_for_key(key);
        let value = fs::read(path).ok()?;
        self.insert_memory_only(key.to_string(), value.clone());
        Some(value)
    }

    pub fn insert(&mut self, key: String, value: Vec<u8>) {
        self.write_to_disk(&key, &value);
        self.insert_memory_only(key, value);
    }

    fn touch(&mut self, key: &str) {
        self.order.retain(|existing| existing != key);
        self.order.push_back(key.to_string());
    }

    fn insert_memory_only(&mut self, key: String, value: Vec<u8>) {
        if let Some(previous) = self.values.remove(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(previous.len());
            self.order.retain(|existing| existing != &key);
        }

        self.current_bytes += value.len();
        self.order.push_back(key.clone());
        self.values.insert(key, value);
        self.evict();
    }

    fn evict(&mut self) {
        while self.current_bytes > self.max_bytes {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(value) = self.values.remove(&oldest) {
                self.current_bytes = self.current_bytes.saturating_sub(value.len());
            }
        }
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            thumbnail_items: self.persisted_items as u32,
            thumbnail_bytes: self.persisted_bytes as u64,
            thumbnail_budget_bytes: self.max_bytes as u64,
            viewer_render_items: 0,
            viewer_render_bytes: 0,
        }
    }

    pub fn clear(&mut self) {
        self.current_bytes = 0;
        self.order.clear();
        self.values.clear();
        self.persisted_bytes = 0;
        self.persisted_items = 0;
        let _ = fs::remove_dir_all(&self.cache_dir);
        let _ = fs::create_dir_all(&self.cache_dir);
    }

    fn write_to_disk(&mut self, key: &str, value: &[u8]) {
        let path = self.path_for_key(key);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let previous_len = fs::metadata(&path).ok().map(|meta| meta.len() as usize);
        if fs::write(&path, value).is_ok() {
            match previous_len {
                Some(previous_len) => {
                    self.persisted_bytes =
                        self.persisted_bytes.saturating_sub(previous_len) + value.len();
                }
                None => {
                    self.persisted_items += 1;
                    self.persisted_bytes += value.len();
                }
            }
        }
    }

    fn refresh_disk_stats(&mut self) {
        self.persisted_items = 0;
        self.persisted_bytes = 0;
        if let Ok(entries) = fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        self.persisted_items += 1;
                        self.persisted_bytes += metadata.len() as usize;
                    }
                }
            }
        }
    }

    fn path_for_key(&self, key: &str) -> PathBuf {
        self.cache_dir.join(cache_filename(key))
    }
}

fn cache_filename(key: &str) -> String {
    let digest = blake3::hash(key.as_bytes());
    format!("{}.jpg", digest.to_hex())
}
