//! 纯 Rust 运行时桥接层
//!
//! 负责 VFS 挂载、剧本驱动、增量 Delta 快照计算与资产解码。
//! 严格解耦于 Godot C ABI，保障在单元/集成测试与无 GUI 环境下 100% 可测试。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use serde::{Deserialize, Serialize};

use pure_connect_core::assets::LruCache;
use pure_connect_core::codec::{
    AnimationDefTable, AnimationSetTable, BwAudio, CompressedBg, DecodedImage, Dsc,
    SpriteCompositor,
};
use pure_connect_core::error::{CoreError, Result};
use pure_connect_core::vfs::VirtualFileSystem;
use pure_connect_core::vm::{
    BgMotion, BgPose, CallFrameSave, CharacterSlot, DialogueEvent, ScriptLoader, SlotMotion,
    SlotTransform, SprOverlayState, VfsScriptLoader, VmEngine, VmState,
};

use crate::protocol::compute_slot_delta;
use crate::session::ScriptSession;

pub const DEFAULT_IMAGE_CACHE_BYTES: usize = 256 * 1024 * 1024; // 256 MB (防 iOS Jetsam OOM)
pub const DEFAULT_AUDIO_CACHE_BYTES: usize = 64 * 1024 * 1024; // 64 MB

#[derive(Debug, Clone)]
pub enum BgUpdate {
    Unchanged,
    Cleared,
    New(Arc<DecodedImage>),
}

/// 独立 SPR 覆盖层增量更新（如主线/共同线手机贴图 slot 0）
#[derive(Debug, Clone)]
pub enum SprUpdate {
    Unchanged,
    Cleared {
        fade_ms: u32,
    },
    New {
        asset: String,
        image: Arc<DecodedImage>,
        fade_ms: u32,
    },
}

/// _Wait(-1, 1) 点击翻页挂起快照（零污染 dialogue_index 与 Backlog）
#[derive(Debug, Clone)]
pub struct WaitClickSnapshot {
    pub spr_update: SprUpdate,
    pub se_plays: Vec<(String, Arc<Vec<u8>>)>,
    pub hide_window_when_selecting: bool,
    pub mail_btn_state: i32,
    pub mail_mode: bool,
}

#[derive(Debug, Clone)]
pub struct CharacterIntroSnapshot {
    pub character: String,
    pub jingle_data: Option<Arc<Vec<u8>>>,
    pub se_data: Option<Arc<Vec<u8>>>,
    pub card_image: Option<Arc<DecodedImage>>,
    /// 5 层原生 CG 登场图层 (ec_{code}_01..05)
    pub layers: [Option<Arc<DecodedImage>>; 5],
}

#[derive(Debug, Clone)]
pub struct DialogueSnapshot {
    pub index: usize,
    pub text: String,
    pub speaker: Option<String>,
    pub voice_data: Option<Arc<Vec<u8>>>,
    pub bg_update: BgUpdate,
    pub bg_key: Option<String>,
    pub bg_pose: Option<BgPose>,
    pub bg_motions: Vec<BgMotion>,
    pub bgm_changed: bool,
    pub bgm_name: Option<String>,
    pub bgm_data: Option<Arc<Vec<u8>>>,
    pub changed_slots: HashMap<u32, Arc<DecodedImage>>,
    pub slot_keys: HashMap<u32, String>,
    pub cleared_slots: Vec<u32>,
    pub slot_transforms: HashMap<u32, SlotTransform>,
    pub slot_motions: Vec<SlotMotion>,
    pub face_changed: bool,
    pub face_texture: Option<Arc<DecodedImage>>,
    pub face_key: Option<String>,
    pub face_offset: (i32, i32),
    pub voice_asset: Option<String>,
    pub character_intro: Option<CharacterIntroSnapshot>,
    pub spr_update: SprUpdate,
    pub se_plays: Vec<(String, Arc<Vec<u8>>)>,
    pub hide_window_when_selecting: bool,
    pub mail_btn_state: i32,
    pub mail_on_notify: bool,
}

#[derive(Debug, Clone)]
pub enum StepResult {
    Dialogue(DialogueSnapshot),
    WaitClick(WaitClickSnapshot),
    Choice {
        prompt: Option<String>,
        options: Vec<String>,
        disabled_options: Vec<usize>,
        spr_update: SprUpdate,
        se_plays: Vec<(String, Arc<Vec<u8>>)>,
        hide_window_when_selecting: bool,
        mail_btn_state: i32,
        mail_mode: bool,
    },
    /// 共同线独立 mail_vm (window_Mail) 正常或取消退出，已无条件回写 flags[200..=201] 与 flags[300..=419]
    MailClosed {
        mail_btn_state: i32,
        spr_fade_ms: u32,
        se_plays: Vec<(String, Arc<Vec<u8>>)>,
    },
    Terminated,
    Ready,
    Unloaded,
    Error(String),
}

fn default_save_version_v1() -> u32 {
    1
}

fn default_sys_flags_version() -> u32 {
    1
}

fn default_sprite_y() -> i32 {
    200
}

fn default_sprite_scale() -> i32 {
    100
}

/// 持久化舞台立绘槽位元数据（仅存轻量元数据，不含像素）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSpriteSlot {
    #[serde(default)]
    pub slot_id: u32,
    #[serde(default)]
    pub sprites: Vec<String>,
    #[serde(default)]
    pub x: i32,
    #[serde(default = "default_sprite_y")]
    pub y: i32,
    #[serde(default = "default_sprite_scale")]
    pub scale: i32,
}

/// 选项界面存档挂起态快照
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingChoiceSave {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub disabled_options: Vec<usize>,
}

/// 存档 Schema v2（向后兼容 v1：所有 v2 新增字段均带 #[serde(default)]）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveStateV2 {
    #[serde(default = "default_save_version_v1")]
    pub save_version: u32,
    #[serde(default)]
    pub script_name: String,
    #[serde(default)]
    pub dialogue_index: usize,
    #[serde(default)]
    pub choices: Vec<usize>,
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub entry_flags: BTreeMap<u32, i32>,
    #[serde(default)]
    pub flags: BTreeMap<u32, i32>,
    #[serde(default)]
    pub pending_choice: Option<PendingChoiceSave>,
    #[serde(default)]
    pub choice_seq: u64,
    #[serde(default)]
    pub suspended_pc: Option<usize>,
    #[serde(default)]
    pub call_stack: Vec<CallFrameSave>,
    #[serde(default)]
    pub script_dialogue_index: Option<usize>,
    /// 当前活动脚本局部选择历史（0x00F0 压栈清空、0x001B 跨脚本返回归并回父帧、0x00F3 清空）
    #[serde(default)]
    pub script_choices: Vec<usize>,
    /// 舞台视觉底色快照（根治 0x00F0 子脚本与 WaitingForChoice 读档丢背景/BGM/立绘/头像）
    #[serde(default)]
    pub active_bg: Option<String>,
    #[serde(default)]
    pub active_bgm: Option<String>,
    #[serde(default)]
    pub active_sprites: Vec<SavedSpriteSlot>,
    #[serde(default)]
    pub active_face: Option<String>,
    #[serde(default)]
    pub active_face_sprites: Vec<String>,
    /// 手机短信在屏期间（男主吐槽对白或 _SelectEx 选项）存档时记录当前 slot 0 贴图名，读档自动补绘
    #[serde(default)]
    pub active_mail_spr: Option<String>,
    /// 0x0167 (HideWindowWhenItemSelecting) 状态持久化，确保手机选项读档后右侧避让排版不丢失
    #[serde(default)]
    pub hide_window_when_selecting: bool,
}

/// 全局系统标志持久化结构 (user://system_flags.json)，独立于单槽位存档
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemFlagsData {
    #[serde(default = "default_sys_flags_version")]
    pub version: u32,
    #[serde(default)]
    pub unlocked_cgs: BTreeSet<u32>,
    #[serde(default)]
    pub sys_regs: BTreeMap<u32, i32>,
}

#[derive(Clone)]
enum PrefetchTask {
    Image(String),
    Composite(Vec<String>),
    Audio(String),
}

struct PrefetchState {
    tasks: Mutex<Vec<PrefetchTask>>,
    cvar: Condvar,
    stop: AtomicBool,
}

#[derive(Clone)]
struct ResolverCtx {
    vfs: Arc<VirtualFileSystem>,
    image_cache: Arc<Mutex<LruCache<String, Arc<DecodedImage>>>>,
    audio_cache: Arc<Mutex<LruCache<String, Arc<Vec<u8>>>>>,
    animation_sets: Arc<HashMap<String, Vec<String>>>,
    animation_aliases: Arc<HashMap<String, String>>,
}

pub struct PureConnectRuntime {
    vfs: Option<Arc<VirtualFileSystem>>,
    script_loader: Option<Arc<VfsScriptLoader>>,
    session: Option<ScriptSession>,
    image_cache: Arc<Mutex<LruCache<String, Arc<DecodedImage>>>>,
    audio_cache: Arc<Mutex<LruCache<String, Arc<Vec<u8>>>>>,
    animation_sets: Arc<HashMap<String, Vec<String>>>,
    animation_aliases: Arc<HashMap<String, String>>,
    prefetch_state: Option<Arc<PrefetchState>>,
    prefetch_worker: Option<JoinHandle<()>>,
    current_script: Option<String>,
    choice_history: Vec<usize>,
    /// 运行期会话级档内标志 (0..=419)：跨脚本保留，仅从头开始清空、读档替换
    session_flags: BTreeMap<u32, i32>,
    /// 当前剧本加载入口时的标志快照（供 v2 存档快进重建精确分支轨迹）
    entry_flags: BTreeMap<u32, i32>,
    /// 系统只读/运行寄存器 (0x00E5 / 0x00E7)
    sys_regs: BTreeMap<u32, i32>,
    /// 玩家配置寄存器 (专门供 0x00E3 查询)
    config_regs: BTreeMap<u32, i32>,
    /// 全局 CG 解锁集合 (0x00E6)，不被旧档读入覆盖
    unlocked_cgs: BTreeSet<u32>,
    system_flags_dirty: bool,
    /// 单调递增选项序列号基准
    choice_seq_base: u64,
    last_dialogue_index: usize,
    last_speaker: Option<String>,
    last_text: String,
    last_rendered_bg: Option<String>,
    last_rendered_bgm: Option<String>,
    last_rendered_slots: HashMap<u32, Vec<String>>,
    last_rendered_face: Option<String>,
    last_rendered_face_sprites: Vec<String>,
    last_rendered_spr: Option<String>,
    /// 方案 4A：共同线彩蛋信封独立轻量临时子 VmEngine（window_Mail），活跃期间独占路由、零污染主引擎
    mail_session: Option<ScriptSession>,
    /// 独立追踪 mail_vm 的手机贴图增量（不污染主引擎 last_rendered_spr）
    mail_last_rendered_spr: Option<String>,
}

impl Default for PureConnectRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PureConnectRuntime {
    fn drop(&mut self) {
        self.stop_prefetch_worker();
    }
}

impl PureConnectRuntime {
    pub fn new() -> Self {
        Self {
            vfs: None,
            script_loader: None,
            session: None,
            image_cache: Arc::new(Mutex::new(LruCache::new(DEFAULT_IMAGE_CACHE_BYTES))),
            audio_cache: Arc::new(Mutex::new(LruCache::new(DEFAULT_AUDIO_CACHE_BYTES))),
            animation_sets: Arc::new(HashMap::new()),
            animation_aliases: Arc::new(HashMap::new()),
            prefetch_state: None,
            prefetch_worker: None,
            current_script: None,
            choice_history: Vec::new(),
            session_flags: BTreeMap::new(),
            entry_flags: BTreeMap::new(),
            sys_regs: VmEngine::default_sys_regs(),
            config_regs: VmEngine::default_config_regs(),
            unlocked_cgs: BTreeSet::new(),
            system_flags_dirty: false,
            choice_seq_base: 0,
            last_dialogue_index: 0,
            last_speaker: None,
            last_text: String::new(),
            last_rendered_bg: None,
            last_rendered_bgm: None,
            last_rendered_slots: HashMap::new(),
            last_rendered_face: None,
            last_rendered_face_sprites: Vec::new(),
            last_rendered_spr: None,
            mail_session: None,
            mail_last_rendered_spr: None,
        }
    }

    fn stop_prefetch_worker(&mut self) {
        if let Some(state) = self.prefetch_state.take() {
            state.stop.store(true, Ordering::Release);
            state.cvar.notify_all();
        }
        if let Some(handle) = self.prefetch_worker.take() {
            let _ = handle.join();
        }
    }

    fn start_prefetch_worker(&mut self) {
        self.stop_prefetch_worker();
        let Some(ctx) = self.resolver_ctx() else {
            return;
        };
        let state = Arc::new(PrefetchState {
            tasks: Mutex::new(Vec::new()),
            cvar: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker_state = Arc::clone(&state);
        let handle = thread::Builder::new()
            .name("pc-prefetch".to_string())
            .spawn(move || {
                loop {
                    let batch = {
                        let mut guard = match worker_state.tasks.lock() {
                            Ok(g) => g,
                            Err(_) => break,
                        };
                        while guard.is_empty() && !worker_state.stop.load(Ordering::Acquire) {
                            guard = match worker_state.cvar.wait(guard) {
                                Ok(g) => g,
                                Err(_) => return,
                            };
                        }
                        if worker_state.stop.load(Ordering::Acquire) {
                            break;
                        }
                        std::mem::take(&mut *guard)
                    };

                    for task in batch {
                        if worker_state.stop.load(Ordering::Acquire) {
                            return;
                        }
                        match task {
                            PrefetchTask::Image(name) => {
                                let _ = ctx.resolve_image_arc(&name);
                            }
                            PrefetchTask::Composite(sprites) => {
                                let _ = ctx.composite_slot_sprites_arc(&sprites);
                            }
                            PrefetchTask::Audio(name) => {
                                let _ = ctx.resolve_audio_arc(&name);
                            }
                        }
                    }
                }
            })
            .ok();
        if let Some(h) = handle {
            self.prefetch_state = Some(state);
            self.prefetch_worker = Some(h);
        }
    }

    fn resolver_ctx(&self) -> Option<ResolverCtx> {
        let vfs = self.vfs.as_ref()?.clone();
        Some(ResolverCtx {
            vfs,
            image_cache: Arc::clone(&self.image_cache),
            audio_cache: Arc::clone(&self.audio_cache),
            animation_sets: Arc::clone(&self.animation_sets),
            animation_aliases: Arc::clone(&self.animation_aliases),
        })
    }

    fn schedule_lookahead(&self) {
        let Some(ref state) = self.prefetch_state else {
            return;
        };
        let Some(ref session) = self.session else {
            return;
        };
        let upcoming = session.peek_upcoming(3);
        if upcoming.is_empty() {
            return;
        }
        let mut tasks = Vec::new();
        for dlg in upcoming {
            if let Some(ref bg) = dlg.bg {
                if self.last_rendered_bg.as_deref() != Some(bg.as_str()) {
                    tasks.push(PrefetchTask::Image(bg.clone()));
                }
            }
            if let Some(ref v) = dlg.voice {
                tasks.push(PrefetchTask::Audio(v.clone()));
            }
            if let Some(ref b) = dlg.bgm {
                if self.last_rendered_bgm.as_deref() != Some(b.as_str()) {
                    tasks.push(PrefetchTask::Audio(b.clone()));
                }
            }
            for slot in dlg.slots.values() {
                if !slot.sprites.is_empty() {
                    tasks.push(PrefetchTask::Composite(slot.sprites.clone()));
                }
            }
            if !dlg.face_sprites.is_empty() {
                tasks.push(PrefetchTask::Composite(dlg.face_sprites.clone()));
            } else if let Some(ref f) = dlg.face {
                tasks.push(PrefetchTask::Image(f.clone()));
            }
            if let Some(ref char_name) = dlg.character_intro {
                let prefix = match char_name.as_str() {
                    "shiho" => "ec_sh_",
                    "arisa" => "ec_ar_",
                    "sora" => "ec_so_",
                    "moemi" => "ec_mo_",
                    "ayumi" => "ec_ay_",
                    _ => "",
                };
                if !prefix.is_empty() {
                    for idx in 1..=5 {
                        tasks.push(PrefetchTask::Image(format!("{prefix}0{idx}")));
                    }
                }
            }
        }
        if !tasks.is_empty() {
            if let Ok(mut guard) = state.tasks.lock() {
                *guard = tasks;
                state.cvar.notify_one();
            }
        }
    }

    /// 初始化虚拟文件系统 (VFS)
    pub fn init_vfs(&mut self, base_dir: &str, patch_dir: &str) -> Result<bool> {
        let mut vfs = VirtualFileSystem::new();
        let mut mounted_count = 0;
        let mut mounted_arcs: HashSet<PathBuf> = HashSet::new();

        let patch_trimmed = patch_dir.trim();
        if !patch_trimmed.is_empty() {
            let p = Path::new(patch_trimmed);
            if p.is_dir() {
                vfs.mount_directory(p)?;
                mounted_count += 1;
            } else if p.is_file() {
                vfs.mount_archive(p)?;
                mounted_arcs.insert(p.to_path_buf());
                mounted_count += 1;
            }
        }

        let base_trimmed = base_dir.trim();
        if !base_trimmed.is_empty() {
            let b = Path::new(base_trimmed);
            if b.is_file() {
                vfs.mount_archive(b)?;
                mounted_arcs.insert(b.to_path_buf());
                mounted_count += 1;
            } else if b.is_dir() {
                // 优先将 base_dir 自身挂载为散落文件源 (补丁/散落资产优先于封包内部)
                if vfs.mount_directory(b).is_ok() {
                    mounted_count += 1;
                }

                // 递归检索 .arc 封包 (如 Archive/data01000..06050.arc)
                let mut arc_paths = Vec::new();
                Self::collect_arc_files(b, &mut arc_paths);
                arc_paths.sort();

                for arc_path in arc_paths {
                    if vfs.mount_archive(&arc_path).is_ok() {
                        mounted_arcs.insert(arc_path);
                        mounted_count += 1;
                    }
                }

                // 补充挂载父级根目录下的 sysgrp.arc / syssnd.arc / sysprg.arc（低优先级，不覆盖汉化散落补丁）
                if let Some(parent_dir) = b.parent() {
                    if let Ok(entries) = fs::read_dir(parent_dir) {
                        let mut parent_arcs = Vec::new();
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_file() && !mounted_arcs.contains(&path) {
                                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                                    if ext.eq_ignore_ascii_case("arc") {
                                        parent_arcs.push(path);
                                    }
                                }
                            }
                        }
                        parent_arcs.sort();
                        for arc_path in parent_arcs {
                            if vfs.mount_archive(&arc_path).is_ok() {
                                mounted_count += 1;
                            }
                        }
                    }
                }
            }
        }

        if mounted_count == 0 {
            return Ok(false);
        }

        self.stop_prefetch_worker();
        let vfs_arc = Arc::new(vfs);
        self.script_loader = Some(Arc::new(VfsScriptLoader::new(Arc::clone(&vfs_arc))));
        self.vfs = Some(vfs_arc);
        self.session = None;
        self.current_script = None;
        self.choice_history.clear();
        self.session_flags.clear();
        self.entry_flags.clear();
        self.last_dialogue_index = 0;
        self.last_speaker = None;
        self.last_text.clear();
        self.last_rendered_bg = None;
        self.last_rendered_bgm = None;
        self.last_rendered_slots.clear();
        self.last_rendered_face = None;
        self.last_rendered_face_sprites.clear();
        self.last_rendered_spr = None;
        if let Ok(mut c) = self.image_cache.lock() {
            c.clear();
        }
        if let Ok(mut c) = self.audio_cache.lock() {
            c.clear();
        }
        self.load_animation_tables();
        self.start_prefetch_worker();

        Ok(true)
    }

    /// 从 VFS 加载 BGI `animationset*` 表情宏展开表与 `animation*` 动画别名表
    fn load_animation_tables(&mut self) {
        let mut sets = HashMap::new();
        let mut aliases = HashMap::new();
        if let Some(vfs) = self.vfs.as_ref() {
            for suffix in ["000", "010", "020", "030", "040", "050"] {
                let set_name = format!("animationset{}", suffix);
                if let Ok(raw) = vfs.read(&set_name) {
                    if let Ok(dec) = Dsc::decode(&raw) {
                        let _ = AnimationSetTable::parse_into(&dec, &mut sets);
                    }
                }
                let anim_name = format!("animation{}", suffix);
                if let Ok(raw) = vfs.read(&anim_name) {
                    if let Ok(dec) = Dsc::decode(&raw) {
                        let _ = AnimationDefTable::parse_into(&dec, &mut aliases);
                    }
                }
            }
        }
        self.animation_sets = Arc::new(sets);
        self.animation_aliases = Arc::new(aliases);
    }

    fn collect_arc_files(dir: &Path, list: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::collect_arc_files(&path, list);
                } else if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                        if ext.eq_ignore_ascii_case("arc") {
                            list.push(path);
                        }
                    }
                }
            }
        }
    }

    /// 全新加载剧本（用于新游戏 / 读档重建）：清空舞台视觉缓存与调用栈，走统一 VfsScriptLoader 管线
    /// （先查高优先级 exe_unpacked 汉化明文 → 后查 .arc → 透明 DSC 解压）。
    /// 注意：剧本内部 0x00F0 / 0x00F3 / 0x001B 跨脚本切换在 VmEngine 内无缝执行，严禁经由本入口清场。
    pub fn load_script_fresh(&mut self, name: &str) -> Result<bool> {
        self.mail_session = None;
        self.mail_last_rendered_spr = None;
        self.session = None;
        self.current_script = None;
        self.choice_history.clear();
        self.last_dialogue_index = 0;
        self.last_speaker = None;
        self.last_text.clear();
        self.last_rendered_bg = None;
        self.last_rendered_bgm = None;
        self.last_rendered_slots.clear();
        self.last_rendered_face = None;
        self.last_rendered_face_sprites.clear();
        self.last_rendered_spr = None;

        let loader = self.script_loader.as_ref().ok_or_else(|| {
            CoreError::InvalidFormat("VFS 虚拟文件系统尚未初始化，请先调用 init_vfs")
        })?;

        let script_bytes = match loader.load_script(name)? {
            Some(b) => b,
            None => return Ok(false),
        };

        let loader_dyn: Arc<dyn ScriptLoader> = Arc::clone(loader) as Arc<dyn ScriptLoader>;
        let mut session = ScriptSession::with_loader(script_bytes, Some(loader_dyn))?;
        let clean_name = name.trim();
        self.entry_flags = self.session_flags.clone();
        {
            let eng = session.engine_mut();
            eng.set_script_name(clean_name);
            eng.set_flags(self.session_flags.clone());
            eng.set_entry_flags(self.entry_flags.clone());
            eng.set_sys_regs(self.sys_regs.clone());
            eng.set_config_regs(self.config_regs.clone());
            eng.set_unlocked_cgs(self.unlocked_cgs.clone());
            eng.set_choice_seq(self.choice_seq_base);
        }
        self.session = Some(session);
        self.current_script = Some(clean_name.to_string());
        self.schedule_lookahead();
        Ok(true)
    }

    /// 从 VFS 加载剧本（外部新开/切章入口，委派至 load_script_fresh）
    pub fn load_script(&mut self, name: &str) -> Result<bool> {
        self.load_script_fresh(name)
    }

    /// 同步当前运行期会话的脚本名 / flags / sys_regs / unlocked_cgs / choice_seq 回运行时缓存
    fn sync_from_session(&mut self) {
        if let Some(ref mut session) = self.session {
            let eng = session.engine_mut();
            let active_name = eng.script_name();
            if !active_name.is_empty() && active_name != "<unknown>" {
                self.current_script = Some(active_name.to_string());
            }
            self.session_flags = eng.flags().clone();
            self.entry_flags = eng.entry_flags().clone();
            self.sys_regs = eng.sys_regs().clone();
            self.config_regs = eng.config_regs().clone();
            for &cg in eng.unlocked_cgs() {
                if self.unlocked_cgs.insert(cg) {
                    self.system_flags_dirty = true;
                }
            }
            if eng.take_unlocked_cgs_dirty() || eng.take_sys_regs_dirty() {
                self.system_flags_dirty = true;
            }
            self.choice_seq_base = self.choice_seq_base.max(eng.choice_seq());
        }
    }

    /// 清空单周目档内变量表（仅在“从头开始”时调用；切换下一脚本时保留，全局 CG 解锁不清空）
    pub fn reset_session_flags(&mut self) {
        self.mail_session = None;
        self.mail_last_rendered_spr = None;
        self.session_flags.clear();
        self.entry_flags.clear();
        if let Some(ref mut session) = self.session {
            session.engine_mut().set_flags(BTreeMap::new());
        }
    }

    /// 读取运行期会话标志
    pub fn get_flag(&self, id: u32) -> i32 {
        self.session
            .as_ref()
            .map(|s| s.engine().get_flag(id))
            .unwrap_or_else(|| self.session_flags.get(&id).copied().unwrap_or(0))
    }

    /// 设置运行期会话标志
    pub fn set_flag(&mut self, id: u32, value: i32) {
        self.session_flags.insert(id, value);
        if let Some(ref mut session) = self.session {
            session.engine_mut().set_flag(id, value);
        }
    }

    /// 外部配置同步系统寄存器（供 0x00E5 / 0x00E7 查询，绝不混用 0x00E3 config_regs）
    pub fn set_sys_reg(&mut self, id: u32, value: i32) {
        self.sys_regs.insert(id, value);
        if let Some(ref mut session) = self.session {
            session.engine_mut().set_sys_reg(id, value);
        }
    }

    #[inline]
    pub fn get_sys_reg(&self, id: u32) -> i32 {
        self.sys_regs.get(&id).copied().unwrap_or(0)
    }

    /// 专用接口：同步 H 场景选项 (0x00E3(20))，内建 0..=2 边界校验
    /// 0=由选项决定 (_SelectEx), 1=中出 (LABEL_IN), 2=外射 (LABEL_OUT)
    pub fn set_hscene_mode(&mut self, val: i32) -> bool {
        if !(0..=2).contains(&val) {
            return false;
        }
        self.config_regs.insert(20, val);
        if let Some(ref mut session) = self.session {
            let _ = session.engine_mut().set_hscene_mode(val);
        }
        true
    }

    #[inline]
    pub fn get_hscene_mode(&self) -> i32 {
        self.config_regs.get(&20).copied().unwrap_or(0)
    }

    #[inline]
    pub fn current_choice_seq(&self) -> u64 {
        if let Some(ref mail) = self.mail_session {
            return mail.engine().choice_seq();
        }
        self.session
            .as_ref()
            .map(|s| s.engine().choice_seq())
            .unwrap_or(self.choice_seq_base)
    }

    /// 查询共同线独立 mail_vm (window_Mail) 是否处于活跃打开状态
    #[inline]
    pub fn is_mail_window_open(&self) -> bool {
        self.mail_session.is_some()
    }

    /// 读取右下角信封按钮四态 (FLAG[200]: 0=熄灭, 1=可发信, 2=未读回信, 3=个人线隐藏)
    #[inline]
    pub fn mail_btn_state(&self) -> i32 {
        self.get_flag(200)
    }

    /// 打开前门禁校验：仅允许主引擎处于 WaitingForInput 且 FLAG[200] ∈ {1, 2} 且当前未打开 mail_vm
    #[inline]
    pub fn can_open_mail_window(&self) -> bool {
        if self.mail_session.is_some() {
            return false;
        }
        let Some(ref session) = self.session else {
            return false;
        };
        if *session.state() != VmState::WaitingForInput {
            return false;
        }
        matches!(session.engine().get_flag(200), 1 | 2)
    }

    #[inline]
    pub fn unlocked_cgs(&self) -> &BTreeSet<u32> {
        &self.unlocked_cgs
    }

    pub fn consume_system_flags_dirty(&mut self) -> bool {
        self.sync_from_session();
        let d = self.system_flags_dirty;
        self.system_flags_dirty = false;
        d
    }

    /// 导出全局系统标志 JSON（user://system_flags.json）
    pub fn export_system_flags_json(&mut self) -> String {
        self.sync_from_session();
        let persistent_sys_regs: BTreeMap<u32, i32> = self
            .sys_regs
            .iter()
            .filter(|(&k, _)| k != 410)
            .map(|(&k, &v)| (k, v))
            .collect();
        let data = SystemFlagsData {
            version: 1,
            unlocked_cgs: self.unlocked_cgs.clone(),
            sys_regs: persistent_sys_regs,
        };
        serde_json::to_string_pretty(&data).unwrap_or_else(|_| "{}".to_string())
    }

    /// 导入全局系统标志 JSON（仅合并解锁集合与持久 sys_regs，强制保底 0=>0 自定义主角名「李骏」与 410=>0 正式版）
    pub fn import_system_flags_json(&mut self, json: &str) -> bool {
        let Ok(data) = serde_json::from_str::<SystemFlagsData>(json) else {
            return false;
        };
        for cg in data.unlocked_cgs {
            self.unlocked_cgs.insert(cg);
        }
        for (k, v) in data.sys_regs {
            if k != 410 && k != 0 {
                self.sys_regs.insert(k, v);
            }
        }
        // 强制保底自定义主角名 0=>0（覆写旧存档/旧 system_flags.json 中的 "0":1）与正式版判定 410=>0
        self.sys_regs.insert(0, 0);
        self.sys_regs.insert(410, 0);
        if let Some(ref mut session) = self.session {
            let eng = session.engine_mut();
            eng.set_unlocked_cgs(self.unlocked_cgs.clone());
            eng.set_sys_regs(self.sys_regs.clone());
        }
        self.system_flags_dirty = false;
        true
    }

    /// 计算独立 SPR 覆盖层增量变更（用于 _DrawSPR / _FadeSPR 与读档补绘）
    fn compute_spr_update(
        &mut self,
        current: Option<&SprOverlayState>,
        fade_ms_fallback: u32,
    ) -> SprUpdate {
        match current {
            Some(ov) => {
                if self.last_rendered_spr.as_deref() != Some(ov.asset.as_str()) {
                    if let Some(img) = self.resolve_image_arc(&ov.asset) {
                        self.last_rendered_spr = Some(ov.asset.clone());
                        SprUpdate::New {
                            asset: ov.asset.clone(),
                            image: img,
                            fade_ms: ov.fade_ms,
                        }
                    } else if self.last_rendered_spr.is_some() {
                        self.last_rendered_spr = None;
                        SprUpdate::Cleared { fade_ms: 0 }
                    } else {
                        SprUpdate::Unchanged
                    }
                } else {
                    SprUpdate::Unchanged
                }
            }
            None => {
                if self.last_rendered_spr.is_some() {
                    self.last_rendered_spr = None;
                    SprUpdate::Cleared {
                        fade_ms: fade_ms_fallback,
                    }
                } else {
                    SprUpdate::Unchanged
                }
            }
        }
    }

    /// 批量解析本步触发的 _PlaySE 音效资产
    fn resolve_se_list(&self, names: &[String]) -> Vec<(String, Arc<Vec<u8>>)> {
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            if let Some(data) = self.resolve_audio_arc(name) {
                out.push((name.clone(), data));
            }
        }
        out
    }

    /// 根据 VM 返回的对白事件构造表现层增量快照
    fn build_dialogue_snapshot(
        &mut self,
        dlg: DialogueEvent,
        include_intro: bool,
        include_se: bool,
    ) -> DialogueSnapshot {
        self.last_dialogue_index = dlg.index;
        self.last_speaker = dlg.speaker.clone();
        self.last_text = dlg.text.clone();

        let voice_data = dlg
            .voice
            .as_deref()
            .and_then(|v_id| self.resolve_audio_arc(v_id));

        let (bg_update, bg_key) = if dlg.bg != self.last_rendered_bg {
            self.last_rendered_bg = dlg.bg.clone();
            if let Some(ref bg_id) = dlg.bg {
                if let Some(img) = self.resolve_image_arc(bg_id) {
                    (BgUpdate::New(img), Some(bg_id.trim().to_ascii_lowercase()))
                } else {
                    (BgUpdate::Cleared, None)
                }
            } else {
                (BgUpdate::Cleared, None)
            }
        } else {
            (BgUpdate::Unchanged, None)
        };

        let bgm_changed = dlg.bgm != self.last_rendered_bgm;
        let (bgm_name, bgm_data) = if bgm_changed {
            self.last_rendered_bgm = dlg.bgm.clone();
            let data = dlg
                .bgm
                .as_deref()
                .and_then(|b_id| self.resolve_audio_arc(b_id));
            (dlg.bgm.clone(), data)
        } else {
            (None, None)
        };

        let (changed_slot_ids, cleared_slot_ids) =
            compute_slot_delta(&self.last_rendered_slots, &dlg.slots);

        let mut changed_slots = HashMap::new();
        let mut slot_keys = HashMap::new();
        for slot_id in changed_slot_ids {
            if let Some(slot) = dlg.slots.get(&slot_id) {
                if let Some(img) = self.composite_slot_sprites_arc(&slot.sprites) {
                    let key = slot.sprites.join("+").to_ascii_lowercase();
                    changed_slots.insert(slot_id, img);
                    slot_keys.insert(slot_id, key);
                    self.last_rendered_slots
                        .insert(slot_id, slot.sprites.clone());
                }
            }
        }

        for slot_id in &cleared_slot_ids {
            self.last_rendered_slots.remove(slot_id);
        }

        // 计算 FaceAvatar 增量变更（优先按完整 face_sprites 部件表合成半身像）
        let face_changed = self.last_rendered_face.as_deref() != dlg.face.as_deref()
            || self.last_rendered_face_sprites != dlg.face_sprites;
        let (face_texture, face_key) = if face_changed {
            self.last_rendered_face = dlg.face.clone();
            self.last_rendered_face_sprites = dlg.face_sprites.clone();
            if !dlg.face_sprites.is_empty() {
                let k = format!("face:{}", dlg.face_sprites.join("+").to_ascii_lowercase());
                (self.composite_slot_sprites_arc(&dlg.face_sprites), Some(k))
            } else if let Some(ref face_id) = dlg.face {
                let k = format!("face:{}", face_id.trim().to_ascii_lowercase());
                (self.resolve_image_arc(face_id), Some(k))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };
        let voice_asset = dlg.voice.clone();

        // 登场特写演出数据解析 (introduction_{char})
        let character_intro = if include_intro {
            if let Some(ref char_name) = dlg.character_intro {
                let jingle_name = match char_name.as_str() {
                    "sora" => "so_title",
                    "moemi" => "mo_title",
                    "arisa" => "ar_title",
                    "ayumi" => "ay_title",
                    "shiho" => "si_title",
                    _ => "",
                };
                let jingle_data = if !jingle_name.is_empty() {
                    self.resolve_audio_arc(jingle_name)
                } else {
                    None
                };
                let se_data = self.resolve_audio_arc("sceneswitch2");

                let ec_prefix = match char_name.as_str() {
                    "shiho" => "ec_sh_",
                    "arisa" => "ec_ar_",
                    "sora" => "ec_so_",
                    "moemi" => "ec_mo_",
                    "ayumi" => "ec_ay_",
                    _ => "",
                };
                let layers = if !ec_prefix.is_empty() {
                    [
                        self.resolve_image_arc(&format!("{ec_prefix}01")),
                        self.resolve_image_arc(&format!("{ec_prefix}02")),
                        self.resolve_image_arc(&format!("{ec_prefix}03")),
                        self.resolve_image_arc(&format!("{ec_prefix}04")),
                        self.resolve_image_arc(&format!("{ec_prefix}05")),
                    ]
                } else {
                    [None, None, None, None, None]
                };

                let card_name = match char_name.as_str() {
                    "sora" => "ch1_sora0a",
                    "moemi" => "ch1_moemi0a",
                    "arisa" => "ch1_arisa0a",
                    "ayumi" => "ch1_ayumi0a",
                    "shiho" => "ch1_siho0a",
                    _ => "",
                };
                let mut card_image = layers[3].clone().or_else(|| {
                    if !card_name.is_empty() {
                        self.resolve_image_arc(card_name)
                    } else {
                        None
                    }
                });
                if card_image.is_none() {
                    let fallback_sprite = match char_name.as_str() {
                        "sora" => "so01aa_n",
                        "moemi" => "mo01aa_m",
                        "arisa" => "ar01aa_n",
                        "ayumi" => "ay01aa_n",
                        "shiho" => "si01aa_n",
                        _ => "",
                    };
                    if !fallback_sprite.is_empty() {
                        card_image = self.resolve_image_arc(fallback_sprite);
                    }
                }

                Some(CharacterIntroSnapshot {
                    character: char_name.clone(),
                    jingle_data,
                    se_data,
                    card_image,
                    layers,
                })
            } else {
                None
            }
        } else {
            None
        };

        let spr_update = self.compute_spr_update(dlg.spr_overlay.as_ref(), dlg.spr_fade_ms);
        let se_plays = if include_se {
            self.resolve_se_list(&dlg.se_plays)
        } else {
            Vec::new()
        };
        let hide_window_when_selecting = self
            .session
            .as_ref()
            .map(|s| s.engine().hide_window_when_selecting())
            .unwrap_or(false);
        let mail_btn_state = self.mail_btn_state();
        let mail_on_notify = if include_se {
            self.session
                .as_mut()
                .map(|s| s.engine_mut().take_mail_on_notify())
                .unwrap_or(false)
        } else {
            if let Some(ref mut s) = self.session {
                s.engine_mut().clear_mail_on_notify();
            }
            false
        };

        DialogueSnapshot {
            index: dlg.index,
            text: dlg.text,
            speaker: dlg.speaker,
            voice_data,
            bg_update,
            bg_key,
            bg_pose: dlg.bg_pose,
            bg_motions: dlg.bg_motions,
            bgm_changed,
            bgm_name,
            bgm_data,
            changed_slots,
            slot_keys,
            cleared_slots: cleared_slot_ids,
            slot_transforms: dlg.slot_transforms,
            slot_motions: dlg.slot_motions,
            face_changed,
            face_texture,
            face_key,
            face_offset: dlg.face_offset,
            voice_asset,
            character_intro,
            spr_update,
            se_plays,
            hide_window_when_selecting,
            mail_btn_state,
            mail_on_notify,
        }
    }

    /// 方案 4A：独立计算 mail_vm 的 SPR 贴图增量（严禁污染主引擎 last_rendered_spr）
    fn compute_mail_spr_update(
        &mut self,
        current: Option<&SprOverlayState>,
        fade_ms_fallback: u32,
    ) -> SprUpdate {
        match current {
            Some(ov) => {
                if self.mail_last_rendered_spr.as_deref() != Some(ov.asset.as_str()) {
                    if let Some(img) = self.resolve_image_arc(&ov.asset) {
                        self.mail_last_rendered_spr = Some(ov.asset.clone());
                        SprUpdate::New {
                            asset: ov.asset.clone(),
                            image: img,
                            fade_ms: ov.fade_ms,
                        }
                    } else if self.mail_last_rendered_spr.is_some() {
                        self.mail_last_rendered_spr = None;
                        SprUpdate::Cleared { fade_ms: 0 }
                    } else {
                        SprUpdate::Unchanged
                    }
                } else {
                    SprUpdate::Unchanged
                }
            }
            None => {
                if self.mail_last_rendered_spr.is_some() {
                    self.mail_last_rendered_spr = None;
                    SprUpdate::Cleared {
                        fade_ms: fade_ms_fallback,
                    }
                } else {
                    SprUpdate::Unchanged
                }
            }
        }
    }

    /// 方案 4A：打开共同线彩蛋信封独立子引擎 (window_Mail)
    /// 门禁：仅允许主引擎处于 WaitingForInput 且 FLAG[200] ∈ {1, 2} 且当前无活跃 mail_vm。
    pub fn open_mail_window(&mut self) -> StepResult {
        if !self.can_open_mail_window() {
            return StepResult::Error("当前状态不允许打开邮件窗口".to_string());
        }
        let Some(ref loader) = self.script_loader else {
            return StepResult::Error("VFS 尚未初始化".to_string());
        };
        let bytes = match loader.load_script("window_Mail") {
            Ok(Some(b)) => b,
            Ok(None) => return StepResult::Error("未找到脚本 window_Mail".to_string()),
            Err(e) => return StepResult::Error(format!("加载 window_Mail 失败: {:?}", e)),
        };

        let loader_dyn: Arc<dyn ScriptLoader> = Arc::clone(loader) as Arc<dyn ScriptLoader>;
        let mut mail_sess = match ScriptSession::with_loader(bytes, Some(loader_dyn)) {
            Ok(s) => s,
            Err(e) => return StepResult::Error(format!("初始化 mail_vm 失败: {:?}", e)),
        };

        // 状态拷入：克隆主引擎 flags 与 sys_regs（含 sys_regs[0]=0 李骏自定义名）与 config_regs
        let (cloned_flags, cloned_sys_regs, cloned_config_regs) = {
            let main_eng = self.session.as_ref().unwrap().engine();
            (
                main_eng.flags().clone(),
                main_eng.sys_regs().clone(),
                main_eng.config_regs().clone(),
            )
        };
        self.choice_seq_base = self.choice_seq_base.saturating_add(1);
        {
            let m_eng = mail_sess.engine_mut();
            m_eng.set_script_name("window_Mail");
            m_eng.set_flags(cloned_flags.clone());
            m_eng.set_entry_flags(cloned_flags);
            m_eng.set_sys_regs(cloned_sys_regs);
            m_eng.set_config_regs(cloned_config_regs);
            m_eng.set_choice_seq(self.choice_seq_base);
        }

        self.mail_session = Some(mail_sess);
        self.mail_last_rendered_spr = None;
        self.step_mail_session()
    }

    /// 方案 4A：独占步进 mail_vm，抵达 Terminated 时无条件回写并释放
    fn step_mail_session(&mut self) -> StepResult {
        let res = match self.mail_session.as_mut() {
            Some(m) => m.next_dialogue(),
            None => return StepResult::Unloaded,
        };
        match res {
            Ok(Some(_dlg)) => {
                // 实证 window_Mail 链路无 0x0140 对白；若未来存在则继续推进直到挂起或退出
                self.step_mail_session()
            }
            Ok(None) => {
                let (state_clone, spr_overlay, fade_ms, se_names, hide_win, mail_flag_200, cseq) = {
                    let eng = self.mail_session.as_mut().unwrap().engine_mut();
                    (
                        eng.state().clone(),
                        eng.spr_overlay().cloned(),
                        eng.last_spr_fade_ms(),
                        eng.take_pending_se_plays(),
                        eng.hide_window_when_selecting(),
                        eng.get_flag(200),
                        eng.choice_seq(),
                    )
                };
                self.choice_seq_base = self.choice_seq_base.max(cseq);
                match state_clone {
                    VmState::WaitingForWaitClick => {
                        let spr_update =
                            self.compute_mail_spr_update(spr_overlay.as_ref(), fade_ms);
                        let se_plays = self.resolve_se_list(&se_names);
                        StepResult::WaitClick(WaitClickSnapshot {
                            spr_update,
                            se_plays,
                            hide_window_when_selecting: hide_win,
                            mail_btn_state: mail_flag_200,
                            mail_mode: true,
                        })
                    }
                    VmState::WaitingForChoice {
                        prompt,
                        options,
                        disabled_options,
                    } => {
                        let spr_update =
                            self.compute_mail_spr_update(spr_overlay.as_ref(), fade_ms);
                        let se_plays = self.resolve_se_list(&se_names);
                        StepResult::Choice {
                            prompt,
                            options,
                            disabled_options,
                            spr_update,
                            se_plays,
                            hide_window_when_selecting: hide_win,
                            mail_btn_state: mail_flag_200,
                            mail_mode: true,
                        }
                    }
                    VmState::Terminated => {
                        let se_plays = self.resolve_se_list(&se_names);
                        self.close_mail_window(fade_ms, se_plays)
                    }
                    _ => {
                        let se_plays = self.resolve_se_list(&se_names);
                        self.close_mail_window(fade_ms, se_plays)
                    }
                }
            }
            Err(e) => {
                let _ = self.close_mail_window(0, Vec::new());
                StepResult::Error(format!("mail_vm 执行异常: {:?}", e))
            }
        }
    }

    /// 方案 4A：无条件回写 flags[200..=201] 与 flags[300..=419] 回主引擎并释放 mail_vm
    /// 严禁触动主引擎 FLAG[0] / call_stack / pending_dialogue / Backlog / choice_history
    fn close_mail_window(
        &mut self,
        fade_ms: u32,
        se_plays: Vec<(String, Arc<Vec<u8>>)>,
    ) -> StepResult {
        if let Some(mail_sess) = self.mail_session.take() {
            let m_flags = mail_sess.engine().flags();
            let writeback_ids = (200u32..=201u32).chain(300u32..=419u32);
            for id in writeback_ids {
                if let Some(&val) = m_flags.get(&id) {
                    self.session_flags.insert(id, val);
                    if let Some(ref mut main_sess) = self.session {
                        main_sess.engine_mut().set_flag(id, val);
                    }
                }
            }
        }
        self.mail_last_rendered_spr = None;
        let mail_btn_state = self.mail_btn_state();
        StepResult::MailClosed {
            mail_btn_state,
            spr_fade_ms: fade_ms,
            se_plays,
        }
    }

    /// 推进剧本并返回纯数据结果快照（当 mail_vm 活跃时 100% 独占路由至 mail_vm）
    pub fn next_step(&mut self) -> StepResult {
        if self.mail_session.is_some() {
            return self.step_mail_session();
        }
        let res = match self.session.as_mut() {
            Some(s) => s.next_dialogue(),
            None => return StepResult::Unloaded,
        };
        self.sync_from_session();

        match res {
            Ok(Some(dlg)) => {
                let snap = self.build_dialogue_snapshot(dlg, true, true);
                self.schedule_lookahead();
                StepResult::Dialogue(snap)
            }
            Ok(None) => {
                let (state_clone, spr_overlay, fade_ms, se_names, hide_win, mail_btn_state) = {
                    let eng = self.session.as_mut().unwrap().engine_mut();
                    (
                        eng.state().clone(),
                        eng.spr_overlay().cloned(),
                        eng.last_spr_fade_ms(),
                        eng.take_pending_se_plays(),
                        eng.hide_window_when_selecting(),
                        eng.get_flag(200),
                    )
                };
                match state_clone {
                    VmState::WaitingForWaitClick => {
                        let spr_update = self.compute_spr_update(spr_overlay.as_ref(), fade_ms);
                        let se_plays = self.resolve_se_list(&se_names);
                        StepResult::WaitClick(WaitClickSnapshot {
                            spr_update,
                            se_plays,
                            hide_window_when_selecting: hide_win,
                            mail_btn_state,
                            mail_mode: false,
                        })
                    }
                    VmState::WaitingForChoice {
                        prompt,
                        options,
                        disabled_options,
                    } => {
                        let spr_update = self.compute_spr_update(spr_overlay.as_ref(), fade_ms);
                        let se_plays = self.resolve_se_list(&se_names);
                        StepResult::Choice {
                            prompt,
                            options,
                            disabled_options,
                            spr_update,
                            se_plays,
                            hide_window_when_selecting: hide_win,
                            mail_btn_state,
                            mail_mode: false,
                        }
                    }
                    VmState::Terminated => StepResult::Terminated,
                    _ => StepResult::Ready,
                }
            }
            Err(e) => StepResult::Error(format!("{:?}", e)),
        }
    }

    /// 带单调序列号校验提交分支选择 (当 mail_vm 活跃时 100% 独占路由至 mail_vm，不写主线 choice_history)
    pub fn submit_choice(&mut self, choice_seq: u64, index: i64) -> bool {
        if index < 0 {
            return false;
        }
        let idx = index as usize;
        if let Some(ref mut mail_sess) = self.mail_session {
            return mail_sess.submit_choice(choice_seq, idx);
        }
        let ok = if let Some(ref mut session) = self.session {
            session.submit_choice(choice_seq, idx)
        } else {
            false
        };
        if ok {
            self.choice_history.push(idx);
            self.sync_from_session();
        }
        ok
    }

    /// 分支选择输入 (带选项边界校验，兼容旧接口)
    pub fn select_choice(&mut self, index: i64) -> bool {
        let seq = self.current_choice_seq();
        self.submit_choice(seq, index)
    }

    /// 获取当前完整存档元数据（兼容 v1 元组返回）
    pub fn get_save_state(&self) -> (String, usize, Vec<usize>, Option<String>, String) {
        (
            self.current_script.clone().unwrap_or_default(),
            self.last_dialogue_index,
            self.choice_history.clone(),
            self.last_speaker.clone(),
            self.last_text.clone(),
        )
    }

    /// 安全构建存档快照：mail_vm 活跃期间强拦截返回 None，禁止在共同线邮件会话内写档
    pub fn try_build_save_v2(&mut self) -> Option<SaveStateV2> {
        if self.mail_session.is_some() {
            return None;
        }
        Some(self.build_save_v2())
    }

    /// 构建 Schema v2 完整存档结构（支持选项界面、舞台视觉快照与 0x00F0 子脚本调用栈持久化）
    pub fn build_save_v2(&mut self) -> SaveStateV2 {
        self.sync_from_session();
        let (
            pending_choice,
            suspended_pc,
            call_stack,
            script_dialogue_index,
            script_choices,
            active_bg,
            active_bgm,
            active_sprites,
            active_face,
            active_face_sprites,
            active_mail_spr,
            hide_window_when_selecting,
        ) = if let Some(ref session) = self.session {
            let eng = session.engine();
            let pc = Some(eng.pc());
            let frames = eng.call_stack_save();
            let s_idx = Some(eng.script_dialogue_index());
            let s_choices = eng.script_choices().to_vec();
            let bg = eng.current_bg().map(|s| s.to_string());
            let bgm = eng.current_bgm().map(|s| s.to_string());
            let transforms = eng.slot_transforms();
            let mut sprites: Vec<SavedSpriteSlot> = eng
                .slots()
                .iter()
                .map(|(&slot_id, slot)| {
                    let tr = transforms.get(&slot_id);
                    SavedSpriteSlot {
                        slot_id,
                        sprites: slot.sprites.clone(),
                        x: tr.map(|t| t.x).unwrap_or(0),
                        y: tr.map(|t| t.y).unwrap_or(200),
                        scale: tr.map(|t| t.scale).unwrap_or(100),
                    }
                })
                .collect();
            sprites.sort_by_key(|s| s.slot_id);
            let face = eng.current_face().map(|s| s.to_string());
            let face_sprites = eng.current_face_sprites().to_vec();
            let mail_spr = eng.spr_overlay().map(|ov| ov.asset.clone());
            let hide_win = eng.hide_window_when_selecting();
            let pending = if let VmState::WaitingForChoice {
                prompt,
                options,
                disabled_options,
            } = session.state()
            {
                Some(PendingChoiceSave {
                    prompt: prompt.clone(),
                    options: options.clone(),
                    disabled_options: disabled_options.clone(),
                })
            } else {
                None
            };
            (
                pending,
                pc,
                frames,
                s_idx,
                s_choices,
                bg,
                bgm,
                sprites,
                face,
                face_sprites,
                mail_spr,
                hide_win,
            )
        } else {
            (
                None,
                None,
                Vec::new(),
                None,
                Vec::new(),
                None,
                None,
                Vec::new(),
                None,
                Vec::new(),
                None,
                false,
            )
        };

        SaveStateV2 {
            save_version: 2,
            script_name: self.current_script.clone().unwrap_or_default(),
            dialogue_index: self.last_dialogue_index,
            choices: self.choice_history.clone(),
            speaker: self.last_speaker.clone(),
            text: self.last_text.clone(),
            entry_flags: self.entry_flags.clone(),
            flags: self.session_flags.clone(),
            pending_choice,
            choice_seq: self.current_choice_seq(),
            suspended_pc,
            call_stack,
            script_dialogue_index,
            script_choices,
            active_bg,
            active_bgm,
            active_sprites,
            active_face,
            active_face_sprites,
            active_mail_spr,
            hide_window_when_selecting,
        }
    }

    /// 读档重放前将 SaveStateV2 舞台视觉底色注入到 VmEngine
    fn seed_engine_visuals_from_save(&mut self, save: &SaveStateV2) {
        let Some(ref mut session) = self.session else {
            return;
        };
        let mut slots = HashMap::new();
        let mut slot_transforms = HashMap::new();
        for sp in &save.active_sprites {
            if sp.sprites.is_empty() {
                continue;
            }
            slots.insert(
                sp.slot_id,
                CharacterSlot {
                    slot_id: sp.slot_id,
                    sprites: sp.sprites.clone(),
                },
            );
            slot_transforms.insert(
                sp.slot_id,
                SlotTransform {
                    x: sp.x,
                    y: sp.y,
                    scale: sp.scale,
                    fade_ms: 0,
                },
            );
        }
        let face_sprites = if !save.active_face_sprites.is_empty() {
            save.active_face_sprites.clone()
        } else if let Some(ref f) = save.active_face {
            vec![f.clone()]
        } else {
            Vec::new()
        };
        session.engine_mut().seed_stage_visuals(
            save.active_bg.clone(),
            save.active_bgm.clone(),
            slots,
            slot_transforms,
            save.active_face.clone(),
            face_sprites,
        );
    }

    /// 针对 WaitingForChoice 读档分支，从 SaveStateV2 一次性构造/补齐完整舞台视觉快照
    fn build_visual_snapshot_from_save(
        &mut self,
        save: &SaveStateV2,
        base_snap: Option<DialogueSnapshot>,
        prev_bgm: Option<&str>,
    ) -> Option<DialogueSnapshot> {
        let has_saved_visuals = save.active_bg.is_some()
            || save.active_bgm.is_some()
            || !save.active_sprites.is_empty()
            || save.active_face.is_some()
            || !save.active_face_sprites.is_empty();

        if !has_saved_visuals {
            if let Some(mut snap) = base_snap {
                if snap.bgm_name.is_none() {
                    self.last_rendered_bgm = None;
                    snap.bgm_changed = true;
                }
                return Some(snap);
            }
            if prev_bgm.is_some() || save.active_bgm.is_none() {
                // 从有 BGM 或标题页 title_bgm 读入无 BGM 的旧选项档时通知前端停音
                self.last_rendered_bgm = None;
                return Some(DialogueSnapshot {
                    index: 0,
                    text: String::new(),
                    speaker: None,
                    voice_data: None,
                    bg_update: BgUpdate::Unchanged,
                    bg_key: None,
                    bg_pose: None,
                    bg_motions: Vec::new(),
                    bgm_changed: true,
                    bgm_name: None,
                    bgm_data: None,
                    changed_slots: HashMap::new(),
                    slot_keys: HashMap::new(),
                    cleared_slots: Vec::new(),
                    slot_transforms: HashMap::new(),
                    slot_motions: Vec::new(),
                    face_changed: false,
                    face_texture: None,
                    face_key: None,
                    face_offset: (-541, 451),
                    voice_asset: None,
                    character_intro: None,
                    spr_update: SprUpdate::Unchanged,
                    se_plays: Vec::new(),
                    hide_window_when_selecting: save.hide_window_when_selecting,
                    mail_btn_state: self.mail_btn_state(),
                    mail_on_notify: false,
                });
            }
            return None;
        }

        let face_offset = self
            .session
            .as_ref()
            .map(|s| s.engine().current_face_offset())
            .unwrap_or((-541, 451));

        let mut snap = base_snap.unwrap_or_else(|| DialogueSnapshot {
            index: 0,
            text: String::new(),
            speaker: None,
            voice_data: None,
            bg_update: BgUpdate::Unchanged,
            bg_key: None,
            bg_pose: None,
            bg_motions: Vec::new(),
            bgm_changed: false,
            bgm_name: None,
            bgm_data: None,
            changed_slots: HashMap::new(),
            slot_keys: HashMap::new(),
            cleared_slots: Vec::new(),
            slot_transforms: HashMap::new(),
            slot_motions: Vec::new(),
            face_changed: false,
            face_texture: None,
            face_key: None,
            face_offset,
            voice_asset: None,
            character_intro: None,
            spr_update: SprUpdate::Unchanged,
            se_plays: Vec::new(),
            hide_window_when_selecting: save.hide_window_when_selecting,
            mail_btn_state: self.mail_btn_state(),
            mail_on_notify: false,
        });

        // 1. 还原 active_bg（若存档瞬间无 BG 则清除 base_snap 过时残留）
        if let Some(ref bg_id) = save.active_bg {
            if let Some(img) = self.resolve_image_arc(bg_id) {
                self.last_rendered_bg = Some(bg_id.clone());
                snap.bg_update = BgUpdate::New(img);
                snap.bg_key = Some(bg_id.trim().to_ascii_lowercase());
            }
        } else {
            self.last_rendered_bg = None;
            snap.bg_update = BgUpdate::Unchanged;
            snap.bg_key = None;
        }

        // 2. 还原 active_bgm（若目标无 BGM 则无条件置 bgm_changed=true，确保标题页 title_bgm 或旧档 BGM 精准停音）
        let _ = prev_bgm;
        if let Some(ref bgm_id) = save.active_bgm {
            let data = self.resolve_audio_arc(bgm_id);
            self.last_rendered_bgm = Some(bgm_id.clone());
            snap.bgm_changed = true;
            snap.bgm_name = Some(bgm_id.clone());
            snap.bgm_data = data;
        } else {
            self.last_rendered_bgm = None;
            snap.bgm_changed = true;
            snap.bgm_name = None;
            snap.bgm_data = None;
        }

        // 3. 还原 active_sprites（若存档瞬间无在屏立绘，必须清空 base_snap 中已退场立绘残留）
        snap.changed_slots.clear();
        snap.slot_keys.clear();
        snap.cleared_slots.clear();
        snap.slot_transforms.clear();
        self.last_rendered_slots.clear();
        for sp in &save.active_sprites {
            if sp.sprites.is_empty() {
                continue;
            }
            if let Some(img) = self.composite_slot_sprites_arc(&sp.sprites) {
                let key = sp.sprites.join("+").to_ascii_lowercase();
                snap.changed_slots.insert(sp.slot_id, img);
                snap.slot_keys.insert(sp.slot_id, key);
                snap.slot_transforms.insert(
                    sp.slot_id,
                    SlotTransform {
                        x: sp.x,
                        y: sp.y,
                        scale: sp.scale,
                        fade_ms: 0,
                    },
                );
                self.last_rendered_slots
                    .insert(sp.slot_id, sp.sprites.clone());
            }
        }

        // 4. 还原 active_face / active_face_sprites（若存档瞬间无头像，清空 base_snap 过时头像残留）
        let face_sprites = if !save.active_face_sprites.is_empty() {
            save.active_face_sprites.clone()
        } else if let Some(ref f) = save.active_face {
            vec![f.clone()]
        } else {
            Vec::new()
        };
        if !face_sprites.is_empty() {
            let k = format!("face:{}", face_sprites.join("+").to_ascii_lowercase());
            self.last_rendered_face = save.active_face.clone();
            self.last_rendered_face_sprites = face_sprites.clone();
            snap.face_changed = true;
            snap.face_texture = self.composite_slot_sprites_arc(&face_sprites);
            snap.face_key = Some(k);
        } else if let Some(ref face_id) = save.active_face {
            let k = format!("face:{}", face_id.trim().to_ascii_lowercase());
            self.last_rendered_face = Some(face_id.clone());
            self.last_rendered_face_sprites.clear();
            snap.face_changed = true;
            snap.face_texture = self.resolve_image_arc(face_id);
            snap.face_key = Some(k);
        } else {
            self.last_rendered_face = None;
            self.last_rendered_face_sprites.clear();
            snap.face_changed = false;
            snap.face_texture = None;
            snap.face_key = None;
        }

        snap.hide_window_when_selecting = save.hide_window_when_selecting;
        snap.mail_btn_state = self.mail_btn_state();
        snap.mail_on_notify = false;
        Some(snap)
    }

    /// 导出 Schema v2 存档 JSON
    pub fn export_save_v2_json(&mut self) -> String {
        let v2 = self.build_save_v2();
        serde_json::to_string(&v2).unwrap_or_else(|_| "{}".to_string())
    }

    /// 从 Schema v1/v2 JSON 字符串加载存档（支持选项画面直接恢复 WaitingForChoice 且不覆盖全局 CG 解锁）
    pub fn load_save_json(&mut self, json: &str) -> (StepResult, Option<DialogueSnapshot>) {
        match serde_json::from_str::<SaveStateV2>(json) {
            Ok(save) => self.load_save_v2(&save),
            Err(e) => (
                StepResult::Error(format!("解析存档 JSON 失败: {}", e)),
                None,
            ),
        }
    }

    /// 加载 Schema v2 存档（兼容 v1 退化重放；不覆盖全局 unlocked_cgs）
    pub fn load_save_v2(&mut self, save: &SaveStateV2) -> (StepResult, Option<DialogueSnapshot>) {
        let prev_bgm = self.last_rendered_bgm.clone();
        let mut init_flags = if save.save_version >= 2 {
            save.entry_flags.clone()
        } else {
            BTreeMap::new()
        };
        if save.save_version >= 2
            && init_flags.is_empty()
            && !save.flags.is_empty()
            && save.dialogue_index == 0
        {
            init_flags = save.flags.clone();
        }
        self.session_flags = init_flags;
        // 读档时递增运行期 choice_seq 基准，防止读档前旧 UI 回调误触发
        self.choice_seq_base = self.choice_seq_base.max(save.choice_seq).saturating_add(1);

        match self.load_script_fresh(&save.script_name) {
            Ok(true) => {}
            Ok(false) => {
                return (
                    StepResult::Error(format!("剧本不存在: {}", save.script_name)),
                    None,
                )
            }
            Err(e) => {
                return (
                    StepResult::Error(format!("加载剧本失败: {:?}", e)),
                    None,
                )
            }
        }

        // 红线 3：重放子脚本前先注入舞台视觉底色（BG / BGM / 立绘 / 头像）；
        // 若子脚本重放途中切换新视觉则自然覆盖，未切换（如睡前短信）则完好保留父脚本视觉。
        if save.save_version >= 2 {
            self.seed_engine_visuals_from_save(save);
        }

        // 红线 2：读档重放当前脚本时优先消费 save.script_choices；
        // 读取不含 script_choices 的旧 v2 存档时，仅在顶层脚本 (call_stack.is_empty()) 下回退消费 save.choices，
        // 绝不在 0x00F0 子脚本内误吞顶层旧历史第 0 项。
        let replay_choices: &[usize] = if !save.script_choices.is_empty() {
            &save.script_choices
        } else if save.call_stack.is_empty() && !save.choices.is_empty() {
            &save.choices
        } else {
            &[]
        };

        // Schema v2 新档优先按当前活动脚本内对白序号 (script_dialogue_index) 快进，
        // 兼容 0x00F0 子脚本与 0x00F3 尾跳转（两者全局 dialogue_index 均大于当前脚本内句数）；
        // 旧版 v1 / 不含 script_dialogue_index 的 v2 存档退化按全局 dialogue_index 重放。
        let use_script_idx = save.save_version >= 2 && save.script_dialogue_index.is_some();
        let replay_target_idx = if use_script_idx {
            save.script_dialogue_index.unwrap()
        } else {
            save.dialogue_index
        };

        // 若保存于选项画面 (WaitingForChoice)，先快进至 replay_target_idx 重建舞台视觉，再直接恢复挂起态
        let mut last_visuals: Option<DialogueSnapshot> = None;
        if replay_target_idx > 0 {
            let mut choice_cursor = 0usize;
            loop {
                let step_res = match self.session.as_mut() {
                    Some(s) => s.next_dialogue(),
                    None => return (StepResult::Unloaded, None),
                };
                match step_res {
                    Ok(Some(mut dlg)) => {
                        let reached = if use_script_idx {
                            self.session
                                .as_ref()
                                .map(|s| {
                                    let eng = s.engine();
                                    eng.cross_script_call_depth() == 0
                                        && eng.script_name().eq_ignore_ascii_case(&save.script_name)
                                        && eng.script_dialogue_index() >= replay_target_idx
                                })
                                .unwrap_or(false)
                        } else {
                            dlg.index >= replay_target_idx
                        };
                        if reached {
                            if save.save_version >= 2 && save.dialogue_index > 0 {
                                dlg.index = save.dialogue_index;
                            }
                            if let Some(ref mut s) = self.session {
                                s.engine_mut().clear_pending_se_plays();
                            }
                            let mut snap = self.build_dialogue_snapshot(dlg, false, false);
                            if snap.bgm_name.is_none() && self.last_rendered_bgm.is_none() {
                                snap.bgm_changed = true;
                            }
                            if save.pending_choice.is_none() {
                                // 红线 2：完整保留 save.choices 全局选择历史，严禁在重放时覆盖截断
                                self.choice_history = save.choices.clone();
                                if save.save_version >= 2 {
                                    self.session_flags = save.flags.clone();
                                    if let Some(ref mut s) = self.session {
                                        let eng = s.engine_mut();
                                        eng.set_flags(self.session_flags.clone());
                                        eng.set_hide_window_when_selecting(
                                            save.hide_window_when_selecting,
                                        );
                                        if !save.script_choices.is_empty()
                                            || !save.call_stack.is_empty()
                                        {
                                            eng.set_script_choices(save.script_choices.clone());
                                        }
                                        if !save.call_stack.is_empty() {
                                            if let Err(e) = eng.restore_call_stack(&save.call_stack)
                                            {
                                                return (
                                                    StepResult::Error(format!(
                                                        "恢复调用栈失败: {:?}",
                                                        e
                                                    )),
                                                    None,
                                                );
                                            }
                                        }
                                        if save.dialogue_index > 0 {
                                            eng.set_dialogue_counter(save.dialogue_index);
                                        }
                                        if let Some(ref spr_asset) = save.active_mail_spr {
                                            eng.set_spr_overlay(Some(SprOverlayState {
                                                slot: 0,
                                                asset: spr_asset.clone(),
                                                fade_ms: 0,
                                            }));
                                        }
                                    }
                                    if let Some(ref spr_asset) = save.active_mail_spr {
                                        // 强制确保读档补绘手机底图（即使重放前已清空）
                                        self.last_rendered_spr = None;
                                        let ov = SprOverlayState {
                                            slot: 0,
                                            asset: spr_asset.clone(),
                                            fade_ms: 0,
                                        };
                                        snap.spr_update = self.compute_spr_update(Some(&ov), 0);
                                    }
                                    snap.hide_window_when_selecting =
                                        save.hide_window_when_selecting;
                                }
                                if let Some(ref mut s) = self.session {
                                    s.engine_mut().clear_mail_on_notify();
                                }
                                snap.mail_on_notify = false;
                                snap.mail_btn_state = self.mail_btn_state();
                                self.sync_from_session();
                                return (StepResult::Dialogue(snap), None);
                            }
                            last_visuals = Some(snap);
                            break;
                        }
                    }
                    Ok(None) => {
                        let st = self.session.as_ref().unwrap().state().clone();
                        match st {
                            VmState::WaitingForWaitClick => {
                                // 读档快进重放期间自动跨越 _Wait(-1, 1) 并消音
                                if let Some(ref mut s) = self.session {
                                    s.engine_mut().clear_pending_se_plays();
                                    s.engine_mut().clear_mail_on_notify();
                                }
                                continue;
                            }
                            VmState::WaitingForChoice {
                                prompt,
                                options,
                                disabled_options,
                            } => {
                                if choice_cursor < replay_choices.len() {
                                    let ch = replay_choices[choice_cursor];
                                    choice_cursor += 1;
                                    if ch < options.len() {
                                        let seq = self.current_choice_seq();
                                        let _ = self
                                            .session
                                            .as_mut()
                                            .unwrap()
                                            .submit_choice(seq, ch);
                                        continue;
                                    }
                                }
                                // 重放未消费完而停在选项态时仍完整保留全局 choice_history，并恢复 call_stack 与舞台视觉防止 0x001B RET 死机
                                self.choice_history = save.choices.clone();
                                let (spr_overlay, fade_ms, hide_win, mail_btn_state) = {
                                    let eng = self.session.as_mut().unwrap().engine_mut();
                                    eng.clear_pending_se_plays();
                                    eng.clear_mail_on_notify();
                                    if save.save_version >= 2 {
                                        if !save.call_stack.is_empty() {
                                            if let Err(e) = eng.restore_call_stack(&save.call_stack)
                                            {
                                                return (
                                                    StepResult::Error(format!(
                                                        "恢复调用栈失败: {:?}",
                                                        e
                                                    )),
                                                    None,
                                                );
                                            }
                                        }
                                        if save.dialogue_index > 0 {
                                            eng.set_dialogue_counter(save.dialogue_index);
                                        }
                                    }
                                    (
                                        eng.spr_overlay().cloned(),
                                        eng.last_spr_fade_ms(),
                                        eng.hide_window_when_selecting(),
                                        eng.get_flag(200),
                                    )
                                };
                                let spr_update =
                                    self.compute_spr_update(spr_overlay.as_ref(), fade_ms);
                                self.sync_from_session();
                                let restored_visuals = self.build_visual_snapshot_from_save(
                                    save,
                                    last_visuals,
                                    prev_bgm.as_deref(),
                                );
                                return (
                                    StepResult::Choice {
                                        prompt,
                                        options,
                                        disabled_options,
                                        spr_update,
                                        se_plays: Vec::new(),
                                        hide_window_when_selecting: hide_win,
                                        mail_btn_state,
                                        mail_mode: false,
                                    },
                                    restored_visuals,
                                );
                            }
                            VmState::Terminated => {
                                self.choice_history = save.choices.clone();
                                self.sync_from_session();
                                return (StepResult::Terminated, last_visuals);
                            }
                            _ => {
                                self.choice_history = save.choices.clone();
                                self.sync_from_session();
                                return (StepResult::Ready, last_visuals);
                            }
                        }
                    }
                    Err(e) => return (StepResult::Error(format!("{:?}", e)), None),
                }
            }
        }

        // 选项画面直接恢复 WaitingForChoice（不重跑 _SelectEx 防止重弹栈）
        if save.save_version >= 2 {
            if let (Some(ref pending), Some(resume_pc)) =
                (&save.pending_choice, save.suspended_pc)
            {
                let is_custom_name = self
                    .sys_regs
                    .get(&0)
                    .copied()
                    .unwrap_or(pure_connect_core::DEFAULT_IS_DEFAULT_NAME)
                    == 0;
                let clean_prompt = pending
                    .prompt
                    .as_ref()
                    .map(|p| pure_connect_core::apply_protagonist_name(p, is_custom_name));
                let clean_options: Vec<String> = pending
                    .options
                    .iter()
                    .map(|o| pure_connect_core::apply_protagonist_name(o, is_custom_name))
                    .collect();
                self.choice_seq_base = self.choice_seq_base.saturating_add(1);
                let new_seq = self.choice_seq_base;
                self.session_flags = save.flags.clone();
                self.choice_history = save.choices.clone();
                self.seed_engine_visuals_from_save(save);
                if let Some(ref mut session) = self.session {
                    let eng = session.engine_mut();
                    eng.set_flags(self.session_flags.clone());
                    eng.set_unlocked_cgs(self.unlocked_cgs.clone());
                    eng.set_hide_window_when_selecting(save.hide_window_when_selecting);
                    eng.clear_pending_se_plays();
                    eng.clear_mail_on_notify();
                    if !save.script_choices.is_empty() || !save.call_stack.is_empty() {
                        eng.set_script_choices(save.script_choices.clone());
                    }
                    if !save.call_stack.is_empty() {
                        if let Err(e) = eng.restore_call_stack(&save.call_stack) {
                            return (
                                StepResult::Error(format!("恢复调用栈失败: {:?}", e)),
                                None,
                            );
                        }
                    }
                    if save.dialogue_index > 0 {
                        eng.set_dialogue_counter(save.dialogue_index);
                    }
                    if let Some(ref spr_asset) = save.active_mail_spr {
                        eng.set_spr_overlay(Some(SprOverlayState {
                            slot: 0,
                            asset: spr_asset.clone(),
                            fade_ms: 0,
                        }));
                    }
                    eng.restore_waiting_for_choice(
                        resume_pc,
                        clean_prompt.clone(),
                        clean_options.clone(),
                        pending.disabled_options.clone(),
                        new_seq,
                    );
                }
                let spr_update = if let Some(ref spr_asset) = save.active_mail_spr {
                    self.last_rendered_spr = None;
                    let ov = SprOverlayState {
                        slot: 0,
                        asset: spr_asset.clone(),
                        fade_ms: 0,
                    };
                    self.compute_spr_update(Some(&ov), 0)
                } else {
                    let current_ov = self
                        .session
                        .as_ref()
                        .and_then(|s| s.engine().spr_overlay().cloned());
                    self.compute_spr_update(current_ov.as_ref(), 0)
                };
                self.sync_from_session();
                let restored_visuals =
                    self.build_visual_snapshot_from_save(save, last_visuals, prev_bgm.as_deref());
                let mail_btn_state = self.mail_btn_state();
                return (
                    StepResult::Choice {
                        prompt: clean_prompt,
                        options: clean_options,
                        disabled_options: pending.disabled_options.clone(),
                        spr_update,
                        se_plays: Vec::new(),
                        hide_window_when_selecting: save.hide_window_when_selecting,
                        mail_btn_state,
                        mail_mode: false,
                    },
                    restored_visuals,
                );
            }
        }

        // replay_target_idx == 0 且非选项挂起：恢复 flags / call_stack / suspended_pc 后推进一步
        self.choice_history = save.choices.clone();
        if save.save_version >= 2 {
            self.session_flags = save.flags.clone();
            if let Some(ref mut session) = self.session {
                let eng = session.engine_mut();
                eng.set_flags(self.session_flags.clone());
                if !save.script_choices.is_empty() || !save.call_stack.is_empty() {
                    eng.set_script_choices(save.script_choices.clone());
                }
                if !save.call_stack.is_empty() {
                    if let Err(e) = eng.restore_call_stack(&save.call_stack) {
                        return (
                            StepResult::Error(format!("恢复调用栈失败: {:?}", e)),
                            None,
                        );
                    }
                    if let Some(pc) = save.suspended_pc {
                        let _ = eng.seek_pc(pc);
                    }
                }
                if save.dialogue_index > 0 {
                    eng.set_dialogue_counter(save.dialogue_index);
                }
            }
        }
        (self.next_step(), None)
    }

    /// 根据存档快照精确快进并重建完整音画状态（返回视觉上下文，供 GDExtension 复用）
    pub fn load_save_state_with_visuals(
        &mut self,
        script_name: &str,
        dialogue_index: usize,
        choices: &[usize],
    ) -> (StepResult, Option<DialogueSnapshot>) {
        let save = SaveStateV2 {
            save_version: 1,
            script_name: script_name.to_string(),
            dialogue_index,
            choices: choices.to_vec(),
            speaker: None,
            text: String::new(),
            entry_flags: BTreeMap::new(),
            flags: BTreeMap::new(),
            pending_choice: None,
            choice_seq: self.choice_seq_base,
            suspended_pc: None,
            call_stack: Vec::new(),
            script_dialogue_index: None,
            script_choices: Vec::new(),
            active_bg: None,
            active_bgm: None,
            active_sprites: Vec::new(),
            active_face: None,
            active_face_sprites: Vec::new(),
            active_mail_spr: None,
            hide_window_when_selecting: false,
        };
        self.load_save_v2(&save)
    }

    /// 根据存档快照精确快进并重建完整音画状态（兼容 v1 接口）
    pub fn load_save_state(
        &mut self,
        script_name: &str,
        dialogue_index: usize,
        choices: &[usize],
    ) -> StepResult {
        self.load_save_state_with_visuals(script_name, dialogue_index, choices)
            .0
    }

    /// 获取当前活动 SPR 覆盖层资产名（用于测试与状态观测；mail_vm 活跃时优先返回 mail_vm 贴图）
    pub fn current_spr_asset(&self) -> Option<&str> {
        if let Some(ref mail) = self.mail_session {
            return mail
                .engine()
                .spr_overlay()
                .map(|ov| ov.asset.as_str());
        }
        self.session
            .as_ref()
            .and_then(|s| s.engine().spr_overlay())
            .map(|ov| ov.asset.as_str())
    }

    /// 获取最近一次 SPR 淡入/淡出耗时 (ms)
    pub fn last_spr_fade_ms(&self) -> u32 {
        if let Some(ref mail) = self.mail_session {
            return mail.engine().last_spr_fade_ms();
        }
        self.session
            .as_ref()
            .map(|s| s.engine().last_spr_fade_ms())
            .unwrap_or(0)
    }

    /// 获取引擎状态字符串（mail_vm 活跃时返回 mail_vm 状态）
    pub fn get_state(&self) -> &'static str {
        let active = self.mail_session.as_ref().or(self.session.as_ref());
        match active {
            None => "Unloaded",
            Some(s) => match s.state() {
                VmState::Ready => "Ready",
                VmState::Running => "Running",
                VmState::WaitingForInput => "WaitingForInput",
                VmState::WaitingForWaitClick => "WaitingForWaitClick",
                VmState::WaitingForChoice { .. } => "WaitingForChoice",
                VmState::Terminated => "Terminated",
            },
        }
    }

    /// 分页拉取历史对白列表 (供 Backlog 弹窗使用)
    pub fn get_history(&self, offset: usize, limit: usize) -> Vec<pure_connect_core::vm::HistoryEntry> {
        if let Some(ref session) = self.session {
            session.engine().get_history(offset, limit).to_vec()
        } else {
            Vec::new()
        }
    }

    /// 获取特定语音资产的原始音频数据 (已剥离 bw 头部，用于历史语音重听)
    pub fn get_voice_data(&mut self, voice_name: &str) -> Option<Vec<u8>> {
        self.resolve_audio(voice_name)
    }

    /// 按需解码 UI 贴图 (支持 VFS CompressedBG 资产及通用切图)
    pub fn load_ui_image(&mut self, asset_name: &str) -> Option<DecodedImage> {
        self.resolve_image(asset_name)
    }

    /// 解析并解码背景或立绘资产（支持宏展开合成、动画别名、CBG v1/v2、BGI 原始位图与通用切图）
    pub fn resolve_image(&mut self, name: &str) -> Option<DecodedImage> {
        self.resolve_image_arc(name).map(|arc| (*arc).clone())
    }

    pub fn resolve_image_arc(&self, name: &str) -> Option<Arc<DecodedImage>> {
        self.resolver_ctx()?.resolve_image_arc(name)
    }

    /// 解析并提取音频流原生 OGG 字节（剥离 bw 头并纳入 LRU）
    pub fn resolve_audio(&mut self, name: &str) -> Option<Vec<u8>> {
        self.resolve_audio_arc(name).map(|arc| (*arc).clone())
    }

    pub fn resolve_audio_arc(&self, name: &str) -> Option<Arc<Vec<u8>>> {
        self.resolver_ctx()?.resolve_audio_arc(name)
    }

    /// 识别立绘差分部件所属图层类别 (按 BGI 规范 `{prefix}_{size}_{part}[_{variant}]` 精确解析第 3 段部件代码，
    /// 彻底避免 `_f_` 头像尺寸与 `_f_` 眉毛部件、`_m_` 中景尺寸与 `_m_` 嘴巴部件、`_s_` 与其他层互相碰撞覆盖)
    pub fn sprite_layer_category(lower_name: &str) -> Option<&'static str> {
        let clean = lower_name.trim().trim_start_matches('_');
        let mut segs = clean.split('_');
        let prefix = segs.next()?;
        let size_seg = segs.next()?;
        let part_seg = segs.next()?;
        if prefix.is_empty() || size_seg.len() != 1 {
            return None;
        }
        match part_seg {
            "f" => Some("f"),
            "r" => Some("r"),
            "m" => Some("m"),
            "e" => Some("e"),
            "s" => Some("s"),
            "d" => Some("d"),
            "t" => Some("t"),
            "hl" => Some("hl"),
            "h" => Some("h"),
            _ => None,
        }
    }

    /// 将单个立绘参数（可能是宏代号或单独部件）按图层类别合并到已有图层列表
    fn merge_sprite_layer(
        animation_sets: &HashMap<String, Vec<String>>,
        layers: &mut Vec<String>,
        item: &str,
    ) {
        let lower = item.trim().to_ascii_lowercase();
        if lower.is_empty() {
            return;
        }
        if let Some(macro_layers) = animation_sets.get(&lower) {
            for ml in macro_layers {
                Self::merge_single_diff_layer(layers, ml);
            }
        } else {
            Self::merge_single_diff_layer(layers, &lower);
        }
    }

    fn merge_single_diff_layer(layers: &mut Vec<String>, lower: &str) {
        if let Some(cat) = Self::sprite_layer_category(lower) {
            if let Some(slot) = layers
                .iter_mut()
                .find(|l| Self::sprite_layer_category(l) == Some(cat))
            {
                *slot = lower.to_string();
                return;
            }
        }
        if !layers.iter().any(|l| l == lower) {
            layers.push(lower.to_string());
        }
    }

    /// 解析并合成多部件差分立绘（支持 `animationset*` 宏展开、图层覆盖与 3 帧纵向动画图带第 0 帧裁取）
    pub fn composite_slot_sprites(&mut self, sprites: &[String]) -> Option<DecodedImage> {
        self.composite_slot_sprites_arc(sprites)
            .map(|arc| (*arc).clone())
    }

    pub fn composite_slot_sprites_arc(&self, sprites: &[String]) -> Option<Arc<DecodedImage>> {
        self.resolver_ctx()?.composite_slot_sprites_arc(sprites)
    }

    #[inline]
    pub fn vfs(&self) -> Option<&VirtualFileSystem> {
        self.vfs.as_deref()
    }

    #[inline]
    pub fn session(&self) -> Option<&ScriptSession> {
        self.session.as_ref()
    }
}

impl ResolverCtx {
    fn resolve_image_arc(&self, name: &str) -> Option<Arc<DecodedImage>> {
        let clean = name.trim().to_ascii_lowercase();
        if clean.is_empty() {
            return None;
        }
        if self.animation_sets.contains_key(&clean) {
            return self.composite_slot_sprites_arc(&[clean]);
        }
        self.resolve_single_image_arc(name)
    }

    fn resolve_single_image_arc(&self, name: &str) -> Option<Arc<DecodedImage>> {
        let mut clean = name.trim().to_ascii_lowercase();
        if clean.is_empty() {
            return None;
        }
        if let Some(alias_target) = self.animation_aliases.get(&clean) {
            clean = alias_target.clone();
        }

        if let Ok(mut guard) = self.image_cache.lock() {
            if let Some(cached) = guard.get(&clean) {
                return Some(Arc::clone(cached));
            }
        }

        if clean == "white" {
            let img = Arc::new(DecodedImage {
                width: 1280,
                height: 720,
                bpp: 32,
                rgba_data: vec![255; 1280 * 720 * 4],
            });
            if let Ok(mut guard) = self.image_cache.lock() {
                guard.insert(clean, Arc::clone(&img), img.rgba_data.len());
            }
            return Some(img);
        }
        if clean == "black" {
            let mut rgba = vec![0; 1280 * 720 * 4];
            for i in (3..rgba.len()).step_by(4) {
                rgba[i] = 255;
            }
            let img = Arc::new(DecodedImage {
                width: 1280,
                height: 720,
                bpp: 32,
                rgba_data: rgba,
            });
            if let Ok(mut guard) = self.image_cache.lock() {
                guard.insert(clean, Arc::clone(&img), img.rgba_data.len());
            }
            return Some(img);
        }

        let base = clean.trim_start_matches('_');
        let mut candidates = Vec::with_capacity(32);
        candidates.push(clean.clone());
        if !clean.starts_with('_') {
            candidates.push(format!("_{}", clean));
        } else {
            candidates.push(base.to_string());
        }

        let exts = ["png", "jpg", "jpeg", "bmp", "webp"];
        for ext in &exts {
            candidates.push(format!("{}.{}", clean, ext));
            candidates.push(format!("{}.{}", base, ext));
            candidates.push(format!("_{}.{}", base, ext));
        }

        if clean.len() >= 6 {
            if let Some(under_pos) = clean.find('_') {
                if under_pos >= 4 {
                    let prefix = &clean[..under_pos];
                    let after = &clean[under_pos + 1..];
                    if !after.is_empty() {
                        let size_char = after.chars().next().unwrap();
                        candidates.push(format!("{}aa_{}", prefix, size_char));
                        candidates.push(format!("_{}aa_{}", prefix, size_char));
                        candidates.push(format!("{}da_{}", prefix, size_char));
                        candidates.push(format!("_{}da_{}", prefix, size_char));
                        candidates.push(format!("{}aa_n", prefix));
                        candidates.push(format!("_{}aa_n", prefix));
                        candidates.push(format!("{}_{}_b_01", prefix, size_char));
                        candidates.push(format!("_{}_{}_b_01", prefix, size_char));
                        candidates.push(format!("{}_n_b_01", prefix));
                        candidates.push(format!("_{}_n_b_01", prefix));

                        if prefix.len() >= 2 {
                            let char2 = &prefix[..2];
                            candidates.push(format!("{char2}01aa_{size_char}"));
                            candidates.push(format!("_{char2}01aa_{size_char}"));
                            candidates.push(format!("{char2}01aa_n"));
                            candidates.push(format!("{char2}03aa_{size_char}"));
                            candidates.push(format!("{char2}05aa_{size_char}"));
                            candidates.push(format!("{char2}01_{size_char}_b_01"));
                            candidates.push(format!("_{char2}01_{size_char}_b_01"));
                            candidates.push(format!("{char2}01_n_b_01"));
                            candidates.push(format!("_{char2}01_n_b_01"));
                            candidates.push(format!("{char2}03_{size_char}_b_01"));
                            candidates.push(format!("{char2}05_{size_char}_b_01"));
                        }
                    }
                }
            }
        }
        candidates.dedup();

        for c in &candidates {
            if let Ok(raw_bytes) = self.vfs.read(c) {
                let decoded_cow = if raw_bytes.starts_with(b"DSC FORMAT 1.00\0") {
                    match Dsc::decode(&raw_bytes) {
                        Ok(inner) => std::borrow::Cow::Owned(inner),
                        Err(e) => {
                            eprintln!(
                                "[PureConnect VFS] Dsc::decode failed for candidate '{}' (requested '{}'): {:?}",
                                c, clean, e
                            );
                            continue;
                        }
                    }
                } else {
                    raw_bytes
                };

                if decoded_cow.starts_with(b"CompressedBG___\0") {
                    match CompressedBg::decode(&decoded_cow) {
                        Ok(img) => {
                            let arc = Arc::new(img);
                            if let Ok(mut guard) = self.image_cache.lock() {
                                guard.insert(clean, Arc::clone(&arc), arc.rgba_data.len());
                            }
                            return Some(arc);
                        }
                        Err(e) => {
                            eprintln!(
                                "[PureConnect VFS] CompressedBg::decode failed for candidate '{}' (requested '{}'): {:?}",
                                c, clean, e
                            );
                            continue;
                        }
                    }
                } else if let Ok(img) = CompressedBg::decode_raw_bgi(&decoded_cow) {
                    let arc = Arc::new(img);
                    if let Ok(mut guard) = self.image_cache.lock() {
                        guard.insert(clean, Arc::clone(&arc), arc.rgba_data.len());
                    }
                    return Some(arc);
                } else if let Ok(dyn_img) = image::load_from_memory(&decoded_cow) {
                    let rgba = dyn_img.to_rgba8();
                    let decoded = DecodedImage {
                        width: rgba.width(),
                        height: rgba.height(),
                        bpp: 32,
                        rgba_data: rgba.into_raw(),
                    };
                    let arc = Arc::new(decoded);
                    if let Ok(mut guard) = self.image_cache.lock() {
                        guard.insert(clean, Arc::clone(&arc), arc.rgba_data.len());
                    }
                    return Some(arc);
                }
            }
        }

        eprintln!(
            "[PureConnect VFS] resolve_image 未命中或全部解码失败: original='{}', clean='{}', 尝试候选清单: {:?}",
            name, clean, candidates
        );
        None
    }

    fn resolve_audio_arc(&self, name: &str) -> Option<Arc<Vec<u8>>> {
        let clean = name.trim().to_ascii_lowercase();
        if clean.is_empty() {
            return None;
        }

        if let Ok(mut guard) = self.audio_cache.lock() {
            if let Some(cached) = guard.get(&clean) {
                return Some(Arc::clone(cached));
            }
        }

        let base = clean.trim_start_matches('_');
        let mut candidates = Vec::with_capacity(12);
        candidates.push(clean.clone());
        if !clean.starts_with('_') {
            candidates.push(format!("_{}", clean));
        } else {
            candidates.push(base.to_string());
        }

        let exts = ["ogg", "bw", "wav"];
        for ext in &exts {
            candidates.push(format!("{}.{}", clean, ext));
            candidates.push(format!("{}.{}", base, ext));
            candidates.push(format!("_{}.{}", base, ext));
        }
        candidates.dedup();

        let mut data_cow = None;
        for c in &candidates {
            if let Ok(d) = self.vfs.read(c) {
                data_cow = Some(d);
                break;
            }
        }

        let raw_bytes = data_cow?;
        let clean_bytes = BwAudio::strip_header(&raw_bytes).ok()?;
        let result = Arc::new(clean_bytes.to_vec());

        if let Ok(mut guard) = self.audio_cache.lock() {
            guard.insert(clean, Arc::clone(&result), result.len());
        }
        Some(result)
    }

    fn resolve_full_body_base(&self, prefix: &str, size_char: char) -> Option<Arc<DecodedImage>> {
        let mut full_body_candidates = vec![
            format!("{}aa_{}", prefix, size_char),
            format!("_{}aa_{}", prefix, size_char),
            format!("{}da_{}", prefix, size_char),
            format!("_{}da_{}", prefix, size_char),
            format!("{}aa_n", prefix),
            format!("_{}aa_n", prefix),
            format!("{}_{}_b_01", prefix, size_char),
            format!("_{}_{}_b_01", prefix, size_char),
            format!("{}_n_b_01", prefix),
        ];
        if prefix.len() >= 2 {
            let char2 = &prefix[..2];
            full_body_candidates.push(format!("{char2}01aa_{size_char}"));
            full_body_candidates.push(format!("{char2}01aa_n"));
            full_body_candidates.push(format!("{char2}03aa_{size_char}"));
            full_body_candidates.push(format!("{char2}05aa_{size_char}"));
            full_body_candidates.push(format!("{char2}01_{size_char}_b_01"));
            full_body_candidates.push(format!("{char2}01_n_b_01"));
            full_body_candidates.push(format!("{char2}03_{size_char}_b_01"));
            full_body_candidates.push(format!("{char2}05_{size_char}_b_01"));
        }
        for body_cand in &full_body_candidates {
            if let Some(img) = self.resolve_single_image_arc(body_cand) {
                return Some(img);
            }
        }
        None
    }

    fn composite_slot_sprites_arc(&self, sprites: &[String]) -> Option<Arc<DecodedImage>> {
        if sprites.is_empty() {
            return None;
        }

        let cache_key = sprites.join("+").to_ascii_lowercase();
        if let Ok(mut guard) = self.image_cache.lock() {
            if let Some(cached) = guard.get(&cache_key) {
                return Some(Arc::clone(cached));
            }
        }

        let first_lower = sprites[0].trim().to_ascii_lowercase();
        let first_is_macro = self.animation_sets.contains_key(&first_lower);
        let first_is_diff = PureConnectRuntime::sprite_layer_category(&first_lower).is_some();

        let (base_arc, diff_names): (Arc<DecodedImage>, Vec<String>) = if first_is_macro {
            let mut layers = self
                .animation_sets
                .get(&first_lower)
                .cloned()
                .unwrap_or_default();
            for ovr in &sprites[1..] {
                PureConnectRuntime::merge_sprite_layer(&self.animation_sets, &mut layers, ovr);
            }
            let base = if let Some(under_pos) = first_lower.find('_') {
                let prefix = &first_lower[..under_pos];
                let size_char = first_lower[under_pos + 1..].chars().next().unwrap_or('n');
                self.resolve_full_body_base(prefix, size_char)
            } else {
                None
            }
            .or_else(|| self.resolve_single_image_arc(&sprites[0]))?;
            (base, layers)
        } else if first_is_diff {
            let base = if let Some(under_pos) = first_lower.find('_') {
                let prefix = &first_lower[..under_pos];
                let size_char = first_lower[under_pos + 1..].chars().next().unwrap_or('n');
                self.resolve_full_body_base(prefix, size_char)
            } else {
                None
            };
            if let Some(b) = base {
                let mut layers = Vec::new();
                for s in sprites {
                    PureConnectRuntime::merge_sprite_layer(&self.animation_sets, &mut layers, s);
                }
                (b, layers)
            } else {
                let b = self.resolve_single_image_arc(&sprites[0])?;
                let mut layers = Vec::new();
                for s in &sprites[1..] {
                    PureConnectRuntime::merge_sprite_layer(&self.animation_sets, &mut layers, s);
                }
                (b, layers)
            }
        } else {
            let b = self.resolve_single_image_arc(&sprites[0])?;
            let mut layers = Vec::new();
            for s in &sprites[1..] {
                PureConnectRuntime::merge_sprite_layer(&self.animation_sets, &mut layers, s);
            }
            (b, layers)
        };

        // 无差分图层时直接复用 Arc，零内存拷贝
        if diff_names.is_empty() {
            if let Ok(mut guard) = self.image_cache.lock() {
                guard.insert(cache_key, Arc::clone(&base_arc), base_arc.rgba_data.len());
            }
            return Some(base_arc);
        }

        let mut base_img = (*base_arc).clone();

        let mut decoded_diffs: Vec<(String, Arc<DecodedImage>)> =
            Vec::with_capacity(diff_names.len());
        let mut ref_h: Option<u32> = None;
        for name in &diff_names {
            if let Some(img) = self.resolve_single_image_arc(name) {
                let is_anim_candidate = matches!(
                    PureConnectRuntime::sprite_layer_category(name),
                    Some("e") | Some("f") | Some("d")
                );
                if !is_anim_candidate && img.height < base_img.height {
                    ref_h = Some(match ref_h {
                        Some(cur) => cur.min(img.height),
                        None => img.height,
                    });
                }
                decoded_diffs.push((name.clone(), img));
            }
        }

        if ref_h.is_none() {
            if let Some((first_diff, _)) = decoded_diffs.first() {
                if let Some(under_pos) = first_diff.find('_') {
                    let prefix = &first_diff[..under_pos];
                    let size_char = first_diff[under_pos + 1..]
                        .chars()
                        .next()
                        .unwrap_or('n');
                    let b01_name = format!("{}_{}_b_01", prefix, size_char);
                    if let Some(b01_img) = self.resolve_single_image_arc(&b01_name) {
                        ref_h = Some(b01_img.height);
                    }
                }
            }
        }

        for (diff_name, diff_img) in &decoded_diffs {
            let is_anim_cat = matches!(
                PureConnectRuntime::sprite_layer_category(diff_name),
                Some("e") | Some("f") | Some("d")
            );
            let is_3frame_strip = diff_img.height % 3 == 0
                && (ref_h.is_some_and(|rh| diff_img.height == rh * 3)
                    || (is_anim_cat && diff_img.height > base_img.height));
            let crop_h = if is_3frame_strip {
                diff_img.height / 3
            } else {
                diff_img.height
            };
            let _ = SpriteCompositor::composite_cropped(&mut base_img, diff_img, 0, 0, crop_h);
        }

        let out_arc = Arc::new(base_img);
        if let Ok(mut guard) = self.image_cache.lock() {
            guard.insert(cache_key, Arc::clone(&out_arc), out_arc.rgba_data.len());
        }
        Some(out_arc)
    }
}
