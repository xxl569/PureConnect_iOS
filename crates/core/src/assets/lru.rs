use std::collections::HashMap;

/// 缓存条目元数据
struct CacheEntry<V> {
    value: V,
    byte_size: usize,
    last_access: u64,
}

/// 限制严格内存上限的 LRU 缓存 (防 iOS TrollStore OOM 闪退)
pub struct LruCache<K, V> {
    entries: HashMap<K, CacheEntry<V>>,
    max_bytes: usize,
    current_bytes: usize,
    access_counter: u64,
    hits: u64,
    misses: u64,
}

impl<K: std::hash::Hash + Eq + Clone, V> LruCache<K, V> {
    /// 创建指定最大字节数限制的 LRU 缓存（默认推荐 384MB ~ 512MB）
    pub fn new(max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_bytes,
            current_bytes: 0,
            access_counter: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// 获取缓存条目，若命中则刷新其 LRU 热度
    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.access_counter += 1;
        let counter = self.access_counter;
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_access = counter;
            self.hits += 1;
            Some(&entry.value)
        } else {
            self.misses += 1;
            None
        }
    }

    /// 插入新条目并自动触发内存配额超额驱逐
    pub fn insert(&mut self, key: K, value: V, byte_size: usize) {
        self.access_counter += 1;
        let counter = self.access_counter;

        // 如果键已存在，先减去旧占用
        if let Some(old) = self.entries.remove(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(old.byte_size);
        }

        // 内存若不足，淘汰最旧的条目
        while self.current_bytes + byte_size > self.max_bytes && !self.entries.is_empty() {
            self.evict_lru();
        }

        self.current_bytes += byte_size;
        self.entries.insert(
            key,
            CacheEntry {
                value,
                byte_size,
                last_access: counter,
            },
        );
    }

    /// 驱逐单个最久未被访问的条目
    fn evict_lru(&mut self) {
        let mut oldest_key: Option<K> = None;
        let mut oldest_time = u64::MAX;

        for (k, entry) in &self.entries {
            if entry.last_access < oldest_time {
                oldest_time = entry.last_access;
                oldest_key = Some(k.clone());
            }
        }

        if let Some(k) = oldest_key {
            if let Some(removed) = self.entries.remove(&k) {
                self.current_bytes = self.current_bytes.saturating_sub(removed.byte_size);
            }
        }
    }

    /// 当前占用字节数
    pub fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    /// 最大容量字节数
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// 当前条目数量
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 缓存是否为空
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 清空全部缓存
    pub fn clear(&mut self) {
        self.entries.clear();
        self.current_bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lru_eviction() {
        // 限额 100 字节
        let mut lru = LruCache::new(100);

        lru.insert("item1", vec![1; 40], 40);
        lru.insert("item2", vec![2; 40], 40);
        assert_eq!(lru.len(), 2);
        assert_eq!(lru.current_bytes(), 80);

        // 访问 item1，使其比 item2 更新
        assert!(lru.get(&"item1").is_some());

        // 插入 item3 (40 字节)，总计 120 > 100，item2 应该被驱逐！
        lru.insert("item3", vec![3; 40], 40);

        assert!(lru.get(&"item1").is_some(), "item1 应该依然在缓存中");
        assert!(lru.get(&"item2").is_none(), "item2 最久未用，应被淘汰");
        assert!(lru.get(&"item3").is_some(), "item3 应在缓存中");
        assert_eq!(lru.current_bytes(), 80);
    }
}
