//! 通信协议与 StringName 字段预缓存模块
//!
//! 避免跨 FFI 每次重复做字符串哈希计算与堆分配，
//! 提供高效的轻量增量快照 (Delta) 字典键支持与差分算法。

use std::collections::HashMap;
use std::sync::OnceLock;

use godot::prelude::*;
use pure_connect_core::vm::CharacterSlot;

pub const KEY_TEXT: &str = "text";
pub const KEY_SPEAKER: &str = "speaker";
pub const KEY_VOICE_STREAM: &str = "voice_stream";
pub const KEY_BG_TEXTURE: &str = "bg_texture";
pub const KEY_CHANGED_SLOTS: &str = "changed_slots";
pub const KEY_CLEARED_SLOTS: &str = "cleared_slots";
pub const KEY_STATE: &str = "state";
pub const KEY_CHOICE_PROMPT: &str = "choice_prompt";
pub const KEY_CHOICE_OPTIONS: &str = "choice_options";
pub const KEY_CHOICE_SEQ: &str = "choice_seq";
pub const KEY_INDEX: &str = "index";
pub const KEY_FACE_CHANGED: &str = "face_changed";
pub const KEY_FACE_TEXTURE: &str = "face_texture";
pub const KEY_VOICE_ASSET: &str = "voice_asset";
pub const KEY_CHARACTER_INTRO: &str = "character_intro";
pub const KEY_BGM_CHANGED: &str = "bgm_changed";
pub const KEY_BGM_NAME: &str = "bgm_name";
pub const KEY_BGM_STREAM: &str = "bgm_stream";
pub const KEY_BG_POSE: &str = "bg_pose";
pub const KEY_BG_MOTIONS: &str = "bg_motions";
pub const KEY_SLOT_TRANSFORMS: &str = "slot_transforms";
pub const KEY_SLOT_MOTIONS: &str = "slot_motions";
pub const KEY_FACE_OFFSET: &str = "face_offset";
pub const KEY_SPR_CHANGED: &str = "spr_changed";
pub const KEY_SPR_TEXTURE: &str = "spr_texture";
pub const KEY_SPR_ASSET: &str = "spr_asset";
pub const KEY_SPR_FADE_MS: &str = "spr_fade_ms";
pub const KEY_SE_STREAMS: &str = "se_streams";
pub const KEY_SE_NAMES: &str = "se_names";
pub const KEY_HIDE_WINDOW_WHEN_SELECTING: &str = "hide_window_when_selecting";
pub const KEY_DISABLED_OPTIONS: &str = "disabled_options";
pub const KEY_MAIL_BTN_STATE: &str = "mail_btn_state";
pub const KEY_MAIL_ON_NOTIFY: &str = "mail_on_notify";
pub const KEY_MAIL_MODE: &str = "mail_mode";
pub const KEY_MAIL_CLOSED: &str = "mail_closed";

/// 预缓存的 GDExtension 通信协议字段名 (StringName)
pub struct ProtocolKeys {
    pub text: StringName,
    pub speaker: StringName,
    pub voice_stream: StringName,
    pub bg_texture: StringName,
    pub changed_slots: StringName,
    pub cleared_slots: StringName,
    pub state: StringName,
    pub choice_prompt: StringName,
    pub choice_options: StringName,
    pub choice_seq: StringName,
    pub index: StringName,
    pub face_changed: StringName,
    pub face_texture: StringName,
    pub voice_asset: StringName,
    pub character_intro: StringName,
    pub bgm_changed: StringName,
    pub bgm_name: StringName,
    pub bgm_stream: StringName,
    pub bg_pose: StringName,
    pub bg_motions: StringName,
    pub slot_transforms: StringName,
    pub slot_motions: StringName,
    pub face_offset: StringName,
    pub spr_changed: StringName,
    pub spr_texture: StringName,
    pub spr_asset: StringName,
    pub spr_fade_ms: StringName,
    pub se_streams: StringName,
    pub se_names: StringName,
    pub hide_window_when_selecting: StringName,
    pub disabled_options: StringName,
    pub mail_btn_state: StringName,
    pub mail_on_notify: StringName,
    pub mail_mode: StringName,
    pub mail_closed: StringName,
}

static PROTOCOL_KEYS: OnceLock<ProtocolKeys> = OnceLock::new();

/// 获取全局预缓存协议字段名引用 (必须在 Godot 运行时主线程中初始化)
pub fn keys() -> &'static ProtocolKeys {
    PROTOCOL_KEYS.get_or_init(|| ProtocolKeys {
        text: StringName::from(KEY_TEXT),
        speaker: StringName::from(KEY_SPEAKER),
        voice_stream: StringName::from(KEY_VOICE_STREAM),
        bg_texture: StringName::from(KEY_BG_TEXTURE),
        changed_slots: StringName::from(KEY_CHANGED_SLOTS),
        cleared_slots: StringName::from(KEY_CLEARED_SLOTS),
        state: StringName::from(KEY_STATE),
        choice_prompt: StringName::from(KEY_CHOICE_PROMPT),
        choice_options: StringName::from(KEY_CHOICE_OPTIONS),
        choice_seq: StringName::from(KEY_CHOICE_SEQ),
        index: StringName::from(KEY_INDEX),
        face_changed: StringName::from(KEY_FACE_CHANGED),
        face_texture: StringName::from(KEY_FACE_TEXTURE),
        voice_asset: StringName::from(KEY_VOICE_ASSET),
        character_intro: StringName::from(KEY_CHARACTER_INTRO),
        bgm_changed: StringName::from(KEY_BGM_CHANGED),
        bgm_name: StringName::from(KEY_BGM_NAME),
        bgm_stream: StringName::from(KEY_BGM_STREAM),
        bg_pose: StringName::from(KEY_BG_POSE),
        bg_motions: StringName::from(KEY_BG_MOTIONS),
        slot_transforms: StringName::from(KEY_SLOT_TRANSFORMS),
        slot_motions: StringName::from(KEY_SLOT_MOTIONS),
        face_offset: StringName::from(KEY_FACE_OFFSET),
        spr_changed: StringName::from(KEY_SPR_CHANGED),
        spr_texture: StringName::from(KEY_SPR_TEXTURE),
        spr_asset: StringName::from(KEY_SPR_ASSET),
        spr_fade_ms: StringName::from(KEY_SPR_FADE_MS),
        se_streams: StringName::from(KEY_SE_STREAMS),
        se_names: StringName::from(KEY_SE_NAMES),
        hide_window_when_selecting: StringName::from(KEY_HIDE_WINDOW_WHEN_SELECTING),
        disabled_options: StringName::from(KEY_DISABLED_OPTIONS),
        mail_btn_state: StringName::from(KEY_MAIL_BTN_STATE),
        mail_on_notify: StringName::from(KEY_MAIL_ON_NOTIFY),
        mail_mode: StringName::from(KEY_MAIL_MODE),
        mail_closed: StringName::from(KEY_MAIL_CLOSED),
    })
}

/// 计算立绘槽位增量 (Delta)
/// 返回: (需要更新渲染的 slot_ids, 被清空的 slot_ids)
pub fn compute_slot_delta(
    last_slots: &HashMap<u32, Vec<String>>,
    current_slots: &HashMap<u32, CharacterSlot>,
) -> (Vec<u32>, Vec<u32>) {
    let mut changed = Vec::new();
    for (slot_id, slot) in current_slots {
        let is_changed = match last_slots.get(slot_id) {
            Some(prev_sprites) => prev_sprites != &slot.sprites,
            None => true,
        };
        if is_changed {
            changed.push(*slot_id);
        }
    }
    changed.sort();

    let mut cleared = Vec::new();
    for slot_id in last_slots.keys() {
        if !current_slots.contains_key(slot_id) {
            cleared.push(*slot_id);
        }
    }
    cleared.sort();

    (changed, cleared)
}
