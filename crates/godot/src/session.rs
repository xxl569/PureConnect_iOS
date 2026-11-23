//! 剧本执行会话管理模块
//!
//! 基于 `OwnedBgiScript` 与 `ScriptLoader` 封装 BGI 虚拟机生命周期，
//! 零 `unsafe`，原生支持跨脚本调用栈 (`0x00F0` / `0x00F3` / `0x001B`) 与廉价克隆预瞻。

use std::sync::Arc;

use pure_connect_core::vm::{DialogueEvent, OwnedBgiScript, ScriptLoader, VmEngine, VmState};
use pure_connect_core::{CoreError, Result};

pub struct ScriptSession {
    engine: VmEngine<'static>,
}

impl ScriptSession {
    /// 从剧本原始字节数据创建会话（不绑定跨脚本加载器）
    pub fn new(data: Vec<u8>) -> Result<Self> {
        Self::with_loader(Arc::new(data), None)
    }

    /// 从剧本原始字节数据创建会话，并可选注入跨脚本统一加载器
    pub fn with_loader(
        data: Arc<Vec<u8>>,
        loader: Option<Arc<dyn ScriptLoader>>,
    ) -> Result<Self> {
        if data.is_empty() {
            return Err(CoreError::OutOfBounds);
        }

        let script = OwnedBgiScript::parse(data)?;
        let mut engine = VmEngine::from_owned(script);
        if let Some(l) = loader {
            engine.set_script_loader(l);
        }

        Ok(Self { engine })
    }

    #[inline]
    pub fn state(&self) -> &VmState {
        self.engine.state()
    }

    #[inline]
    pub fn next_dialogue(&mut self) -> Result<Option<DialogueEvent>> {
        self.engine.next_dialogue()
    }

    #[inline]
    pub fn submit_choice(&mut self, choice_seq: u64, index: usize) -> bool {
        self.engine.submit_choice(choice_seq, index)
    }

    #[inline]
    pub fn select_choice(&mut self, index: usize) {
        self.engine.select_choice(index);
    }

    #[inline]
    pub fn set_script_name(&mut self, name: &str) {
        self.engine.set_script_name(name);
    }

    #[inline]
    pub fn engine(&self) -> &VmEngine<'static> {
        &self.engine
    }

    #[inline]
    pub fn engine_mut(&mut self) -> &mut VmEngine<'static> {
        &mut self.engine
    }

    /// 无副作用预瞻接下来若干句对白事件（用于后台线程预解码即将登场的背景/立绘/特写资产）
    pub fn peek_upcoming(&self, count: usize) -> Vec<DialogueEvent> {
        let mut out = Vec::with_capacity(count);
        let mut cloned = self.engine.clone();
        for _ in 0..count {
            match cloned.next_dialogue() {
                Ok(Some(dlg)) => out.push(dlg),
                _ => break,
            }
        }
        out
    }
}
