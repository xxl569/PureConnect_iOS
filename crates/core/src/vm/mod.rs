//! BGI 字节码虚拟机与对白渲染模块

pub mod decoder;
pub mod engine;
pub mod loader;
pub mod opcode;

pub use decoder::{BgiScript, OwnedBgiScript, RawInstruction, BGI_SCRIPT_MAGIC};
pub use engine::{
    apply_protagonist_name, BgMotion, BgPose, CallFrameSave, CharacterSlot, DialogueEvent,
    HistoryEntry, SlotMotion, SlotTransform, SprOverlayState, VmEngine, VmEvent, VmState, VmValue,
    DEFAULT_IS_DEFAULT_NAME, MAX_CALL_DEPTH, MAX_OPS_PER_TICK,
};
pub use loader::{
    build_script_candidates, decode_script_bytes, load_script_from_vfs, ScriptLoader,
    VfsScriptLoader, DSC_MAGIC,
};
pub use opcode::OpCode;
