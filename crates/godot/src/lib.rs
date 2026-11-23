//! PURE×CONNECT Godot 4.3 GDExtension 绑定层核心入口
//!
//! 采用路线 B 架构：Rust 核心状态机 + Godot 表现层渲染。
//! 遵循 Panic 防火墙、单向数据流与主线程工厂规范。

pub mod engine;
pub mod factory;
pub mod protocol;
pub mod runtime;
pub mod session;

#[cfg(test)]
mod tests;

use godot::prelude::*;

pub use engine::PureConnectEngine;
pub use factory::MainThreadFactory;
pub use protocol::{keys, ProtocolKeys};
pub use runtime::{
    BgUpdate, DialogueSnapshot, PendingChoiceSave, PureConnectRuntime, SaveStateV2, StepResult,
    SystemFlagsData,
};
pub use session::ScriptSession;

struct PureConnectExtension;

#[gdextension]
unsafe impl ExtensionLibrary for PureConnectExtension {}
