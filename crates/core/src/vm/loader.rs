//! 跨脚本统一加载管线（汉化优先级 + DSC 透明解压）
//!
//! 供 Rust 集成测试与 Godot 运行时共用唯一脚本解析逻辑：
//! 1. 先查高优先级挂载源（如 `exe_unpacked` 明文汉化脚本），绝不让 `.arc` 原版覆盖汉化；
//! 2. 未命中再查 `.arc` 封包；
//! 3. 透明检测 `DSC FORMAT 1.00\0` 魔数并自动 `Dsc::decode` 解压为明文 BGI 字节码。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::codec::Dsc;
use crate::error::Result;
use crate::vfs::VirtualFileSystem;

/// DSC 压缩脚本标准 16 字节魔数
pub const DSC_MAGIC: &[u8; 16] = b"DSC FORMAT 1.00\0";

/// 跨脚本加载抽象（供 `VmEngine` 在 `0x00F0` / `0x00F3` / 读档恢复调用栈时加载目标脚本）
pub trait ScriptLoader: Send + Sync {
    /// 按脚本名加载明文 BGI 编译脚本字节流（若底层为 DSC 压缩包已自动解压）
    fn load_script(&self, name: &str) -> Result<Option<Arc<Vec<u8>>>>;
}

/// 生成标准 BGI 脚本候选文件名列表（小写去重）
pub fn build_script_candidates(name: &str) -> Vec<String> {
    let clean = name.trim().to_ascii_lowercase();
    if clean.is_empty() {
        return Vec::new();
    }
    let base = clean.trim_start_matches('_');
    let mut candidates = Vec::with_capacity(12);
    candidates.push(clean.clone());
    if !clean.starts_with('_') {
        candidates.push(format!("_{}", clean));
    } else {
        candidates.push(base.to_string());
    }
    candidates.push(format!("{}.bs5", clean));
    candidates.push(format!("{}.bs5", base));
    candidates.push(format!("{}.bin", clean));
    candidates.push(format!("_bs/{}", clean));
    candidates.push(format!("_bs/{}", base));
    candidates.push(format!("_bs/{}.bs5", clean));
    candidates.push(format!("_bs/{}.bs5", base));
    candidates.dedup();
    candidates
}

/// 若数据带有 `DSC FORMAT 1.00\0` 魔数则透明解压，否则原样返回
pub fn decode_script_bytes(raw: Vec<u8>) -> Result<Vec<u8>> {
    if raw.len() >= DSC_MAGIC.len() && &raw[..DSC_MAGIC.len()] == DSC_MAGIC {
        Dsc::decode(&raw)
    } else {
        Ok(raw)
    }
}

/// 从 VFS 按“先高优先级散落目录（汉化）→ 后 .arc 封包 → 透明 DSC 解压”唯一管线加载脚本
pub fn load_script_from_vfs(vfs: &VirtualFileSystem, name: &str) -> Result<Option<Vec<u8>>> {
    let candidates = build_script_candidates(name);
    if candidates.is_empty() {
        return Ok(None);
    }
    let Some(cow) = vfs.read_first_matching(&candidates) else {
        return Ok(None);
    };
    let decoded = decode_script_bytes(cow.into_owned())?;
    Ok(Some(decoded))
}

/// 基于 `VirtualFileSystem` 的带缓存标准脚本加载器
pub struct VfsScriptLoader {
    vfs: Arc<VirtualFileSystem>,
    cache: RwLock<HashMap<String, Option<Arc<Vec<u8>>>>>,
}

impl VfsScriptLoader {
    pub fn new(vfs: Arc<VirtualFileSystem>) -> Self {
        Self {
            vfs,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn vfs(&self) -> &Arc<VirtualFileSystem> {
        &self.vfs
    }
}

impl ScriptLoader for VfsScriptLoader {
    fn load_script(&self, name: &str) -> Result<Option<Arc<Vec<u8>>>> {
        let key = name.trim().to_ascii_lowercase();
        if key.is_empty() {
            return Ok(None);
        }
        if let Ok(guard) = self.cache.read() {
            if let Some(hit) = guard.get(&key) {
                return Ok(hit.clone());
            }
        }
        let loaded = load_script_from_vfs(&self.vfs, &key)?.map(Arc::new);
        if let Ok(mut guard) = self.cache.write() {
            guard.insert(key, loaded.clone());
        }
        Ok(loaded)
    }
}
