use std::collections::{HashMap, VecDeque};

use crate::models::CacheStats;

#[derive(Clone, Default)]
pub struct ThumbnailCache {
    max_bytes: usize,
    current_bytes: usize,
    order: VecDeque<String>,
    values: HashMap<String, Vec<u8>>,
}

impl ThumbnailCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            ..Self::default()
        }
    }

    pub fn get(&mut self, key: &str) -> Option<Vec<u8>> {
        let value = self.values.get(key).cloned();
        if value.is_some() {
            self.touch(key);
        }
        value
    }

    pub fn insert(&mut self, key: String, value: Vec<u8>) {
        if let Some(previous) = self.values.remove(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(previous.len());
            self.order.retain(|existing| existing != &key);
        }

        self.current_bytes += value.len();
        self.order.push_back(key.clone());
        self.values.insert(key, value);
        self.evict();
    }

    fn touch(&mut self, key: &str) {
        self.order.retain(|existing| existing != key);
        self.order.push_back(key.to_string());
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
            thumbnail_items: self.values.len() as u32,
            thumbnail_bytes: self.current_bytes as u64,
            thumbnail_budget_bytes: self.max_bytes as u64,
        }
    }

    pub fn clear(&mut self) {
        self.current_bytes = 0;
        self.order.clear();
        self.values.clear();
    }
}
