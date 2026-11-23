use std::collections::{HashSet, VecDeque};

/// 4 级异步预取优先级定义
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrefetchPriority {
    /// Level 1: 关键前台（当前帧台词对白语音、当前立绘、当前背景）
    Critical = 0,
    /// Level 2: 高优先级（下 1~3 句对白语音、表情微差分）
    High = 1,
    /// Level 3: 中优先级（场景预热：下一个 Scene 的背景图与 BGM 音轨）
    Medium = 2,
    /// Level 4: 低优先级（空闲投机：整段剧本可能用到的立绘合集）
    Low = 3,
}

/// 预取任务
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefetchTask {
    pub name: String,
    pub priority: PrefetchPriority,
}

/// 4 级异步预取任务调度队列
pub struct PrefetchQueue {
    critical: VecDeque<String>,
    high: VecDeque<String>,
    medium: VecDeque<String>,
    low: VecDeque<String>,
    pending_set: HashSet<String>,
}

impl PrefetchQueue {
    pub fn new() -> Self {
        Self {
            critical: VecDeque::new(),
            high: VecDeque::new(),
            medium: VecDeque::new(),
            low: VecDeque::new(),
            pending_set: HashSet::new(),
        }
    }

    /// 提交预取任务（自动排重，同一资源不重复排队）
    pub fn push(&mut self, name: impl Into<String>, priority: PrefetchPriority) -> bool {
        let name_str = name.into().to_ascii_lowercase();
        if self.pending_set.contains(&name_str) {
            return false;
        }

        self.pending_set.insert(name_str.clone());
        match priority {
            PrefetchPriority::Critical => self.critical.push_back(name_str),
            PrefetchPriority::High => self.high.push_back(name_str),
            PrefetchPriority::Medium => self.medium.push_back(name_str),
            PrefetchPriority::Low => self.low.push_back(name_str),
        }
        true
    }

    /// 弹出下一个最高优先级的预取任务 (Critical -> High -> Medium -> Low)
    pub fn pop(&mut self) -> Option<PrefetchTask> {
        if let Some(name) = self.critical.pop_front() {
            self.pending_set.remove(&name);
            return Some(PrefetchTask {
                name,
                priority: PrefetchPriority::Critical,
            });
        }
        if let Some(name) = self.high.pop_front() {
            self.pending_set.remove(&name);
            return Some(PrefetchTask {
                name,
                priority: PrefetchPriority::High,
            });
        }
        if let Some(name) = self.medium.pop_front() {
            self.pending_set.remove(&name);
            return Some(PrefetchTask {
                name,
                priority: PrefetchPriority::Medium,
            });
        }
        if let Some(name) = self.low.pop_front() {
            self.pending_set.remove(&name);
            return Some(PrefetchTask {
                name,
                priority: PrefetchPriority::Low,
            });
        }
        None
    }

    /// 当前等待预取的任务总数
    pub fn len(&self) -> usize {
        self.pending_set.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.pending_set.is_empty()
    }

    /// 当玩家快速跳跃或分支选择时，清空非关键预取任务 (保留 Critical)
    pub fn purge_speculative(&mut self) {
        for n in self.high.drain(..) {
            self.pending_set.remove(&n);
        }
        for n in self.medium.drain(..) {
            self.pending_set.remove(&n);
        }
        for n in self.low.drain(..) {
            self.pending_set.remove(&n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefetch_priority_order() {
        let mut queue = PrefetchQueue::new();

        queue.push("bg_forest", PrefetchPriority::Medium);
        queue.push("voice_001", PrefetchPriority::Critical);
        queue.push("chara_face_smile", PrefetchPriority::High);
        queue.push("chara_all_poses", PrefetchPriority::Low);

        // 重复提交应被过滤
        assert!(!queue.push("voice_001", PrefetchPriority::Critical));

        // 必须按优先级严格出队：Critical -> High -> Medium -> Low
        let t1 = queue.pop().unwrap();
        assert_eq!(t1.name, "voice_001");
        assert_eq!(t1.priority, PrefetchPriority::Critical);

        let t2 = queue.pop().unwrap();
        assert_eq!(t2.name, "chara_face_smile");
        assert_eq!(t2.priority, PrefetchPriority::High);

        let t3 = queue.pop().unwrap();
        assert_eq!(t3.name, "bg_forest");
        assert_eq!(t3.priority, PrefetchPriority::Medium);

        let t4 = queue.pop().unwrap();
        assert_eq!(t4.name, "chara_all_poses");
        assert_eq!(t4.priority, PrefetchPriority::Low);

        assert!(queue.pop().is_none());
    }
}
