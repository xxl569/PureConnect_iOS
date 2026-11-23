//! BGI 字节码执行引擎与对白状态机 (VM Engine)

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::error::{CoreError, Result};
use super::decoder::{BgiScript, OwnedBgiScript};
use super::loader::ScriptLoader;
use super::opcode::OpCode;

/// 单次 step() 最大执行指令数熔断上限，防止畸形或死循环字节码卡死主线程
pub const MAX_OPS_PER_TICK: usize = 50_000;

/// 0x00F0 子脚本最大嵌套调用深度上限，防止恶意/循环脚本造成调用栈溢出
pub const MAX_CALL_DEPTH: usize = 16;

/// 默认 PRNG 初始种子（非零，xorshift64）
const DEFAULT_RNG_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// 单句对白推进最大中间事件数上限，防止无限事件循环
const MAX_EVENTS_PER_DIALOGUE: usize = 10_000;

/// 主人公默认名标志寄存器 (sys_regs[0] / 0x0107 IsDefaultName) 极性：
/// 0 = 自定义姓名（「李骏」），1 = 原版默认名（「宇佐見隼人」）
pub const DEFAULT_IS_DEFAULT_NAME: i32 = 0;

/// 统一替换主人公姓名占位符，并在自定义姓名态 (`is_custom_name == true`) 下修正 4 处汉化组漏删台词
pub fn apply_protagonist_name(s: &str, is_custom_name: bool) -> String {
    let mut out = s.to_string();
    if out.contains("主人公") {
        for pat in [
            "主人公（姓）\u{3000}主人公（名）",
            "主人公（姓） 主人公（名）",
            "主人公（姓）主人公（名）",
            "主人公（名）",
        ] {
            if out.contains(pat) {
                out = out.replace(pat, "李骏");
            }
        }
        if out.contains("主人公（姓）") {
            out = out.replace("主人公（姓）", "李");
        }
    }
    if is_custom_name {
        const SCOPED_CUSTOM_NAME_FIXES: [(&str, &str); 4] = [
            // arisa_05 @ 0x190a4
            ("想要一直呆在隼人君的身边", "想要一直呆在前辈的身边"),
            // moemi_03 @ 0x1a134
            (
                "今天一整天都能和最喜欢的宇佐喵一起约会",
                "今天一整天都能和最喜欢的你一起约会",
            ),
            // moemi_07_h1 @ 0x1870
            ("在家的时候一直都在在意隼人的事", "在家的时候一直都在在意你的事"),
            // moemi_08 @ 0x40f8
            ("总觉得隼人像小孩子一样", "总觉得你像小孩子一样"),
        ];
        for (from, to) in SCOPED_CUSTOM_NAME_FIXES {
            if out.contains(from) {
                out = out.replace(from, to);
            }
        }
    }
    out
}

/// 虚拟机运行状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmState {
    /// 准备就绪，尚未开始
    Ready,
    /// 正常执行中
    Running,
    /// 遇到对白断点，挂起等待用户点击或推进 (Wait for Click/Next)
    WaitingForInput,
    /// 遇到无限期点击等待 (_Wait(-1, 1))，挂起等待玩家点击翻页（零污染 dialogue_counter / Backlog）
    WaitingForWaitClick,
    /// 遇到分支选择，等待用户做出决策
    WaitingForChoice {
        prompt: Option<String>,
        options: Vec<String>,
        disabled_options: Vec<usize>,
    },
    /// 剧本执行结束
    Terminated,
}

/// 独立 SPR 覆盖层状态（如主线/共同线手机短信贴图 slot 0）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SprOverlayState {
    pub slot: u32,
    pub asset: String,
    pub fade_ms: u32,
}

/// 立绘槽位状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterSlot {
    /// 槽位编号 (如 1, 2, 3)
    pub slot_id: u32,
    /// 差分/部件资产名列表 (如 ["SO01_L_S_A11", "SO01_L_M_A14"])
    pub sprites: Vec<String>,
}

/// 背景几何起始/当前位姿
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BgPose {
    /// 水平偏移 (None 表示 BGI 32767 自动居中)
    pub x: Option<i32>,
    /// 垂直偏移 (None 表示 BGI 32767 自动居中)
    pub y: Option<i32>,
    /// 缩放百分比 (默认 100)
    pub scale: i32,
    /// 场景淡入/切换耗时 (毫秒)
    pub transition_ms: u32,
}

/// 背景缓动运镜轨迹 (由 `_MoveSight` / `_ScrollBG` 驱动)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BgMotion {
    pub target_x: Option<i32>,
    pub target_y: Option<i32>,
    pub keep_x: bool,
    pub keep_y: bool,
    pub target_scale: Option<i32>,
    pub duration_ms: u32,
}

/// 舞台立绘槽位坐标与缩放状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotTransform {
    /// 相对于屏幕中心 (640) 的水平偏移
    pub x: i32,
    /// 相对于屏幕中心 (360) 的垂直偏移 (常规站姿为 200)
    pub y: i32,
    /// 缩放百分比 (默认 100)
    pub scale: i32,
    /// 淡入耗时 (毫秒)
    pub fade_ms: u32,
}

/// 舞台立绘位移/晃动指令 (由 `_MoveBS` / `_MoveRepeatBS` 驱动)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotMotion {
    pub slot_id: u32,
    pub is_repeat: bool,
    pub target_x: Option<i32>,
    pub target_y: Option<i32>,
    pub dx: i32,
    pub dy: i32,
    pub from_x: i32,
    pub from_y: i32,
    pub to_x: i32,
    pub to_y: i32,
    pub duration_ms: u32,
}

/// 对白与剧情快照事件 (Dialogue Event)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogueEvent {
    /// 对白序号 (1-based)
    pub index: usize,
    /// 说话人/角色名 (若为 None 则为旁白或内心独白)
    pub speaker: Option<String>,
    /// 台词正文
    pub text: String,
    /// 语音音频文件名/ID (如 "AY_0202")
    pub voice: Option<String>,
    /// 当前半身头像标识 (如 "MO01_F004")
    pub face: Option<String>,
    /// 当前半身头像完整合成部件列表 (如 ["MO01PC_F", "MO01_F004"])
    pub face_sprites: Vec<String>,
    /// 半身头像在 1280x720 坐标系中的偏移 (默认 (-541, 451))
    pub face_offset: (i32, i32),
    /// 当前背景画面标识 (如 "white", "pc_bg_13A")
    pub bg: Option<String>,
    /// 当前背景初始坐标与缩放
    pub bg_pose: Option<BgPose>,
    /// 本句台词关联的背景运镜序列 (_MoveSight / _ScrollBG)
    pub bg_motions: Vec<BgMotion>,
    /// 当前背景音乐 (如 "PC_BGM_17")
    pub bgm: Option<String>,
    /// 当前各槽位立绘状态
    pub slots: HashMap<u32, CharacterSlot>,
    /// 各槽位几何坐标状态
    pub slot_transforms: HashMap<u32, SlotTransform>,
    /// 本句台词触发的立绘位移动画序列
    pub slot_motions: Vec<SlotMotion>,
    /// 触发该对白的指令字节偏移 (PC)
    pub pc: usize,
    /// 新角色登场特写转场 (如 "sora", "moemi", "arisa", "ayumi", "shiho")
    pub character_intro: Option<String>,
    /// 当前活动独立 SPR 覆盖层（如手机在屏期间叠加男主吐槽对白）
    pub spr_overlay: Option<SprOverlayState>,
    /// 最近一次 SPR 淡出/切换耗时 (ms)
    pub spr_fade_ms: u32,
    /// 本步推进期间触发的 _PlaySE 音效名列表
    pub se_plays: Vec<String>,
}

/// 履历记录条目 (Backlog Entry)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub index: usize,
    pub speaker: Option<String>,
    pub text: String,
    pub voice: Option<String>,
}

/// VM 执行过程中发出的细粒度指令事件 (可供 Godot / GDExtension 表现层精准监听)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmEvent {
    /// 对白推进事件 (断点挂起)
    Dialogue(DialogueEvent),
    /// 背景切换
    SceneChange { bg: String },
    /// 背景音乐触发
    BgmPlay { bgm: String },
    /// 角色语音播放
    VoicePlay { voice: String },
    /// 立绘槽位加载/绘制
    SpriteDraw { slot: u32, sprites: Vec<String> },
    /// 立绘槽位清空
    SpriteClear { slot: Option<u32> },
    /// 分支选择
    Choice {
        prompt: Option<String>,
        options: Vec<String>,
        disabled_options: Vec<usize>,
    },
    /// 角色登场介绍特写 (Opcode 0x00F0 introduction_{char})
    CharacterIntro {
        character: String,
        voice_jingle: Option<String>,
    },
    /// 脚本等待事件 (_Wait(duration_ms, skippable))
    Wait {
        duration_ms: u32,
        skippable: bool,
    },
    /// 无限期等待玩家点击翻页 (_Wait(-1, 1))
    WaitClick,
    /// 独立 SPR 覆盖层绘制 (_DrawSPR，白名单放行)
    SprDraw {
        slot: u32,
        asset: String,
        fade_ms: u32,
    },
    /// 独立 SPR 覆盖层淡出 (_FadeSPR)
    SprFade {
        slot: u32,
        fade_ms: u32,
    },
    /// 音效播放 (_PlaySE)
    SePlay {
        se: String,
        volume: i32,
    },
}

/// 持久化调用栈帧（供 SaveStateV2 序列化；恢复时按目标脚本头重新计算 base_offset）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallFrameSave {
    #[serde(default)]
    pub script_name: String,
    #[serde(default)]
    pub return_pc: usize,
    #[serde(default)]
    pub is_local: bool,
    #[serde(default)]
    pub script_dialogue_index: usize,
    #[serde(default)]
    pub entry_flags: BTreeMap<u32, i32>,
    #[serde(default)]
    pub script_choices: Vec<usize>,
}

/// 当前活动脚本容器（兼容借用单测与跨脚本所有权切换，零 unsafe）
#[derive(Clone)]
enum ActiveScript<'a> {
    Borrowed(&'a BgiScript<'a>),
    Owned(Arc<OwnedBgiScript>),
}

impl<'a> ActiveScript<'a> {
    #[inline]
    fn base_offset(&self) -> usize {
        match self {
            ActiveScript::Borrowed(s) => s.base_offset(),
            ActiveScript::Owned(s) => s.base_offset(),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match self {
            ActiveScript::Borrowed(s) => s.len(),
            ActiveScript::Owned(s) => s.len(),
        }
    }

    #[inline]
    fn read_u32(&self, abs_offset: usize) -> Result<u32> {
        match self {
            ActiveScript::Borrowed(s) => s.read_u32(abs_offset),
            ActiveScript::Owned(s) => s.read_u32(abs_offset),
        }
    }

    #[inline]
    fn read_i32(&self, abs_offset: usize) -> Result<i32> {
        match self {
            ActiveScript::Borrowed(s) => s.read_i32(abs_offset),
            ActiveScript::Owned(s) => s.read_i32(abs_offset),
        }
    }

    #[inline]
    fn get_string(&self, rel_offset: u32) -> Result<String> {
        match self {
            ActiveScript::Borrowed(s) => s.get_string(rel_offset),
            ActiveScript::Owned(s) => s.get_string(rel_offset),
        }
    }
}

/// 运行期子脚本/页内本地调用栈帧 (0x00F0 / 0x001A 压栈 / 0x001B 弹栈)
#[derive(Clone)]
struct CallFrame<'a> {
    script: ActiveScript<'a>,
    script_name: String,
    return_pc: usize,
    has_standard_opcodes: bool,
    script_dialogue_index: usize,
    entry_flags: BTreeMap<u32, i32>,
    script_choices: Vec<usize>,
    /// true 表示 0x001A 页内本地标签调用帧：0x001B 返回时仅恢复 pc，不重载脚本/不重置 entry_flags 与对白计数
    is_local: bool,
}

/// 强类型 BGI 求值栈单元（严禁用裸 u32 混存有符号整数与偏移）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmValue {
    Int(i32),
    StrOffset(u32),
    Addr(u32),
    Other,
}

impl VmValue {
    #[inline]
    pub fn as_int(&self) -> Option<i32> {
        match *self {
            VmValue::Int(v) => Some(v),
            _ => None,
        }
    }

    #[inline]
    pub fn as_addr(&self) -> Option<u32> {
        match *self {
            VmValue::Addr(a) | VmValue::StrOffset(a) => Some(a),
            VmValue::Int(v) if v >= 0 => Some(v as u32),
            _ => None,
        }
    }
}

/// BGI 字节码执行虚拟机引擎
#[derive(Clone)]
pub struct VmEngine<'a> {
    script: ActiveScript<'a>,
    script_name: String,
    pc: usize,
    state: VmState,
    has_standard_opcodes: bool,
    eval_stack: Vec<VmValue>,
    /// 跨脚本统一加载器（可选；绑定后支持 0x00F0 / 0x00F3 真实跨脚本切换与调用栈恢复）
    script_loader: Option<Arc<dyn ScriptLoader>>,
    /// 0x00F0 子脚本调用栈（上限 MAX_CALL_DEPTH=16，0x00F3 尾跳转清空，0x001B 弹栈返回）
    call_stack: Vec<CallFrame<'a>>,
    /// 轻量可设种子 PRNG 状态（xorshift64，供 Random(n) 使用）
    rng_state: u64,
    /// 档内剧本进度变量表 (0..=419)，随存档保存与回滚，切换下一脚本时保留
    flags: BTreeMap<u32, i32>,
    /// 当前活动脚本入口时的 flags 快照（随 0x00F0 / 0x00F3 切换更新，0x001B 返回时恢复父帧快照）
    entry_flags: BTreeMap<u32, i32>,
    /// 系统只读/运行寄存器表 (如 0=>1 主角名"隼人", 410=>0 正式版, 50=>0 输入判定)
    sys_regs: BTreeMap<u32, i32>,
    /// 0x00E4 修改跨周目持久系统寄存器时的脏标记
    sys_regs_dirty: bool,
    /// 玩家配置寄存器表 (专门供 0x00E3 查询，如 20=>0 H场景选项：0=由选项决定, 1=中出, 2=外射)
    config_regs: BTreeMap<u32, i32>,
    /// 全局 CG 解锁集合 (0x00E6)，独立持久化且不被旧档覆盖回滚
    unlocked_cgs: BTreeSet<u32>,
    unlocked_cgs_dirty: bool,
    /// 单调递增选项序列号，防止连点或读档前旧 UI 回调误消费新选项
    choice_seq: u64,
    current_bg: Option<String>,
    current_bg_pose: Option<BgPose>,
    pending_bg_motions: Vec<BgMotion>,
    current_bgm: Option<String>,
    current_voice: Option<String>,
    current_face: Option<String>,
    current_face_sprites: Vec<String>,
    current_face_offset: (i32, i32),
    slots: HashMap<u32, CharacterSlot>,
    slot_transforms: HashMap<u32, SlotTransform>,
    pending_slot_motions: Vec<SlotMotion>,
    /// 记录上一句对白之后、当前场景批次内新绘制过的槽位（避免在 _DrawScene 同批预置立绘被误清）
    slots_drawn_since_dialogue: HashSet<u32>,
    /// 记录上一句对白之后是否新绘制了半身头像
    face_drawn_since_dialogue: bool,
    dialogue_counter: usize,
    /// 当前活动脚本内的对白计数（切换脚本时清零、0x001B 返回时恢复父帧计数）
    script_dialogue_index: usize,
    /// 当前活动脚本局部选择历史（0x00F0 压栈清空、0x001B 跨脚本返回归并回父帧、0x00F3 尾跳转清空）
    script_choices: Vec<usize>,
    pending_dialogue: Option<DialogueEvent>,
    pending_intro: Option<String>,
    history: Vec<HistoryEntry>,
    /// 局部短字符串表 (_SetLocalShortStrings(slot, text))，支持 %0 / %1 参数展开
    local_short_strings: BTreeMap<u32, String>,
    /// 独立 SPR 覆盖层（仅白名单 slot 0，如主线/共同线手机贴图）
    spr_overlay: Option<SprOverlayState>,
    /// 最近一次 SPR 淡出/清屏耗时 (ms)
    last_spr_fade_ms: u32,
    /// 本步推进期间收集的 _PlaySE 音效队列
    pending_se_plays: Vec<String>,
    /// 0x0167 (HideWindowWhenItemSelecting) 状态标志
    hide_window_when_selecting: bool,
    /// 0x001A 页内本地标签调用传入的待匹配标签名（0x0016/0x00F7/0x0017 消费后清空）
    pending_local_label: Option<String>,
    /// 0x0166 (DisableItem) 待下次 _SelectEx 消费的 0-based 禁用选项下标集合
    pending_disabled_choices: BTreeSet<usize>,
    /// _AnimateSPR(9, "mail_on_A", ...) 触发的可发邮件提示气泡一次性标志
    pending_mail_on_notify: bool,
}

impl<'a> VmEngine<'a> {
    /// 构建默认系统寄存器表 (供 0x00E5 / 0x00E7 查询)
    pub fn default_sys_regs() -> BTreeMap<u32, i32> {
        let mut regs = BTreeMap::new();
        regs.insert(0, DEFAULT_IS_DEFAULT_NAME); // 默认启用自定义主角名「李骏」(0=自定义名)
        regs.insert(410, 0); // 正式版，非试玩截断
        regs
    }

    /// 构建默认玩家配置寄存器表 (专门供 0x00E3 查询)
    pub fn default_config_regs() -> BTreeMap<u32, i32> {
        let mut regs = BTreeMap::new();
        regs.insert(20, 0); // 默认 H 场景选项：0=由选项决定 (_SelectEx)
        regs
    }

    /// 检测剧本是否采用标准 BGI 8字节定长 Push 指令格式（含 0x003F STMT_END）
    fn detect_standard_opcodes_active(script: &ActiveScript<'_>) -> bool {
        let base = script.base_offset();
        let data_len = script.len();
        let mut min_str_abs = data_len;
        let mut p = base;
        let limit = data_len.min(base + 8192);
        while p + 4 <= limit && p + 4 <= min_str_abs {
            let Ok(op) = script.read_u32(p) else {
                break;
            };
            if op == 0x003F {
                return true;
            }
            if op == 3 && p + 8 <= data_len {
                if let Ok(rel) = script.read_u32(p + 4) {
                    let abs = base + rel as usize;
                    if abs > p + 8 && abs < min_str_abs {
                        min_str_abs = abs;
                    }
                }
                p += 8;
            } else if OpCode(op).is_8byte_instruction() && p + 8 <= data_len {
                p += 8;
            } else {
                p += 4;
            }
        }
        false
    }

    fn from_active_script(script: ActiveScript<'a>) -> Self {
        let base = script.base_offset();
        let has_standard_opcodes = Self::detect_standard_opcodes_active(&script);
        Self {
            script,
            script_name: "<unknown>".to_string(),
            pc: base,
            state: VmState::Ready,
            has_standard_opcodes,
            eval_stack: Vec::with_capacity(64),
            script_loader: None,
            call_stack: Vec::new(),
            rng_state: DEFAULT_RNG_SEED,
            flags: BTreeMap::new(),
            entry_flags: BTreeMap::new(),
            sys_regs: Self::default_sys_regs(),
            sys_regs_dirty: false,
            config_regs: Self::default_config_regs(),
            unlocked_cgs: BTreeSet::new(),
            unlocked_cgs_dirty: false,
            choice_seq: 0,
            current_bg: None,
            current_bg_pose: None,
            pending_bg_motions: Vec::new(),
            current_bgm: None,
            current_voice: None,
            current_face: None,
            current_face_sprites: Vec::new(),
            current_face_offset: (-541, 451),
            slots: HashMap::new(),
            slot_transforms: HashMap::new(),
            pending_slot_motions: Vec::new(),
            slots_drawn_since_dialogue: HashSet::new(),
            face_drawn_since_dialogue: false,
            dialogue_counter: 0,
            script_dialogue_index: 0,
            script_choices: Vec::new(),
            pending_dialogue: None,
            pending_intro: None,
            history: Vec::new(),
            local_short_strings: BTreeMap::new(),
            spr_overlay: None,
            last_spr_fade_ms: 0,
            pending_se_plays: Vec::new(),
            hide_window_when_selecting: false,
            pending_local_label: None,
            pending_disabled_choices: BTreeSet::new(),
            pending_mail_on_notify: false,
        }
    }

    /// 初始化虚拟机并定位至剧本代码段入口（借用模式，兼容既有单元测试）
    pub fn new(script: &'a BgiScript<'a>) -> Self {
        Self::from_active_script(ActiveScript::Borrowed(script))
    }

    /// 从拥有所有权的剧本初始化虚拟机（零 unsafe，推荐用于跨脚本运行期与集成测试）
    pub fn from_owned(script: OwnedBgiScript) -> VmEngine<'static> {
        VmEngine::from_active_script(ActiveScript::Owned(Arc::new(script)))
    }

    /// 绑定跨脚本统一加载器
    pub fn set_script_loader(&mut self, loader: Arc<dyn ScriptLoader>) {
        self.script_loader = Some(loader);
    }

    /// 设置内置 PRNG 种子（传入 0 时自动使用默认非零种子，便于测试保确定性）
    pub fn set_rng_seed(&mut self, seed: u64) {
        self.rng_state = if seed == 0 { DEFAULT_RNG_SEED } else { seed };
    }

    /// 生成下一个伪随机 u64 (xorshift64)
    #[inline]
    fn next_random_u64(&mut self) -> u64 {
        let mut x = if self.rng_state == 0 {
            DEFAULT_RNG_SEED
        } else {
            self.rng_state
        };
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        x
    }

    /// 获取当前调用栈深度（含 0x00F0 跨脚本帧与 0x001A 页内本地帧）
    #[inline]
    pub fn call_depth(&self) -> usize {
        self.call_stack.len()
    }

    /// 获取当前跨脚本调用深度（仅统计 0x00F0 跨脚本帧，排除 0x001A 页内本地帧）
    #[inline]
    pub fn cross_script_call_depth(&self) -> usize {
        self.call_stack.iter().filter(|f| !f.is_local).count()
    }

    /// 导出可持久化调用栈快照（供 SaveStateV2 序列化）
    pub fn call_stack_save(&self) -> Vec<CallFrameSave> {
        self.call_stack
            .iter()
            .map(|f| CallFrameSave {
                script_name: f.script_name.clone(),
                return_pc: f.return_pc,
                is_local: f.is_local,
                script_dialogue_index: f.script_dialogue_index,
                entry_flags: f.entry_flags.clone(),
                script_choices: f.script_choices.clone(),
            })
            .collect()
    }

    /// 从 SaveStateV2 恢复调用栈（通过 script_loader 重新加载各父帧脚本并校验偏移）
    pub fn restore_call_stack(&mut self, frames: &[CallFrameSave]) -> Result<()> {
        if frames.len() > MAX_CALL_DEPTH {
            return Err(CoreError::InvalidFormat("存档调用栈深度超过上限 (MAX_CALL_DEPTH=16)"));
        }
        if frames.is_empty() {
            self.call_stack.clear();
            return Ok(());
        }
        let loader = self.script_loader.as_ref().ok_or_else(|| {
            CoreError::InvalidFormat("恢复调用栈需要已绑定 ScriptLoader")
        })?;
        let mut restored = Vec::with_capacity(frames.len());
        for frame in frames {
            let bytes = loader
                .load_script(&frame.script_name)?
                .ok_or_else(|| CoreError::FileNotFound(frame.script_name.clone()))?;
            let owned = Arc::new(OwnedBgiScript::parse(bytes)?);
            let active = ActiveScript::Owned(owned);
            let has_std = Self::detect_standard_opcodes_active(&active);
            if frame.return_pc < active.base_offset() || frame.return_pc > active.len() {
                return Err(CoreError::OutOfBounds);
            }
            restored.push(CallFrame {
                script: active,
                script_name: frame.script_name.clone(),
                return_pc: frame.return_pc,
                has_standard_opcodes: has_std,
                script_dialogue_index: frame.script_dialogue_index,
                entry_flags: frame.entry_flags.clone(),
                script_choices: frame.script_choices.clone(),
                is_local: frame.is_local,
            });
        }
        self.call_stack = restored;
        Ok(())
    }

    /// 获取当前活动脚本局部选择历史
    #[inline]
    pub fn script_choices(&self) -> &[usize] {
        &self.script_choices
    }

    /// 覆盖设置当前活动脚本局部选择历史（供读档精确恢复）
    pub fn set_script_choices(&mut self, choices: Vec<usize>) {
        self.script_choices = choices;
    }

    /// 检查并清除跨周目持久系统寄存器脏标志
    pub fn take_sys_regs_dirty(&mut self) -> bool {
        let d = self.sys_regs_dirty;
        self.sys_regs_dirty = false;
        d
    }

    /// 获取当前活动脚本内的对白计数
    #[inline]
    pub fn script_dialogue_index(&self) -> usize {
        self.script_dialogue_index
    }

    /// 设置全局对白计数器（供跨脚本读档对齐显示序号）
    pub fn set_dialogue_counter(&mut self, counter: usize) {
        self.dialogue_counter = counter;
    }

    /// 获取当前活动脚本入口时的 flags 快照
    #[inline]
    pub fn entry_flags(&self) -> &BTreeMap<u32, i32> {
        &self.entry_flags
    }

    /// 设置当前活动脚本入口时的 flags 快照（供 load_script_fresh 初始化）
    pub fn set_entry_flags(&mut self, flags: BTreeMap<u32, i32>) {
        self.entry_flags = flags;
    }

    /// 直接定位 PC（供读档精确恢复挂起点）
    pub fn seek_pc(&mut self, pc: usize) -> Result<()> {
        if pc < self.script.base_offset() || pc > self.script.len() {
            return Err(CoreError::OutOfBounds);
        }
        self.pc = pc;
        self.eval_stack.clear();
        Ok(())
    }

    /// 设置当前剧本名（用于诊断与异常日志定位）
    pub fn set_script_name(&mut self, name: impl Into<String>) {
        self.script_name = name.into();
    }

    #[inline]
    pub fn script_name(&self) -> &str {
        &self.script_name
    }

    /// 获取档内标志表
    #[inline]
    pub fn flags(&self) -> &BTreeMap<u32, i32> {
        &self.flags
    }

    /// 整体替换档内标志表（用于切脚本继承或读档恢复）
    pub fn set_flags(&mut self, flags: BTreeMap<u32, i32>) {
        self.flags = flags;
    }

    /// 写入单个档内标志
    pub fn set_flag(&mut self, id: u32, value: i32) {
        self.flags.insert(id, value);
    }

    /// 读取单个档内标志（默认 0）
    #[inline]
    pub fn get_flag(&self, id: u32) -> i32 {
        self.flags.get(&id).copied().unwrap_or(0)
    }

    /// 获取系统寄存器表
    #[inline]
    pub fn sys_regs(&self) -> &BTreeMap<u32, i32> {
        &self.sys_regs
    }

    /// 写入系统寄存器
    pub fn set_sys_reg(&mut self, id: u32, value: i32) {
        self.sys_regs.insert(id, value);
    }

    /// 整体替换系统寄存器
    pub fn set_sys_regs(&mut self, regs: BTreeMap<u32, i32>) {
        self.sys_regs = regs;
    }

    /// 获取玩家配置寄存器表 (0x00E3)
    #[inline]
    pub fn config_regs(&self) -> &BTreeMap<u32, i32> {
        &self.config_regs
    }

    /// 写入玩家配置寄存器
    pub fn set_config_reg(&mut self, id: u32, value: i32) {
        self.config_regs.insert(id, value);
    }

    /// 整体替换玩家配置寄存器
    pub fn set_config_regs(&mut self, regs: BTreeMap<u32, i32>) {
        self.config_regs = regs;
    }

    /// 专用接口：设置 H 场景射精选项 (0x00E3(20))，内建 0..=2 边界校验
    /// 0=由选项决定 (_SelectEx), 1=中出 (LABEL_IN), 2=外射 (LABEL_OUT)
    pub fn set_hscene_mode(&mut self, val: i32) -> bool {
        if !(0..=2).contains(&val) {
            return false;
        }
        self.config_regs.insert(20, val);
        true
    }

    /// 获取全局 CG 解锁集合
    #[inline]
    pub fn unlocked_cgs(&self) -> &BTreeSet<u32> {
        &self.unlocked_cgs
    }

    /// 注入全局 CG 解锁集合（初始化/跨脚本继承，不标记 dirty）
    pub fn set_unlocked_cgs(&mut self, cgs: BTreeSet<u32>) {
        self.unlocked_cgs = cgs;
        self.unlocked_cgs_dirty = false;
    }

    /// 检查并清除全局 CG 解锁脏标志
    pub fn take_unlocked_cgs_dirty(&mut self) -> bool {
        let d = self.unlocked_cgs_dirty;
        self.unlocked_cgs_dirty = false;
        d
    }

    /// 获取当前单调选项序列号
    #[inline]
    pub fn choice_seq(&self) -> u64 {
        self.choice_seq
    }

    /// 设置选项序列号基准（用于读档后递增基准防旧回调）
    pub fn set_choice_seq(&mut self, seq: u64) {
        self.choice_seq = seq;
    }

    /// 在选项画面直接恢复挂起态（不重跑 _SelectEx 防止重弹栈）
    pub fn restore_waiting_for_choice(
        &mut self,
        resume_pc: usize,
        prompt: Option<String>,
        options: Vec<String>,
        disabled_options: Vec<usize>,
        new_choice_seq: u64,
    ) {
        let is_custom_name = self
            .sys_regs
            .get(&0)
            .copied()
            .unwrap_or(DEFAULT_IS_DEFAULT_NAME)
            == 0;
        let prompt = prompt.map(|p| apply_protagonist_name(&p, is_custom_name));
        let options = options
            .into_iter()
            .map(|o| apply_protagonist_name(&o, is_custom_name))
            .collect();
        self.pc = resume_pc;
        self.eval_stack.clear();
        self.choice_seq = new_choice_seq;
        self.pending_dialogue = None;
        self.state = VmState::WaitingForChoice {
            prompt,
            options,
            disabled_options,
        };
    }

    /// 获取当前求值栈深度（用于单测断言栈平衡）
    #[inline]
    pub fn eval_stack_len(&self) -> usize {
        self.eval_stack.len()
    }

    /// 获取当前执行状态
    #[inline]
    pub fn state(&self) -> &VmState {
        &self.state
    }

    /// 获取当前程序计数器偏移 (PC)
    #[inline]
    pub fn pc(&self) -> usize {
        self.pc
    }

    /// 获取当前背景标识
    #[inline]
    pub fn current_bg(&self) -> Option<&str> {
        self.current_bg.as_deref()
    }

    /// 获取当前 BGM 标识
    #[inline]
    pub fn current_bgm(&self) -> Option<&str> {
        self.current_bgm.as_deref()
    }

    /// 获取当前暂存的语音 ID
    #[inline]
    pub fn current_voice(&self) -> Option<&str> {
        self.current_voice.as_deref()
    }

    /// 获取各立绘槽位状态
    #[inline]
    pub fn slots(&self) -> &HashMap<u32, CharacterSlot> {
        &self.slots
    }

    /// 获取各立绘槽位变换状态
    #[inline]
    pub fn slot_transforms(&self) -> &HashMap<u32, SlotTransform> {
        &self.slot_transforms
    }

    /// 获取当前半身头像差分部件表
    #[inline]
    pub fn current_face_sprites(&self) -> &[String] {
        &self.current_face_sprites
    }

    /// 获取当前半身头像偏移坐标
    #[inline]
    pub fn current_face_offset(&self) -> (i32, i32) {
        self.current_face_offset
    }

    /// 读档前注入舞台视觉底色快照（若后续脚本重放切换新 BG/BGM/立绘则自然覆盖，未切换则保留父帧视觉）
    pub fn seed_stage_visuals(
        &mut self,
        bg: Option<String>,
        bgm: Option<String>,
        slots: HashMap<u32, CharacterSlot>,
        slot_transforms: HashMap<u32, SlotTransform>,
        face: Option<String>,
        face_sprites: Vec<String>,
    ) {
        self.current_bg = bg;
        self.current_bgm = bgm;
        self.slots = slots;
        self.slot_transforms = slot_transforms;
        self.current_face = face;
        self.current_face_sprites = face_sprites;
    }

    /// 获取已处理的对白总句数
    #[inline]
    pub fn dialogue_counter(&self) -> usize {
        self.dialogue_counter
    }

    /// 获取当前等待确认的对白快照
    #[inline]
    pub fn pending_dialogue(&self) -> Option<&DialogueEvent> {
        self.pending_dialogue.as_ref()
    }

    /// 读取局部短字符串槽位 (%0 / %1)
    #[inline]
    pub fn local_short_string(&self, slot: u32) -> Option<&str> {
        self.local_short_strings.get(&slot).map(|s| s.as_str())
    }

    /// 展开参数中的局部短字符串占位符 (%0..%9)，返回 (展开后结果, 是否发生过替换)
    pub fn expand_local_short_string(&self, raw: &str) -> (String, bool) {
        if !raw.contains('%') || self.local_short_strings.is_empty() {
            return (raw.to_string(), false);
        }
        let mut out = String::with_capacity(raw.len());
        let bytes = raw.as_bytes();
        let mut i = 0;
        let mut expanded = false;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                let slot = (bytes[i + 1] - b'0') as u32;
                if let Some(val) = self.local_short_strings.get(&slot) {
                    out.push_str(val);
                    expanded = true;
                    i += 2;
                    continue;
                }
            }
            if let Some(ch) = raw[i..].chars().next() {
                out.push(ch);
                i += ch.len_utf8();
            } else {
                break;
            }
        }
        (out, expanded)
    }

    /// 判定当前是否处于短信/邮件 SPR 白名单上下文
    fn is_mail_spr_context(&self) -> bool {
        let is_mail_name = |name: &str| {
            let lower = name.trim().to_ascii_lowercase();
            matches!(
                lower.as_str(),
                "set_mail_rs" | "set_mail_received" | "set_mail_send" | "window_mail"
            ) || lower.starts_with("mail_chapter")
                || lower.starts_with("re_mail_chapter")
        };
        if is_mail_name(&self.script_name) {
            return true;
        }
        self.call_stack.iter().any(|f| is_mail_name(&f.script_name))
    }

    /// 获取当前独立 SPR 覆盖层状态（slot 0）
    #[inline]
    pub fn spr_overlay(&self) -> Option<&SprOverlayState> {
        self.spr_overlay.as_ref()
    }

    /// 直接设置独立 SPR 覆盖层状态（供读档补绘手机底图）
    pub fn set_spr_overlay(&mut self, overlay: Option<SprOverlayState>) {
        self.spr_overlay = overlay;
    }

    /// 获取最近一次 SPR 淡出/切换耗时 (ms)
    #[inline]
    pub fn last_spr_fade_ms(&self) -> u32 {
        self.last_spr_fade_ms
    }

    /// 设置最近一次 SPR 淡出耗时 (ms)
    pub fn set_last_spr_fade_ms(&mut self, ms: u32) {
        self.last_spr_fade_ms = ms;
    }

    /// 取出并清空本步推进期间触发的 _PlaySE 音效列表
    pub fn take_pending_se_plays(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_se_plays)
    }

    /// 清空本步推进期间暂存的 _PlaySE 音效列表（用于读档重放消音）
    pub fn clear_pending_se_plays(&mut self) {
        self.pending_se_plays.clear();
    }

    /// 获取 0x0167 (HideWindowWhenItemSelecting) 当前开关状态
    #[inline]
    pub fn hide_window_when_selecting(&self) -> bool {
        self.hide_window_when_selecting
    }

    /// 设置 0x0167 (HideWindowWhenItemSelecting) 状态（供读档恢复）
    pub fn set_hide_window_when_selecting(&mut self, hide: bool) {
        self.hide_window_when_selecting = hide;
    }

    /// 取出并清空 `_AnimateSPR(9, "mail_on_A", ...)` 触发的一次性提示气泡事件
    pub fn take_mail_on_notify(&mut self) -> bool {
        std::mem::replace(&mut self.pending_mail_on_notify, false)
    }

    /// 清空 `_AnimateSPR(9, "mail_on_A", ...)` 提示标志（读档重放期间防止旧气泡重放）
    pub fn clear_mail_on_notify(&mut self) {
        self.pending_mail_on_notify = false;
    }

    /// 读取当前待消费的 0x0166 禁用选项下标集合
    #[inline]
    pub fn pending_disabled_choices(&self) -> &BTreeSet<usize> {
        &self.pending_disabled_choices
    }

    /// 获取当前背景位姿
    #[inline]
    pub fn current_bg_pose(&self) -> Option<&BgPose> {
        self.current_bg_pose.as_ref()
    }

    /// 取出并清空本步累积的背景运镜列表
    pub fn take_pending_bg_motions(&mut self) -> Vec<BgMotion> {
        std::mem::take(&mut self.pending_bg_motions)
    }

    /// 当处于 WaitingForInput / WaitingForWaitClick / WaitingForChoice 状态时，恢复执行引擎以推进下一句
    pub fn resume(&mut self) {
        if self.state == VmState::WaitingForInput {
            self.state = VmState::Running;
            self.pending_dialogue = None;
        } else if self.state == VmState::WaitingForWaitClick {
            self.state = VmState::Running;
        } else if matches!(self.state, VmState::WaitingForChoice { .. }) {
            self.state = VmState::Running;
        }
    }

    /// 当处于 WaitingForChoice 状态时，带单调序号校验提交分支选择 (0-based)
    /// 校验通过后将结果写入 flags[0] 并恢复执行，防止连点两次、误触 0x0166 禁用项或读档前旧回调误消费下一次选项
    pub fn submit_choice(&mut self, choice_seq: u64, index: usize) -> bool {
        if choice_seq != self.choice_seq {
            return false;
        }
        if let VmState::WaitingForChoice {
            ref options,
            ref disabled_options,
            ..
        } = self.state
        {
            if index < options.len() && !disabled_options.contains(&index) {
                self.flags.insert(0, index as i32);
                self.script_choices.push(index);
                self.state = VmState::Running;
                return true;
            }
        }
        false
    }

    /// 当处于 WaitingForChoice 状态时，提交分支选择 (0-based) 并恢复执行引擎（兼容旧接口）
    pub fn select_choice(&mut self, index: usize) {
        let seq = self.choice_seq;
        let _ = self.submit_choice(seq, index);
    }

    /// 从求值栈按顺序弹出 n 个参数（返回按压栈顺序排列的 Vec，即 args[0] 最早压栈）
    fn pop_args(&mut self, count: usize) -> Vec<VmValue> {
        let mut args = Vec::with_capacity(count);
        for _ in 0..count {
            args.push(self.eval_stack.pop().unwrap_or(VmValue::Other));
        }
        args.reverse();
        args
    }

    /// 执行 0x003F + (op, sys_id) 系统调用分发（严禁误清 eval_stack）
    fn dispatch_syscall(&mut self, p_op: u32, p_id: u32) {
        let argc = p_op as usize;
        let args = self.pop_args(argc);
        match p_id {
            // 有返回值系统调用 (弹出 argc 参 -> 压入 1 个 VmValue::Int)
            0x00E1 => {
                // GetFlag(id)
                let id = args.first().and_then(|v| v.as_int()).unwrap_or(-1);
                let val = if id >= 0 {
                    self.flags.get(&(id as u32)).copied().unwrap_or(0)
                } else {
                    0
                };
                self.eval_stack.push(VmValue::Int(val));
            }
            0x00E3 => {
                // 查询玩家配置寄存器 (0xE3(20)=H场景射精选项，与 sys_regs 物理隔离)
                let id = args.first().and_then(|v| v.as_int()).unwrap_or(-1);
                let val = if id >= 0 {
                    self.config_regs.get(&(id as u32)).copied().unwrap_or(0)
                } else {
                    0
                };
                self.eval_stack.push(VmValue::Int(val));
            }
            0x00E5 | 0x00E7 => {
                // 读取系统只读/运行寄存器 (0xE5(0)=主角名, 0xE5(410)=正式版判定, 0xE7(50)=输入禁用判定等)
                let id = args.first().and_then(|v| v.as_int()).unwrap_or(-1);
                let val = if id >= 0 {
                    self.sys_regs.get(&(id as u32)).copied().unwrap_or(0)
                } else {
                    0
                };
                self.eval_stack.push(VmValue::Int(val));
            }
            // 无返回值 Void 系统调用 (弹出 argc 参 -> 严禁压栈)
            0x00E0 => {
                // SetFlag(id, value)
                let id = args.first().and_then(|v| v.as_int()).unwrap_or(-1);
                let val = args.get(1).and_then(|v| v.as_int()).unwrap_or(0);
                if id >= 0 {
                    self.flags.insert(id as u32, val);
                }
            }
            0x00E4 => {
                // SetSysReg(id, value) — 实证 chk_Tel 字节码：先压 id，再压 value，与 0x00E0 栈序完全一致
                let id = args.first().and_then(|v| v.as_int()).unwrap_or(-1);
                let val = args.get(1).and_then(|v| v.as_int()).unwrap_or(0);
                if id >= 0 {
                    let uid = id as u32;
                    let prev = self.sys_regs.insert(uid, val);
                    // 跨周目持久系统寄存器（如 chk_Tel 的 400..=409 已看电话记录）标脏落盘 system_flags.json；
                    // 410 为 chk_Tel 本轮瞬态标志（启动默认 0 表示正式版），不作为跨周目脏写覆盖默认值
                    if uid != 410 && prev != Some(val) {
                        self.sys_regs_dirty = true;
                    }
                }
            }
            0x00E6 => {
                // UnlockCG(id, val) -> 写入独立全局解锁集合
                let id = args.first().and_then(|v| v.as_int()).unwrap_or(-1);
                let val = args.get(1).and_then(|v| v.as_int()).unwrap_or(0);
                if id >= 0 {
                    let uid = id as u32;
                    if val != 0 {
                        if self.unlocked_cgs.insert(uid) {
                            self.unlocked_cgs_dirty = true;
                        }
                    } else if self.unlocked_cgs.remove(&uid) {
                        self.unlocked_cgs_dirty = true;
                    }
                }
            }
            0x0107 => {
                // 显式封口主角名系统寄存器 0 => DEFAULT_IS_DEFAULT_NAME (0=自定义名「李骏」)
                self.sys_regs.insert(0, DEFAULT_IS_DEFAULT_NAME);
            }
            0x0164 => {
                // 选项前置状态重置：清空上一级残留的 0x0166 禁用项（双保险清零第 1 处）
                self.pending_disabled_choices.clear();
            }
            0x0166 => {
                // DisableItem(idx)：实证 Mail_chapter111 传入 0-based 选项下标
                let idx = args.first().and_then(|v| v.as_int()).unwrap_or(-1);
                if idx >= 0 {
                    self.pending_disabled_choices.insert(idx as usize);
                }
            }
            0x0167 => {
                // HideWindowWhenItemSelecting(flag): 1=选项期间隐藏对话框并启用手机右侧避让排版，0=恢复正常
                let flag = args.first().and_then(|v| v.as_int()).unwrap_or(0);
                self.hide_window_when_selecting = flag != 0;
            }
            0x00F4 | 0x00FB | 0x00FE | 0x0108 | 0x0109 | 0x0112 | 0x0113 | 0x0119 | 0x011F
            | 0x0120 | 0x0124 | 0x0136 | 0x0137 | 0x0139 | 0x013A | 0x013F | 0x014A | 0x0150
            | 0x0151 | 0x0152 | 0x0153 | 0x0225 | 0x0226 | 0x02F8 => {
                // 实证 Void 系统调用：参数已按 p_op 精确弹出，无需额外动作
            }
            _ => {
                #[cfg(test)]
                panic!(
                    "[VmEngine] 未知 BGI sys_id: script={}, pc=0x{:x}, op={}, sys_id=0x{:04x} ({})",
                    self.script_name, self.pc, p_op, p_id, p_id
                );
                #[cfg(not(test))]
                eprintln!(
                    "[VmEngine WARNING] 未知 BGI sys_id: script={}, pc=0x{:x}, op={}, sys_id=0x{:04x} ({})",
                    self.script_name, self.pc, p_op, p_id, p_id
                );
            }
        }
    }

    /// 单步执行字节码流，若触发关键剧情事件则返回
    pub fn step(&mut self) -> Result<Option<VmEvent>> {
        if self.state == VmState::WaitingForInput {
            return Ok(self.pending_dialogue.clone().map(VmEvent::Dialogue));
        }
        if self.state == VmState::WaitingForWaitClick {
            return Ok(Some(VmEvent::WaitClick));
        }
        if let VmState::WaitingForChoice {
            ref prompt,
            ref options,
            ref disabled_options,
        } = self.state
        {
            return Ok(Some(VmEvent::Choice {
                prompt: prompt.clone(),
                options: options.clone(),
                disabled_options: disabled_options.clone(),
            }));
        }
        if self.state == VmState::Terminated {
            return Ok(None);
        }
        self.state = VmState::Running;

        let mut data_len = self.script.len();
        let mut base = self.script.base_offset();
        let mut ops_executed: usize = 0;
        let mut trace: VecDeque<(usize, u32, Option<VmValue>)> = VecDeque::with_capacity(20);

        while self.pc + 4 <= data_len {
            let val = self.script.read_u32(self.pc)?;
            let op = OpCode(val);

            if ops_executed >= MAX_OPS_PER_TICK {
                let trace_str = trace
                    .iter()
                    .map(|(tpc, top, st)| format!("(pc=0x{:x}, op=0x{:04x}, top={:?})", tpc, top, st))
                    .collect::<Vec<_>>()
                    .join(" -> ");
                return Err(CoreError::VmError(format!(
                    "VM 单次步进指令数超过熔断上限 {} (疑似死循环, script={}, pc=0x{:x}), 最近 {} 步轨迹: [{}]",
                    MAX_OPS_PER_TICK,
                    self.script_name,
                    self.pc,
                    trace.len(),
                    trace_str
                )));
            }
            ops_executed += 1;
            if trace.len() == 20 {
                trace.pop_front();
            }
            trace.push_back((self.pc, val, self.eval_stack.last().cloned()));

            if self.has_standard_opcodes {
                match val {
                    // 1. 压栈指令 (8 字节定长：4B opcode + 4B inline arg)
                    0x0000 => {
                        if self.pc + 8 > data_len {
                            return Err(CoreError::OutOfBounds);
                        }
                        let arg = self.script.read_i32(self.pc + 4)?;
                        self.eval_stack.push(VmValue::Int(arg));
                        self.pc += 8;
                        continue;
                    }
                    0x0001 => {
                        if self.pc + 8 > data_len {
                            return Err(CoreError::OutOfBounds);
                        }
                        let arg = self.script.read_u32(self.pc + 4)?;
                        self.eval_stack.push(VmValue::Addr(arg));
                        self.pc += 8;
                        continue;
                    }
                    0x0002 => {
                        if self.pc + 8 > data_len {
                            return Err(CoreError::OutOfBounds);
                        }
                        let arg = self.script.read_i32(self.pc + 4)?;
                        self.eval_stack.push(VmValue::Int(arg));
                        self.pc += 8;
                        continue;
                    }
                    0x0003 => {
                        if self.pc + 8 > data_len {
                            return Err(CoreError::OutOfBounds);
                        }
                        let arg = self.script.read_u32(self.pc + 4)?;
                        self.eval_stack.push(VmValue::StrOffset(arg));
                        self.pc += 8;
                        continue;
                    }
                    0x0009 => {
                        // 8 字节存储指令（实证仅在 _SelectEx 每个选项后出现且 arg==2）
                        // 消耗其前方的 0x0002 4 地址槽并保留选项字符串在操作数栈上
                        if self.pc + 8 > data_len {
                            return Err(CoreError::OutOfBounds);
                        }
                        let _arg = self.script.read_u32(self.pc + 4)?;
                        if let (Some(v), Some(_addr)) =
                            (self.eval_stack.pop(), self.eval_stack.pop())
                        {
                            self.eval_stack.push(v);
                        }
                        self.pc += 8;
                        continue;
                    }
                    // 2. 栈帧保护指令 (4 字节)：0x0010 压入占位 Int(0)，0x0011 弹出 1 个栈顶
                    0x0010 => {
                        self.eval_stack.push(VmValue::Int(0));
                        self.pc += 4;
                        continue;
                    }
                    0x0011 => {
                        self.eval_stack.pop();
                        self.pc += 4;
                        continue;
                    }
                    // 3. 脚本入口标签表分派 0x0016 (4B) 与标签条件跳转 0x0017 (8B)
                    0x0016 => {
                        let table_rel = self.eval_stack.pop().and_then(|v| v.as_addr());
                        if self.pending_local_label.is_some() {
                            self.eval_stack.clear();
                            if let Some(rel) = table_rel {
                                let target_pc = base
                                    .checked_add(rel as usize)
                                    .filter(|&t| t >= base && t + 4 <= data_len)
                                    .ok_or_else(|| {
                                        CoreError::VmError(format!(
                                            "0x0016 标签表跳转越界: script={}, pc=0x{:x}, table_rel=0x{:x}",
                                            self.script_name, self.pc, rel
                                        ))
                                    })?;
                                self.pc = target_pc;
                                continue;
                            }
                        }
                        self.pc += 4;
                        continue;
                    }
                    0x0017 => {
                        if self.pc + 8 > data_len {
                            return Err(CoreError::OutOfBounds);
                        }
                        let cond_type = self.script.read_u32(self.pc + 4)?;
                        let target_rel = self.eval_stack.pop().and_then(|v| v.as_addr());
                        let cond = self.eval_stack.pop().and_then(|v| v.as_int());
                        // 每次标签匹配判定后严格清栈，杜绝多封连读时 eval_stack 漏值
                        self.eval_stack.clear();
                        let should_jump = match cond {
                            Some(c) => {
                                if cond_type != 0 {
                                    c != 0
                                } else {
                                    c == 0
                                }
                            }
                            None => false,
                        };
                        if should_jump {
                            if let Some(rel) = target_rel {
                                let target_pc = base
                                    .checked_add(rel as usize)
                                    .filter(|&t| t >= base && t + 4 <= data_len)
                                    .ok_or_else(|| {
                                        CoreError::VmError(format!(
                                            "0x0017 标签跳转越界: script={}, pc=0x{:x}, target_rel=0x{:x}",
                                            self.script_name, self.pc, rel
                                        ))
                                    })?;
                                self.pending_local_label = None;
                                self.pc = target_pc;
                                continue;
                            }
                        }
                        self.pc += 8;
                        continue;
                    }
                    // 4. 无条件跳转 0x0018 (JMP, 4B): 弹 target_rel，跳转至 base + target_rel 并清空 eval_stack
                    0x0018 => {
                        let target_rel = self.eval_stack.pop().and_then(|v| v.as_addr());
                        self.eval_stack.clear();
                        if let Some(rel) = target_rel {
                            let target_pc = base
                                .checked_add(rel as usize)
                                .filter(|&t| t >= base && t + 4 <= data_len)
                                .ok_or_else(|| {
                                    CoreError::VmError(format!(
                                        "JMP 跳转越界: script={}, pc=0x{:x}, target_rel=0x{:x}",
                                        self.script_name, self.pc, rel
                                    ))
                                })?;
                            self.pc = target_pc;
                            continue;
                        }
                        return Err(CoreError::VmError(format!(
                            "JMP 缺少有效跳转目标: script={}, pc=0x{:x}",
                            self.script_name, self.pc
                        )));
                    }
                    // 5. 条件跳转 0x0019 (JZ, 8B): 先弹 target_rel，再弹 cond；cond == 0 时跳至 base + target_rel
                    0x0019 => {
                        if self.pc + 8 > data_len {
                            return Err(CoreError::OutOfBounds);
                        }
                        let _cond_type = self.script.read_u32(self.pc + 4)?;
                        let target_rel = self.eval_stack.pop().and_then(|v| v.as_addr());
                        let cond = self.eval_stack.pop().and_then(|v| v.as_int());
                        if !self.eval_stack.is_empty() {
                            self.eval_stack.clear();
                        }
                        match (cond, target_rel) {
                            (Some(0), Some(rel)) => {
                                let target_pc = base
                                    .checked_add(rel as usize)
                                    .filter(|&t| t >= base && t + 4 <= data_len)
                                    .ok_or_else(|| {
                                        CoreError::VmError(format!(
                                            "JZ 跳转越界: script={}, pc=0x{:x}, target_rel=0x{:x}",
                                            self.script_name, self.pc, rel
                                        ))
                                    })?;
                                self.pc = target_pc;
                                continue;
                            }
                            (Some(_), Some(_)) => {
                                self.pc += 8;
                                continue;
                            }
                            _ => {
                                return Err(CoreError::VmError(format!(
                                    "JZ 栈操作数缺失或类型非法: script={}, pc=0x{:x}, cond={:?}, target={:?}",
                                    self.script_name, self.pc, cond, target_rel
                                )));
                            }
                        }
                    }
                    // 5b. 页内本地标签调用 0x001A (4B)：
                    // 实证字节码流：0x0003 STR 'MAIL_XX' -> 0x00F6 -> 0x0001 target_rel(0x0) -> 0x001A (4B)
                    // 压入 is_local=true 的 CallFrame（return_pc = pc + 4，共享 MAX_CALL_DEPTH=16 熔断），
                    // 提取目标标签名存入 pending_local_label 并严格清空 eval_stack 后跳转至 base + target_rel。
                    0x001A => {
                        if self.call_stack.len() >= MAX_CALL_DEPTH {
                            return Err(CoreError::InvalidFormat(
                                "子脚本/页内本地调用深度超过上限 (MAX_CALL_DEPTH=16)",
                            ));
                        }
                        let target_rel = self.eval_stack.pop().and_then(|v| v.as_addr());
                        let label_name = self
                            .eval_stack
                            .iter()
                            .rev()
                            .find_map(|v| match *v {
                                VmValue::StrOffset(off) => self.script.get_string(off).ok(),
                                _ => None,
                            });
                        self.eval_stack.clear();
                        if let Some(rel) = target_rel {
                            let target_pc = base
                                .checked_add(rel as usize)
                                .filter(|&t| t >= base && t + 4 <= data_len)
                                .ok_or_else(|| {
                                    CoreError::VmError(format!(
                                        "0x001A 页内调用跳转越界: script={}, pc=0x{:x}, target_rel=0x{:x}",
                                        self.script_name, self.pc, rel
                                    ))
                                })?;
                            self.pending_local_label = label_name;
                            self.call_stack.push(CallFrame {
                                script: self.script.clone(),
                                script_name: self.script_name.clone(),
                                return_pc: self.pc + 4,
                                has_standard_opcodes: self.has_standard_opcodes,
                                script_dialogue_index: self.script_dialogue_index,
                                entry_flags: BTreeMap::new(),
                                script_choices: Vec::new(),
                                is_local: true,
                            });
                            self.pc = target_pc;
                            continue;
                        }
                        self.pc += 4;
                        continue;
                    }
                    // 6. 返回/停机指令 (0x001B RET / 0x00F4 END_SCRIPT)：
                    //    call_stack 非空时弹栈恢复父帧继续执行（本地帧仅恢复 pc，跨脚本帧恢复完整上下文并归并 script_choices）；
                    //    为空时置 Terminated 防扫穿末尾字符串池。
                    0x001B | 0x00F4 => {
                        self.eval_stack.clear();
                        if let Some(frame) = self.call_stack.pop() {
                            if frame.is_local {
                                self.pc = frame.return_pc;
                                continue;
                            }
                            let child_choices = std::mem::take(&mut self.script_choices);
                            self.script = frame.script;
                            self.script_name = frame.script_name;
                            self.has_standard_opcodes = frame.has_standard_opcodes;
                            self.script_dialogue_index = frame.script_dialogue_index;
                            self.entry_flags = frame.entry_flags;
                            let mut merged_choices = frame.script_choices;
                            merged_choices.extend(child_choices);
                            self.script_choices = merged_choices;
                            self.pc = frame.return_pc;
                            data_len = self.script.len();
                            base = self.script.base_offset();
                            continue;
                        }
                        self.state = VmState::Terminated;
                        return Ok(None);
                    }
                    // 7. 算术与比较/逻辑二元运算（先弹 rhs，再弹 lhs）
                    0x0020..=0x0023 => {
                        let rhs = self.eval_stack.pop();
                        let lhs = self.eval_stack.pop();
                        let res = match (lhs, rhs) {
                            (Some(VmValue::Int(a)), Some(VmValue::Int(b))) => match val {
                                0x0020 => VmValue::Int(a.wrapping_add(b)),
                                0x0021 => VmValue::Int(a.wrapping_sub(b)),
                                0x0022 => VmValue::Int(a.wrapping_mul(b)),
                                0x0023 => VmValue::Int(if b != 0 { a.wrapping_div(b) } else { 0 }),
                                _ => VmValue::Other,
                            },
                            _ => VmValue::Other,
                        };
                        self.eval_stack.push(res);
                        self.pc += 4;
                        continue;
                    }
                    0x0030 | 0x0033 | 0x0038 => {
                        let rhs = self.eval_stack.pop().and_then(|v| v.as_int());
                        let lhs = self.eval_stack.pop().and_then(|v| v.as_int());
                        let res = match (lhs, rhs) {
                            (Some(a), Some(b)) => match val {
                                0x0030 => VmValue::Int(if a == b { 1 } else { 0 }),
                                0x0033 => VmValue::Int(if a >= b { 1 } else { 0 }),
                                0x0038 => VmValue::Int(if a != 0 && b != 0 { 1 } else { 0 }),
                                _ => VmValue::Other,
                            },
                            _ => VmValue::Other,
                        };
                        self.eval_stack.push(res);
                        self.pc += 4;
                        continue;
                    }
                    // 8. 0x003F 系统调用前缀（前瞻 0..=2 + 224..=760 覆盖 0x02F8，严禁清栈）或孤立 stmt_end 兼容
                    0x003F => {
                        if self.pc + 12 <= data_len {
                            let p_op = self.script.read_u32(self.pc + 4)?;
                            let p_id = self.script.read_u32(self.pc + 8)?;
                            if p_op <= 2 && (224..=760).contains(&p_id) {
                                self.dispatch_syscall(p_op, p_id);
                                self.pc += 12;
                                continue;
                            }
                        }
                        // 孤立 0x003F（兼容无前缀合成单元测试）
                        self.eval_stack.clear();
                        self.pc += 4;
                        continue;
                    }
                    // 9. 用户函数调用 (0x001C)、子脚本调用 (0x00F0)、尾跳转 (0x00F3)、对白 (0x0140/0x0143) 在下方公共分支处理
                    0x001C | 0x00F0 | 0x00F3 | 0x0140 | 0x0143 => {}
                    // 10. 标签表比对 0x00F7 (LABEL_CMP) 与标签表结束 0x00F9 (LABEL_END)
                    0x00F7 => {
                        let cand = self.eval_stack.pop().and_then(|v| match v {
                            VmValue::StrOffset(off) => self.script.get_string(off).ok(),
                            _ => None,
                        });
                        let matched = match (cand.as_deref(), self.pending_local_label.as_deref()) {
                            (Some(c), Some(target)) => c.eq_ignore_ascii_case(target),
                            _ => false,
                        };
                        self.eval_stack.push(VmValue::Int(if matched { 1 } else { 0 }));
                        self.pc += 4;
                        continue;
                    }
                    0x00F9 => {
                        self.pending_local_label = None;
                        self.eval_stack.clear();
                        self.pc += 4;
                        continue;
                    }
                    0x00F6 | 0x0160 => {
                        self.pc += 4;
                        continue;
                    }
                    // 11. 未知操作码：#[cfg(test)] 直接 panic! 暴露，运行期打警告日志
                    _ => {
                        #[cfg(test)]
                        panic!(
                            "[VmEngine] 未知 BGI opcode: script={}, pc=0x{:x}, opcode=0x{:04x}",
                            self.script_name, self.pc, val
                        );
                        #[cfg(not(test))]
                        {
                            eprintln!(
                                "[VmEngine WARNING] 未知 BGI opcode: script={}, pc=0x{:x}, opcode=0x{:04x}",
                                self.script_name, self.pc, val
                            );
                            self.pc += 4;
                            continue;
                        }
                    }
                }
            } else if op.is_push_string() {
                // 合成/精简测试格式：仅 0x0003 字符串压栈占 8 字节
                if self.pc + 8 > data_len {
                    return Err(CoreError::OutOfBounds);
                }
                self.pc += 8;
                continue;
            }

            // 2. 用户函数调用指令 (0x001C)
            if op.is_call_func() {
                let event = self.handle_user_function_call(base)?;
                self.eval_stack.clear();
                self.pc += 4;
                if let Some(evt) = event {
                    return Ok(Some(evt));
                }
                continue;
            }

            // 3. 子脚本调用指令 (0x00F0) - 保护 introduction_* 原生拦截，其余真实压栈跳转子脚本
            if op == OpCode::CALL_SUBSCRIPT {
                let (event, switched) = self.handle_subscript_call(base)?;
                self.eval_stack.clear();
                if switched {
                    data_len = self.script.len();
                    base = self.script.base_offset();
                } else {
                    self.pc += 4;
                }
                if let Some(evt) = event {
                    return Ok(Some(evt));
                }
                continue;
            }

            // 3b. 不可返回尾跳转指令 (0x00F3) - 清空 call_stack，保留舞台与寄存器，直接切换目标脚本
            if op == OpCode::CALL_SCRIPT {
                let switched = self.handle_tail_script_jump(base)?;
                self.eval_stack.clear();
                if switched {
                    data_len = self.script.len();
                    base = self.script.base_offset();
                } else {
                    self.pc += 4;
                }
                continue;
            }

            // 4. 核心对白渲染指令 (0x0140 或 0x0143)
            if op.is_dialogue() {
                if let Some(dialogue) = self.handle_dialogue(base)? {
                    self.eval_stack.clear();
                    self.pc += 4;
                    self.state = VmState::WaitingForInput;
                    self.pending_dialogue = Some(dialogue.clone());
                    return Ok(Some(VmEvent::Dialogue(dialogue)));
                } else {
                    // 无有效台词文本（仅为内部同步/视听标记），直接递进不挂起断点
                    self.eval_stack.clear();
                    self.pc += 4;
                    continue;
                }
            }

            // 5. 其余普通指令字（非标准合成格式），PC 递进 4 字节
            self.pc += 4;
        }

        self.state = VmState::Terminated;
        Ok(None)
    }


    /// 流式运行至下一句对白 (自动跨越所有中间音画准备指令，遇到分支选择、_Wait(-1, 1) 点击翻页或剧本结束时暂停并返回 None)
    pub fn next_dialogue(&mut self) -> Result<Option<DialogueEvent>> {
        if self.state == VmState::WaitingForInput || self.state == VmState::WaitingForWaitClick {
            self.resume();
        }

        let mut events_seen: usize = 0;
        while self.state != VmState::Terminated {
            // 若遇到分支选择或点击翻页挂起，停止自动推进，等待外部决策/点击
            if matches!(
                self.state,
                VmState::WaitingForChoice { .. } | VmState::WaitingForWaitClick
            ) {
                return Ok(None);
            }

            if let Some(event) = self.step()? {
                events_seen += 1;
                if events_seen > MAX_EVENTS_PER_DIALOGUE {
                    return Err(CoreError::VmError(format!(
                        "单句对白推进跨事件数超过上限 {} (疑似事件死循环, script={}, pc=0x{:x})",
                        MAX_EVENTS_PER_DIALOGUE, self.script_name, self.pc
                    )));
                }
                match event {
                    VmEvent::Dialogue(dlg) => return Ok(Some(dlg)),
                    VmEvent::Choice { .. } | VmEvent::WaitClick => return Ok(None),
                    _ => {}
                }
            }
        }

        Ok(None)
    }

    /// 解码 BGI 坐标标记：
    /// - 0x1000_0000 (268435456) 表示 KEEP（保持当前值）
    /// - 0x1000_0000 | (i28 & 0x0FFF_FFFF) 表示带相对/目标标志的 28 位有符号数
    /// - 267_000_000..=269_999_999 (如 268435456 + (-920)) 表示带基数偏移的编码
    fn decode_bgi_flagged_int(raw: i32) -> (bool, Option<i32>) {
        if raw == 0x1000_0000 {
            return (true, None);
        }
        let u = raw as u32;
        if (u & 0xF000_0000) == 0x1000_0000 {
            let v28 = u & 0x0FFF_FFFF;
            let signed = if (v28 & 0x0800_0000) != 0 {
                (v28 | 0xF000_0000) as i32
            } else {
                v28 as i32
            };
            return (true, Some(signed));
        }
        if (267_000_000..=269_999_999).contains(&raw) {
            return (true, Some(raw - 0x1000_0000));
        }
        (false, Some(raw))
    }

    /// 检查场景背景名是否有效（兼容 white/black、pc_bg_*、_pc_bg_*、eye_scene_*、事件 CG 等全部资产名）
    fn is_valid_scene_arg(s: &str) -> bool {
        let lower = s.trim().to_ascii_lowercase();
        !lower.is_empty()
            && lower != "_drawscene"
            && !lower.ends_with(".bs5")
            && !lower.ends_with(".h")
            && !lower.starts_with("effect")
            && !lower.starts_with("printmessage")
            && !lower.starts_with("introduction_")
    }

    /// 检查立绘部件名是否有效（过滤脚本名、特效关键字与函数名）
    fn is_valid_sprite_arg(s: &str) -> bool {
        !s.is_empty()
            && s != "_DrawBS"
            && !s.ends_with(".bs5")
            && !s.ends_with(".h")
            && !s.starts_with("EFFECT")
            && !s.starts_with("PrintMessage")
            && !s.starts_with("introduction_")
    }

    /// 从当前求值栈读取指定索引的整型参数
    fn stack_int(args: &[VmValue], idx: usize) -> Option<i32> {
        match args.get(idx) {
            Some(VmValue::Int(v)) => Some(*v),
            _ => None,
        }
    }

    /// 从当前求值栈读取指定索引的字符串参数
    fn stack_str(&self, args: &[VmValue], idx: usize) -> Option<String> {
        match args.get(idx) {
            Some(VmValue::StrOffset(off)) => self.script.get_string(*off).ok(),
            _ => None,
        }
    }

    /// 处理用户函数调用 (Opcode 0x001C)
    fn handle_user_function_call(&mut self, base: usize) -> Result<Option<VmEvent>> {
        // 标准栈模式下，栈顶元素通常就是所调用的函数名字符串
        let (fn_name, stack_args_len) = if self.has_standard_opcodes && !self.eval_stack.is_empty() {
            if let Some(VmValue::StrOffset(off)) = self.eval_stack.last() {
                (
                    Some(self.script.get_string(*off)?),
                    self.eval_stack.len().saturating_sub(1),
                )
            } else {
                (self.find_prev_string(self.pc, 16, base)?, 0)
            }
        } else {
            (self.find_prev_string(self.pc, 16, base)?, 0)
        };

        let use_stack = self.has_standard_opcodes && stack_args_len > 0;

        match fn_name.as_deref() {
            Some("_PlayVoice") => {
                let mut voice_id = None;
                if use_stack {
                    for idx in (0..stack_args_len).rev() {
                        if let Some(s) = self.stack_str(&self.eval_stack[..stack_args_len], idx) {
                            if !s.is_empty()
                                && s != "_PlayVoice"
                                && !s.ends_with(".bs5")
                                && !s.ends_with(".h")
                                && !s.starts_with("introduction_")
                            {
                                voice_id = Some(s);
                                break;
                            }
                        }
                    }
                }
                if voice_id.is_none() {
                    voice_id = self.find_prev_voice_string(self.pc, 120, base)?;
                }
                if let Some(voice_id) = voice_id {
                    self.current_voice = Some(voice_id.clone());
                    return Ok(Some(VmEvent::VoicePlay { voice: voice_id }));
                }
            }
            Some("_PlayBGM") => {
                let mut bgm_id = None;
                if use_stack {
                    for idx in 0..stack_args_len {
                        if let Some(s) = self.stack_str(&self.eval_stack[..stack_args_len], idx) {
                            if s.starts_with("PC_BGM_") {
                                bgm_id = Some(s);
                                break;
                            }
                        }
                    }
                }
                if bgm_id.is_none() {
                    bgm_id = self.find_prev_bgm_string(self.pc, 120, base)?;
                }
                if let Some(bgm_id) = bgm_id {
                    self.current_bgm = Some(bgm_id.clone());
                    return Ok(Some(VmEvent::BgmPlay { bgm: bgm_id }));
                }
            }
            Some("_DrawScene") => {
                let mut scene_name = None;
                let mut pose = BgPose {
                    x: None,
                    y: None,
                    scale: 100,
                    transition_ms: 1000,
                };
                if use_stack {
                    let args = &self.eval_stack[..stack_args_len];
                    for idx in 0..args.len() {
                        if let Some(s) = self.stack_str(args, idx) {
                            let (expanded, _) = self.expand_local_short_string(&s);
                            if Self::is_valid_scene_arg(&expanded) && !expanded.starts_with('%') {
                                scene_name = Some(expanded);
                                break;
                            }
                        }
                    }
                    if args.len() >= 14 {
                        if let Some(raw_x) = Self::stack_int(args, 4) {
                            pose.x = if raw_x == 32767 {
                                None
                            } else {
                                Self::decode_bgi_flagged_int(raw_x).1
                            };
                        }
                        if let Some(raw_y) = Self::stack_int(args, 5) {
                            pose.y = if raw_y == 32767 {
                                None
                            } else {
                                Self::decode_bgi_flagged_int(raw_y).1
                            };
                        }
                        if let Some(sc) = Self::stack_int(args, 6) {
                            if sc > 0 && sc != 32767 && sc != 0x1000_0000 {
                                pose.scale = sc;
                            }
                        }
                        if let Some(tms) = Self::stack_int(args, 13) {
                            pose.transition_ms = tms.max(0) as u32;
                        }
                    }
                }
                if scene_name.is_none() {
                    scene_name = self.find_prev_scene_string(self.pc, 250, base)?;
                }
                if let Some(scene_name) = scene_name {
                    self.current_bg = Some(scene_name.clone());
                    self.current_bg_pose = Some(pose);
                    self.pending_bg_motions.clear();
                    // BGI _DrawScene 切换背景时，清空上一幕遗留的舞台立绘与半身头像（保留同批次 _DrawScene 前刚预置的槽位）
                    let keep = &self.slots_drawn_since_dialogue;
                    self.slots.retain(|sid, _| keep.contains(sid));
                    self.slot_transforms.retain(|sid, _| keep.contains(sid));
                    self.pending_slot_motions.retain(|m| keep.contains(&m.slot_id));
                    if !self.face_drawn_since_dialogue {
                        self.current_face = None;
                        self.current_face_sprites.clear();
                    }
                    // _DrawScene 切换大场景时一律强制清空独立 SprOverlay（防止手机等覆盖层跨场景残留）
                    self.spr_overlay = None;
                    self.last_spr_fade_ms = 0;
                    return Ok(Some(VmEvent::SceneChange { bg: scene_name }));
                }
            }
            Some("_MoveSight") => {
                if use_stack && stack_args_len >= 4 {
                    let args = &self.eval_stack[..stack_args_len];
                    let raw_x = Self::stack_int(args, 0).unwrap_or(0x1000_0000);
                    let raw_y = Self::stack_int(args, 1).unwrap_or(0x1000_0000);
                    let raw_sc = Self::stack_int(args, 2).unwrap_or(0x1000_0000);
                    let dur = Self::stack_int(args, 3).unwrap_or(0).max(0) as u32;

                    let (_, tx) = Self::decode_bgi_flagged_int(raw_x);
                    let (_, ty) = Self::decode_bgi_flagged_int(raw_y);
                    let target_scale = if raw_sc == 0x1000_0000 || raw_sc == 32767 {
                        None
                    } else {
                        Some(raw_sc)
                    };
                    self.pending_bg_motions.push(BgMotion {
                        target_x: tx,
                        target_y: ty,
                        keep_x: tx.is_none(),
                        keep_y: ty.is_none(),
                        target_scale,
                        duration_ms: dur,
                    });
                }
            }
            Some("_ScrollBG") => {
                if use_stack && stack_args_len >= 6 {
                    let args = &self.eval_stack[..stack_args_len];
                    let raw_x = Self::stack_int(args, 0).unwrap_or(0x1000_0000);
                    let raw_y = Self::stack_int(args, 1).unwrap_or(0x1000_0000);
                    let raw_sc = Self::stack_int(args, 2).unwrap_or(0x1000_0000);
                    let dur = Self::stack_int(args, 5).unwrap_or(0).max(0) as u32;

                    let (_, tx) = Self::decode_bgi_flagged_int(raw_x);
                    let (_, ty) = Self::decode_bgi_flagged_int(raw_y);
                    let target_scale = if raw_sc == 0x1000_0000 || raw_sc == 32767 {
                        None
                    } else {
                        Some(raw_sc)
                    };
                    self.pending_bg_motions.push(BgMotion {
                        target_x: tx,
                        target_y: ty,
                        keep_x: tx.is_none(),
                        keep_y: ty.is_none(),
                        target_scale,
                        duration_ms: dur,
                    });
                }
            }
            Some("_DrawBS") => {
                let (slot_id, sprites, x_opt, y_opt, scale_opt, fade_ms) = if use_stack
                    && stack_args_len >= 13
                {
                    let args = &self.eval_stack[..stack_args_len];
                    let sid = Self::stack_int(args, 0).unwrap_or(1).max(0) as u32;
                    let mut list = Vec::new();
                    for idx in 1..args.len() {
                        if let Some(s) = self.stack_str(args, idx) {
                            if Self::is_valid_sprite_arg(&s) {
                                list.push(s);
                            }
                        }
                    }
                    let rx = Self::stack_int(args, 6).unwrap_or(0x1000_0000);
                    let ry = Self::stack_int(args, 7).unwrap_or(0x1000_0000);
                    let rsc = Self::stack_int(args, 12).unwrap_or(0x1000_0000);
                    let x_dec = if rx == 0x1000_0000 || rx == 32767 {
                        None
                    } else {
                        Self::decode_bgi_flagged_int(rx).1
                    };
                    let y_dec = if ry == 0x1000_0000 || ry == 32767 {
                        None
                    } else {
                        Self::decode_bgi_flagged_int(ry).1
                    };
                    let sc_dec = if rsc == 0x1000_0000 || rsc == 32767 {
                        None
                    } else {
                        Some(rsc)
                    };
                    let mut fms = 200u32;
                    for pos in 18..args.len().min(30) {
                        if Self::stack_int(args, pos) == Some(-256) && pos >= 2 {
                            if let Some(v) = Self::stack_int(args, pos - 2) {
                                if (0..=5000).contains(&v) {
                                    fms = v as u32;
                                }
                            }
                            break;
                        }
                    }
                    (sid, list, x_dec, y_dec, sc_dec, fms)
                } else {
                    let sprites = self.find_prev_sprites(self.pc, 200, base)?;
                    let slot_id = self.find_prev_slot_id(self.pc, base)?.unwrap_or(1);
                    (slot_id, sprites, None, None, None, 200)
                };

                // 严格隔离半身头像槽位 (slot 9 / 10) 与中央舞台槽位，杜绝 MO01PC_F 污染 Slot1
                let is_face_slot = slot_id == 9
                    || slot_id == 10
                    || (sprites
                        .iter()
                        .any(|s| s.ends_with("_F") || s.contains("_F0") || s.contains("_F1")));

                if is_face_slot {
                    self.face_drawn_since_dialogue = true;
                    self.current_face_sprites = sprites.clone();
                    self.current_face = sprites
                        .iter()
                        .skip(1)
                        .find(|s| s.contains("_F") || s.contains("_N") || s.contains("_L"))
                        .or_else(|| sprites.first())
                        .cloned();
                    if let Some(fx) = x_opt {
                        self.current_face_offset.0 = fx;
                    }
                    if let Some(fy) = y_opt {
                        self.current_face_offset.1 = fy;
                    }
                } else {
                    self.slots_drawn_since_dialogue.insert(slot_id);
                    let prev_tr = self.slot_transforms.get(&slot_id).cloned().unwrap_or(SlotTransform {
                        x: 0,
                        y: 200,
                        scale: 100,
                        fade_ms,
                    });
                    let next_tr = SlotTransform {
                        x: x_opt.unwrap_or(prev_tr.x),
                        y: y_opt.unwrap_or(prev_tr.y),
                        scale: scale_opt.unwrap_or(prev_tr.scale),
                        fade_ms,
                    };
                    self.slot_transforms.insert(slot_id, next_tr);
                    self.slots.insert(
                        slot_id,
                        CharacterSlot {
                            slot_id,
                            sprites: sprites.clone(),
                        },
                    );
                }

                return Ok(Some(VmEvent::SpriteDraw {
                    slot: slot_id,
                    sprites,
                }));
            }
            Some("_MoveBS") => {
                if use_stack && stack_args_len >= 13 {
                    let args = &self.eval_stack[..stack_args_len];
                    let sid = Self::stack_int(args, 0).unwrap_or(1).max(0) as u32;
                    let raw_x = Self::stack_int(args, 4).unwrap_or(0x1000_0000);
                    let raw_y = Self::stack_int(args, 5).unwrap_or(0x1000_0000);
                    // _MoveBS 耗时位于 -256 哨兵前 2 位（标准索引 12），args[13] 为 0
                    let mut dur = Self::stack_int(args, 12).unwrap_or(100).max(0) as u32;
                    for pos in 13..args.len().min(20) {
                        if Self::stack_int(args, pos) == Some(-256) && pos >= 2 {
                            if let Some(v) = Self::stack_int(args, pos - 2) {
                                if (0..=30000).contains(&v) {
                                    dur = v as u32;
                                }
                            }
                            break;
                        }
                    }

                    let (x_flagged, x_val) = Self::decode_bgi_flagged_int(raw_x);
                    let (y_flagged, y_val) = Self::decode_bgi_flagged_int(raw_y);

                    let (target_x, dx) = match (x_flagged, x_val) {
                        (_, None) => (None, 0),
                        (true, Some(d)) => (None, d),
                        (false, Some(abs)) => (Some(abs), 0),
                    };
                    let (target_y, dy) = match (y_flagged, y_val) {
                        (_, None) => (None, 0),
                        (true, Some(d)) => (None, d),
                        (false, Some(abs)) => (Some(abs), 0),
                    };

                    if sid != 9 && sid != 10 {
                        let (mut from_x, mut from_y, mut to_x, mut to_y) = (0, 200, 0, 200);
                        if let Some(tr) = self.slot_transforms.get_mut(&sid) {
                            from_x = tr.x;
                            from_y = tr.y;
                            if let Some(tx) = target_x {
                                tr.x = tx;
                            } else {
                                tr.x += dx;
                            }
                            if let Some(ty) = target_y {
                                tr.y = ty;
                            } else {
                                tr.y += dy;
                            }
                            to_x = tr.x;
                            to_y = tr.y;
                        }
                        self.pending_slot_motions.push(SlotMotion {
                            slot_id: sid,
                            is_repeat: false,
                            target_x,
                            target_y,
                            dx,
                            dy,
                            from_x,
                            from_y,
                            to_x,
                            to_y,
                            duration_ms: dur,
                        });
                    }
                }
            }
            Some("_MoveRepeatBS") => {
                if use_stack && stack_args_len >= 11 {
                    let args = &self.eval_stack[..stack_args_len];
                    let sid = Self::stack_int(args, 0).unwrap_or(1).max(0) as u32;
                    let dy = Self::stack_int(args, 6).unwrap_or(0);
                    let dur = Self::stack_int(args, 10).unwrap_or(175).max(0) as u32;
                    if sid != 9 && sid != 10 {
                        let (cur_x, cur_y) = self
                            .slot_transforms
                            .get(&sid)
                            .map(|tr| (tr.x, tr.y))
                            .unwrap_or((0, 200));
                        self.pending_slot_motions.push(SlotMotion {
                            slot_id: sid,
                            is_repeat: true,
                            target_x: None,
                            target_y: None,
                            dx: 0,
                            dy,
                            from_x: cur_x,
                            from_y: cur_y,
                            to_x: cur_x,
                            to_y: cur_y,
                            duration_ms: dur,
                        });
                    }
                }
            }
            Some("_FadeBS") => {
                let slot_opt = if use_stack {
                    Self::stack_int(&self.eval_stack[..stack_args_len], 0)
                        .filter(|&v| v >= 0)
                        .map(|v| v as u32)
                } else {
                    self.find_prev_slot_id(self.pc, base)?
                };
                match slot_opt {
                    Some(9) | Some(10) => {
                        self.current_face = None;
                        self.current_face_sprites.clear();
                        self.face_drawn_since_dialogue = false;
                        return Ok(Some(VmEvent::SpriteClear { slot: slot_opt }));
                    }
                    Some(sid) => {
                        self.slots.remove(&sid);
                        self.slot_transforms.remove(&sid);
                        self.slots_drawn_since_dialogue.remove(&sid);
                        return Ok(Some(VmEvent::SpriteClear { slot: Some(sid) }));
                    }
                    None => {
                        self.current_face = None;
                        self.current_face_sprites.clear();
                        self.face_drawn_since_dialogue = false;
                        return Ok(Some(VmEvent::SpriteClear { slot: None }));
                    }
                }
            }
            Some("clear_bs") => {
                self.slots.clear();
                self.slot_transforms.clear();
                self.pending_slot_motions.clear();
                self.slots_drawn_since_dialogue.clear();
                self.face_drawn_since_dialogue = false;
                self.current_face = None;
                self.current_face_sprites.clear();
                return Ok(Some(VmEvent::SpriteClear { slot: None }));
            }
            Some("_SelectEx") => {
                // 标准模式下从 eval_stack 按 N 与栈顶 prompt 类型解析选项，保留无标准操作码路径的回退以兼容既有单测
                let (prompt, options) = if self.has_standard_opcodes && stack_args_len >= 2 {
                    let args = &self.eval_stack[..stack_args_len];
                    let n = match args.first() {
                        Some(VmValue::Int(v)) if *v > 0 => *v as usize,
                        _ => 0,
                    };
                    if n > 0 && args.len() >= n + 2 {
                        let prompt = match args.last() {
                            Some(VmValue::StrOffset(off)) => {
                                let s = self.script.get_string(*off)?;
                                if s.is_empty() {
                                    None
                                } else {
                                    Some(s)
                                }
                            }
                            Some(VmValue::Int(0)) | None => None,
                            _ => None,
                        };
                        let opt_start = args.len() - 1 - n;
                        let mut opts = Vec::with_capacity(n);
                        let mut all_str = true;
                        for idx in opt_start..(args.len() - 1) {
                            if let Some(s) = self.stack_str(args, idx) {
                                opts.push(s);
                            } else {
                                all_str = false;
                                break;
                            }
                        }
                        if all_str && opts.len() == n {
                            (prompt, opts)
                        } else {
                            self.find_prev_choice_options(self.pc, 300, base)?
                        }
                    } else {
                        self.find_prev_choice_options(self.pc, 300, base)?
                    }
                } else {
                    self.find_prev_choice_options(self.pc, 300, base)?
                };
                let is_custom_name = self
                    .sys_regs
                    .get(&0)
                    .copied()
                    .unwrap_or(DEFAULT_IS_DEFAULT_NAME)
                    == 0;
                let prompt = prompt.map(|p| apply_protagonist_name(&p, is_custom_name));
                let options: Vec<String> = options
                    .into_iter()
                    .map(|o| apply_protagonist_name(&o, is_custom_name))
                    .collect();
                // 双保险清零第 2 处：消费 0x0166 禁用集合并过滤越界项，防止串到下一级选单
                let disabled_options: Vec<usize> =
                    std::mem::take(&mut self.pending_disabled_choices)
                        .into_iter()
                        .filter(|&idx| idx < options.len())
                        .collect();
                self.choice_seq = self.choice_seq.saturating_add(1);
                self.state = VmState::WaitingForChoice {
                    prompt: prompt.clone(),
                    options: options.clone(),
                    disabled_options: disabled_options.clone(),
                };
                return Ok(Some(VmEvent::Choice {
                    prompt,
                    options,
                    disabled_options,
                }));
            }
            Some("Random") => {
                // Random(n)：实证 eye_short 写入 flags[0]，防御 n <= 0 返回 0 防除零 panic
                let n = if use_stack {
                    Self::stack_int(&self.eval_stack[..stack_args_len], 0).unwrap_or(0)
                } else {
                    0
                };
                let val = if n <= 0 {
                    0
                } else {
                    (self.next_random_u64() % (n as u64)) as i32
                };
                self.flags.insert(0, val);
            }
            Some("_SetLocalShortStrings") => {
                // _SetLocalShortStrings(slot, text)：写入局部短字符串表（%0 供 eye_long，%1 供短信贴图）
                if use_stack && stack_args_len >= 2 {
                    let args = &self.eval_stack[..stack_args_len];
                    if let (Some(slot), Some(text)) =
                        (Self::stack_int(args, 0), self.stack_str(args, 1))
                    {
                        if slot >= 0 {
                            self.local_short_strings.insert(slot as u32, text);
                        }
                    }
                }
            }
            Some("_DrawSPR") => {
                // 精准白名单特征放行（严禁盲放 02_prologue2 / 03_prologue3 / end_roll 等非零偏移道具图）：
                // 仅放行 slot==0 && x==0 && y==0 && scale==100，且资产名来自已展开 %1 或处于短信上下文
                if use_stack && stack_args_len >= 8 {
                    let args = &self.eval_stack[..stack_args_len];
                    let slot = Self::stack_int(args, 0).unwrap_or(-1);
                    let raw_asset = self.stack_str(args, 1).unwrap_or_default();
                    let x = Self::stack_int(args, 5).unwrap_or(-1);
                    let y = Self::stack_int(args, 6).unwrap_or(-1);
                    let scale = Self::stack_int(args, 7).unwrap_or(-1);
                    let fade_ms = Self::stack_int(args, 19).unwrap_or(300).max(0) as u32;

                    let (asset, was_expanded) = self.expand_local_short_string(&raw_asset);
                    let clean_asset = asset.trim().to_string();
                    if slot == 0
                        && x == 0
                        && y == 0
                        && scale == 100
                        && (was_expanded || self.is_mail_spr_context())
                        && !clean_asset.is_empty()
                        && !clean_asset.starts_with('%')
                    {
                        self.spr_overlay = Some(SprOverlayState {
                            slot: 0,
                            asset: clean_asset.clone(),
                            fade_ms,
                        });
                        self.last_spr_fade_ms = fade_ms;
                        return Ok(Some(VmEvent::SprDraw {
                            slot: 0,
                            asset: clean_asset,
                            fade_ms,
                        }));
                    }
                }
            }
            Some("_FadeSPR") => {
                // _FadeSPR(slot, ..., fade_ms=args[7], ...)：仅在 slot 0 存在活动 SprOverlay 时淡出清空
                if use_stack && stack_args_len >= 1 {
                    let args = &self.eval_stack[..stack_args_len];
                    let slot = Self::stack_int(args, 0).unwrap_or(-1);
                    if slot == 0 && self.spr_overlay.is_some() {
                        let fade_ms = Self::stack_int(args, 7).unwrap_or(200).max(0) as u32;
                        self.spr_overlay = None;
                        self.last_spr_fade_ms = fade_ms;
                        return Ok(Some(VmEvent::SprFade { slot: 0, fade_ms }));
                    }
                }
            }
            Some("_AnimateSPR") => {
                // 严格白名单放行：仅放行 slot == 9 的 "mail_on_A" 5 帧气泡提示，不干扰 slot == 0 的告白动画
                if use_stack && stack_args_len >= 2 {
                    let args = &self.eval_stack[..stack_args_len];
                    let slot = Self::stack_int(args, 0).unwrap_or(-1);
                    let asset = self.stack_str(args, 1).unwrap_or_default();
                    if slot == 9 && asset.trim().eq_ignore_ascii_case("mail_on_a") {
                        self.pending_mail_on_notify = true;
                    }
                }
            }
            Some("_PlaySE") => {
                // _PlaySE(ch, asset_name, volume, pan, ...)：提取音效名（如 "keitai-Key_01"）并发出 SePlay 事件
                if use_stack && stack_args_len >= 1 {
                    let args = &self.eval_stack[..stack_args_len];
                    let mut se_name = self.stack_str(args, 1).filter(|s| !s.trim().is_empty());
                    if se_name.is_none() {
                        for idx in 0..args.len() {
                            if let Some(s) = self.stack_str(args, idx) {
                                let trimmed = s.trim();
                                if !trimmed.is_empty()
                                    && trimmed != "_PlaySE"
                                    && !trimmed.ends_with(".bs5")
                                    && !trimmed.ends_with(".h")
                                {
                                    se_name = Some(trimmed.to_string());
                                    break;
                                }
                            }
                        }
                    }
                    let volume = Self::stack_int(args, 2).unwrap_or(100);
                    if let Some(se) = se_name {
                        let clean = se.trim().to_string();
                        if !clean.is_empty() {
                            self.pending_se_plays.push(clean.clone());
                            return Ok(Some(VmEvent::SePlay { se: clean, volume }));
                        }
                    }
                }
            }
            Some("_Wait") => {
                // _Wait(duration_ms, skippable)：
                // 当 duration_ms < 0 且 skippable != 0（即 _Wait(-1, 1)）时进入独立点击翻页挂起态 WaitingForWaitClick，
                // 零污染 dialogue_counter 与 Backlog；正毫秒数保持无阻塞 Wait 事件。
                let raw_dur = if use_stack {
                    Self::stack_int(&self.eval_stack[..stack_args_len], 0).unwrap_or(0)
                } else {
                    0
                };
                let skippable = if use_stack {
                    Self::stack_int(&self.eval_stack[..stack_args_len], 1).unwrap_or(1) != 0
                } else {
                    true
                };
                if raw_dur < 0 && skippable {
                    self.state = VmState::WaitingForWaitClick;
                    return Ok(Some(VmEvent::WaitClick));
                }
                let duration_ms = raw_dur.max(0) as u32;
                return Ok(Some(VmEvent::Wait {
                    duration_ms,
                    skippable,
                }));
            }
            Some("_PlayMovie") => {
                // Batch 1 限流优雅降级桩：按实参个数精准清理求值栈（由外层 eval_stack.clear() 保证），输出一条 info 日志并无阻塞推进
                let movie_name = if use_stack {
                    self.stack_str(&self.eval_stack[..stack_args_len], 0)
                        .unwrap_or_else(|| "<unknown>".to_string())
                } else {
                    "<unknown>".to_string()
                };
                eprintln!(
                    "[VmEngine INFO] _PlayMovie stub skipped (Batch 1): script={}, movie={}",
                    self.script_name, movie_name
                );
            }
            _ => {}
        }

        Ok(None)
    }


    /// 从求值栈提取跨脚本调用目标名（配合 0x00F6 入参，自栈顶向下扫描防止旧残留串扰，未压栈时回退字节码前瞻）
    fn extract_subscript_target(&self, base: usize) -> Result<Option<String>> {
        if self.has_standard_opcodes {
            for val in self.eval_stack.iter().rev() {
                if let VmValue::StrOffset(off) = *val {
                    if let Ok(s) = self.script.get_string(off) {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            return Ok(Some(trimmed.to_string()));
                        }
                    }
                }
            }
        }
        self.find_prev_subscript_string(self.pc, 64, base)
    }

    /// 处理子脚本调用指令 (Opcode 0x00F0)：
    /// - 对 `introduction_*` 保持原生 `VmEvent::CharacterIntro` 拦截，绝不双触发或破坏登场特写；
    /// - 对其他目标：校验 MAX_CALL_DEPTH=16 熔断，压栈保存父帧并无缝切换至子脚本（保留舞台视觉与寄存器）。
    /// 返回 `(Option<VmEvent>, switched_script)`
    fn handle_subscript_call(&mut self, base: usize) -> Result<(Option<VmEvent>, bool)> {
        let sub_name = self.extract_subscript_target(base)?;
        if let Some(ref name) = sub_name {
            let lower = name.trim().to_ascii_lowercase();
            if lower.starts_with("introduction_") {
                let char_key = lower.trim_start_matches("introduction_");
                let normalized_char = match char_key {
                    "sora" => "sora",
                    "moemi" => "moemi",
                    "arisa" => "arisa",
                    "ayumi" => "ayumi",
                    "shiho" | "shiho2" => "shiho",
                    other => other,
                };
                let jingle = match normalized_char {
                    "sora" => Some("so_title".to_string()),
                    "moemi" => Some("mo_title".to_string()),
                    "arisa" => Some("ar_title".to_string()),
                    "ayumi" => Some("ay_title".to_string()),
                    "shiho" => Some("si_title".to_string()),
                    _ => None,
                };
                self.pending_intro = Some(normalized_char.to_string());
                return Ok((
                    Some(VmEvent::CharacterIntro {
                        character: normalized_char.to_string(),
                        voice_jingle: jingle,
                    }),
                    false,
                ));
            }

            if let Some(ref loader) = self.script_loader {
                if self.call_stack.len() >= MAX_CALL_DEPTH {
                    return Err(CoreError::InvalidFormat(
                        "子脚本调用深度超过上限 (MAX_CALL_DEPTH=16)",
                    ));
                }
                if let Some(bytes) = loader.load_script(name)? {
                    let owned = Arc::new(OwnedBgiScript::parse(bytes)?);
                    let next_active = ActiveScript::Owned(owned);
                    let next_std = Self::detect_standard_opcodes_active(&next_active);
                    let next_base = next_active.base_offset();

                    let old_script = std::mem::replace(&mut self.script, next_active);
                    let old_name = std::mem::replace(&mut self.script_name, name.clone());
                    let old_std = std::mem::replace(&mut self.has_standard_opcodes, next_std);
                    let old_sidx = std::mem::replace(&mut self.script_dialogue_index, 0);
                    let old_entry_flags =
                        std::mem::replace(&mut self.entry_flags, self.flags.clone());
                    let old_script_choices = std::mem::take(&mut self.script_choices);

                    self.call_stack.push(CallFrame {
                        script: old_script,
                        script_name: old_name,
                        return_pc: self.pc + 4,
                        has_standard_opcodes: old_std,
                        script_dialogue_index: old_sidx,
                        entry_flags: old_entry_flags,
                        script_choices: old_script_choices,
                        is_local: false,
                    });
                    self.pc = next_base;
                    return Ok((None, true));
                } else {
                    eprintln!("[VmEngine WARNING] 0x00F0 target script not found: {}", name);
                }
            } else {
                eprintln!("[VmEngine] Unhandled 0x00F0 subscript (no loader): {}", name);
            }
        }
        Ok((None, false))
    }

    /// 处理不可返回尾跳转指令 (Opcode 0x00F3)：
    /// 先清空 `call_stack` 与 `script_choices`，加载并切换至目标脚本从 `base_offset` 启动，严禁清空舞台视觉与寄存器。
    fn handle_tail_script_jump(&mut self, base: usize) -> Result<bool> {
        let target = self.extract_subscript_target(base)?;
        if let Some(ref name) = target {
            if let Some(ref loader) = self.script_loader {
                if let Some(bytes) = loader.load_script(name)? {
                    let owned = Arc::new(OwnedBgiScript::parse(bytes)?);
                    let next_active = ActiveScript::Owned(owned);
                    let next_std = Self::detect_standard_opcodes_active(&next_active);
                    let next_base = next_active.base_offset();

                    self.call_stack.clear();
                    self.script_choices.clear();
                    self.local_short_strings.clear();
                    self.pending_local_label = None;
                    self.pending_disabled_choices.clear();
                    self.script = next_active;
                    self.script_name = name.clone();
                    self.has_standard_opcodes = next_std;
                    self.script_dialogue_index = 0;
                    self.entry_flags = self.flags.clone();
                    self.pc = next_base;
                    return Ok(true);
                } else {
                    eprintln!("[VmEngine WARNING] 0x00F3 target script not found: {}", name);
                }
            } else {
                eprintln!("[VmEngine] Unhandled 0x00F3 tail jump (no loader): {}", name);
            }
        }
        Ok(false)
    }

    /// 向前提取立绘槽位编号
    fn find_prev_slot_id(&self, from_pc: usize, base: usize) -> Result<Option<u32>> {
        if from_pc >= base + 12 {
            let val = self.script.read_u32(from_pc - 12)?;
            if (1..=10).contains(&val) {
                return Ok(Some(val));
            }
        }
        Ok(None)
    }

    /// 向前检索子脚本名称
    fn find_prev_subscript_string(&self, from_pc: usize, max_bytes: usize, base: usize) -> Result<Option<String>> {
        let mut p = from_pc.saturating_sub(8);
        let min_p = from_pc.saturating_sub(max_bytes).max(base);
        while p >= min_p {
            let op = self.script.read_u32(p)?;
            if p != from_pc.saturating_sub(8)
                && (op == 0x001c || op == 0x0140 || op == 0x0143 || op == 0x00f0 || op == 0x00f3)
            {
                break;
            }
            if op == 3 {
                let off = self.script.read_u32(p + 4)?;
                let s = self.script.get_string(off)?;
                if !s.is_empty() && !s.ends_with(".bs5") && !s.ends_with(".h") {
                    return Ok(Some(s));
                }
            }
            if p < 4 { break; }
            p -= 4;
        }
        Ok(None)
    }


    /// 处理对白指令 (Opcode 0x0140 / 0x0143)
    /// 若无对应台词文本（例如仅为内部视听指令的帧同步标记），则返回 Ok(None)
    fn handle_dialogue(&mut self, base: usize) -> Result<Option<DialogueEvent>> {
        let pc = self.pc;

        // 检查前一条压栈字符串 (台词内容)
        let op1 = if pc >= base + 8 {
            self.script.read_u32(pc - 8)?
        } else {
            0
        };
        let arg1 = if pc >= base + 4 {
            self.script.read_u32(pc - 4)?
        } else {
            0
        };

        // 如果前一条指令不是 0x0003 (PushString)，说明不是对白显示，而是内部同步/刷新标记，直接忽略
        if op1 != 3 {
            return Ok(None);
        }

        let text = self.script.get_string(arg1)?;
        // 校验台词文本有效性：若为空白或脚本文件名，则非用户对白
        if text.trim().is_empty() || text.ends_with(".bs5") || text.ends_with(".h") {
            return Ok(None);
        }

        // 检查更早一条压栈字符串 (说话人姓名)
        let op2 = if pc >= base + 16 {
            self.script.read_u32(pc - 16)?
        } else {
            0
        };
        let arg2 = if pc >= base + 12 {
            self.script.read_u32(pc - 12)?
        } else {
            0
        };

        let speaker = if op2 == 3 {
            let spk = self.script.get_string(arg2)?;
            if spk.trim().is_empty() || spk.ends_with(".bs5") || spk.ends_with(".h") {
                None
            } else {
                Some(spk)
            }
        } else {
            None
        };

        let is_custom_name = self
            .sys_regs
            .get(&0)
            .copied()
            .unwrap_or(DEFAULT_IS_DEFAULT_NAME)
            == 0;
        let text = apply_protagonist_name(&text, is_custom_name);
        let speaker = speaker.map(|spk| apply_protagonist_name(&spk, is_custom_name));

        self.dialogue_counter += 1;
        self.script_dialogue_index += 1;
        let voice = self.current_voice.take(); // 语音伴随该句台词消费

        // 统一在 Rust 单一数据源中记录履历，防止前后端状态撕裂
        self.history.push(HistoryEntry {
            index: self.dialogue_counter,
            speaker: speaker.clone(),
            text: text.clone(),
            voice: voice.clone(),
        });

        let intro = self.pending_intro.take();
        let bg_motions = std::mem::take(&mut self.pending_bg_motions);
        let slot_motions = std::mem::take(&mut self.pending_slot_motions);
        let se_plays = std::mem::take(&mut self.pending_se_plays);
        self.slots_drawn_since_dialogue.clear();
        self.face_drawn_since_dialogue = false;

        Ok(Some(DialogueEvent {
            index: self.dialogue_counter,
            speaker,
            text,
            voice,
            face: self.current_face.clone(),
            face_sprites: self.current_face_sprites.clone(),
            face_offset: self.current_face_offset,
            bg: self.current_bg.clone(),
            bg_pose: self.current_bg_pose.clone(),
            bg_motions,
            bgm: self.current_bgm.clone(),
            slots: self.slots.clone(),
            slot_transforms: self.slot_transforms.clone(),
            slot_motions,
            pc,
            character_intro: intro,
            spr_overlay: self.spr_overlay.clone(),
            spr_fade_ms: self.last_spr_fade_ms,
            se_plays,
        }))
    }

    /// 获取历史履历切片 (供 Backlog 分页拉取)
    pub fn get_history(&self, offset: usize, limit: usize) -> &[HistoryEntry] {
        let total = self.history.len();
        if offset >= total {
            return &[];
        }
        let end = (offset + limit).min(total);
        &self.history[offset..end]
    }

    /// 获取历史履历总条数
    #[inline]
    pub fn history_count(&self) -> usize {
        self.history.len()
    }

    /// 获取当前 FaceAvatar 标识
    #[inline]
    pub fn current_face(&self) -> Option<&str> {
        self.current_face.as_deref()
    }

    /// 向前快速检索最近的一条 PushString 字符串
    fn find_prev_string(&self, from_pc: usize, max_bytes: usize, base: usize) -> Result<Option<String>> {
        let mut p = from_pc.saturating_sub(8);
        let min_p = from_pc.saturating_sub(max_bytes).max(base);
        while p >= min_p {
            let op = self.script.read_u32(p)?;
            if op == 3 {
                let off = self.script.read_u32(p + 4)?;
                return Ok(Some(self.script.get_string(off)?));
            }
            if p < 4 { break; }
            p -= 4;
        }
        Ok(None)
    }


    /// 向前检索语音 ID (排除函数名与脚本名)
    fn find_prev_voice_string(&self, from_pc: usize, max_bytes: usize, base: usize) -> Result<Option<String>> {
        let mut p = from_pc.saturating_sub(8);
        let min_p = from_pc.saturating_sub(max_bytes).max(base);
        while p >= min_p {
            let op = self.script.read_u32(p)?;
            if p != from_pc.saturating_sub(8)
                && (op == 0x001c || op == 0x0140 || op == 0x0143 || op == 0x00f0 || op == 0x00f3)
            {
                break;
            }
            if op == 3 {
                let off = self.script.read_u32(p + 4)?;
                let s = self.script.get_string(off)?;
                if !s.is_empty() && s != "_PlayVoice" && !s.ends_with(".bs5") && !s.ends_with(".h") && !s.starts_with("introduction_") {
                    return Ok(Some(s));
                }
            }
            if p < 4 { break; }
            p -= 4;
        }
        Ok(None)
    }


    /// 向前检索 BGM 标识 (以 PC_BGM_ 开头)
    fn find_prev_bgm_string(&self, from_pc: usize, max_bytes: usize, base: usize) -> Result<Option<String>> {
        let mut p = from_pc.saturating_sub(8);
        let min_p = from_pc.saturating_sub(max_bytes).max(base);
        while p >= min_p {
            let op = self.script.read_u32(p)?;
            if p != from_pc.saturating_sub(8)
                && (op == 0x001c || op == 0x0140 || op == 0x0143 || op == 0x00f0 || op == 0x00f3)
            {
                break;
            }
            if op == 3 {
                let off = self.script.read_u32(p + 4)?;
                let s = self.script.get_string(off)?;
                if s.starts_with("PC_BGM_") {
                    return Ok(Some(s));
                }
            }
            if p < 4 { break; }
            p -= 4;
        }
        Ok(None)
    }


    /// 向前检索场景背景图名
    fn find_prev_scene_string(&self, from_pc: usize, max_bytes: usize, base: usize) -> Result<Option<String>> {
        let mut p = from_pc.saturating_sub(8);
        let min_p = from_pc.saturating_sub(max_bytes).max(base);
        while p >= min_p {
            let op = self.script.read_u32(p)?;
            if p != from_pc.saturating_sub(8)
                && (op == 0x001c || op == 0x0140 || op == 0x0143 || op == 0x00f0 || op == 0x00f3)
            {
                break;
            }
            if op == 3 {
                let off = self.script.read_u32(p + 4)?;
                let s = self.script.get_string(off)?;
                if Self::is_valid_scene_arg(&s) {
                    return Ok(Some(s));
                }
            }
            if p < 4 { break; }
            p -= 4;
        }
        Ok(None)
    }


    /// 向前检索立绘部件列表
    fn find_prev_sprites(&self, from_pc: usize, max_bytes: usize, base: usize) -> Result<Vec<String>> {
        let mut sprites = Vec::new();
        let mut p = from_pc.saturating_sub(8);
        let min_p = from_pc.saturating_sub(max_bytes).max(base);
        while p >= min_p {
            let op = self.script.read_u32(p)?;
            if p != from_pc.saturating_sub(8)
                && (op == 0x001c || op == 0x0140 || op == 0x0143 || op == 0x00f0 || op == 0x00f3)
            {
                break;
            }
            if op == 3 {
                let off = self.script.read_u32(p + 4)?;
                let s = self.script.get_string(off)?;
                if !s.is_empty()
                    && s != "_DrawBS"
                    && !s.ends_with(".bs5")
                    && !s.ends_with(".h")
                    && !s.starts_with("EFFECT")
                    && !s.starts_with("PrintMessage")
                    && !s.starts_with("introduction_")
                {
                    sprites.push(s);
                }
            }
            if p < 4 { break; }
            p -= 4;
        }
        sprites.reverse();
        Ok(sprites)
    }

    /// 向前检索分支选项文本
    fn find_prev_choice_options(
        &self,
        from_pc: usize,
        max_bytes: usize,
        base: usize,
    ) -> Result<(Option<String>, Vec<String>)> {
        let mut list = Vec::new();
        let mut p = from_pc.saturating_sub(8);
        let min_p = from_pc.saturating_sub(max_bytes).max(base);
        while p >= min_p {
            let op = self.script.read_u32(p)?;
            if op == 3 {
                let off = self.script.read_u32(p + 4)?;
                let s = self.script.get_string(off)?;
                if !s.is_empty() && s != "_SelectEx" && !s.ends_with(".bs5") {
                    list.push(s);
                }
            }
            if p < 4 { break; }
            p -= 4;
        }
        list.reverse();
        if list.is_empty() {
            return Ok((None, Vec::new()));
        }
        let prompt = list.pop();
        Ok((prompt, list))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::decoder::BGI_SCRIPT_MAGIC;

    #[test]
    fn test_synthetic_vm_dialogue_flow() {
        let mut data = Vec::new();
        // 头部
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes()); // header_size = 8
        data.extend_from_slice(&0u32.to_le_bytes()); // 0 modules
        // base_offset = 28 + 8 = 36

        // 我们将字节码放在前面，字符串放在后面
        // 计算字符串起始位置：假设字节码长度为 200 字节
        let code_len = 200usize;
        let mut strings = Vec::new();

        let mut add_str = |s: &str| -> u32 {
            let rel_offset = (code_len + strings.len()) as u32;
            let (gbk, _, _) = encoding_rs::GBK.encode(s);
            strings.extend_from_slice(&gbk);
            strings.push(0);
            rel_offset
        };

        let s_white = add_str("white");
        let s_draw_scene = add_str("_DrawScene");
        let s_bgm = add_str("PC_BGM_17");
        let s_play_bgm = add_str("_PlayBGM");
        let s_voice = add_str("AY_0202");
        let s_play_voice = add_str("_PlayVoice");
        let s_speaker = add_str("步海");
        let s_dialogue = add_str("哥哥，步海想玩过家家");

        let mut code = Vec::new();
        // 1. _DrawScene("white")
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&s_white.to_le_bytes());
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&s_draw_scene.to_le_bytes());
        code.extend_from_slice(&0x1cu32.to_le_bytes());

        // 2. _PlayBGM("PC_BGM_17")
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&s_bgm.to_le_bytes());
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&s_play_bgm.to_le_bytes());
        code.extend_from_slice(&0x1cu32.to_le_bytes());

        // 3. _PlayVoice("AY_0202")
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&s_voice.to_le_bytes());
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&s_play_voice.to_le_bytes());
        code.extend_from_slice(&0x1cu32.to_le_bytes());

        // 4. Dialogue: Speaker="步海", Text="哥哥，步海想玩过家家"
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&s_speaker.to_le_bytes());
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&s_dialogue.to_le_bytes());
        code.extend_from_slice(&0x140u32.to_le_bytes());

        // 补齐代码段至 code_len
        assert!(code.len() <= code_len);
        code.resize(code_len, 0);

        // 拼合整体剧本数据
        data.extend_from_slice(&code);
        data.extend_from_slice(&strings);

        let script = BgiScript::parse(&data).expect("解析合成脚本失败");
        let mut vm = VmEngine::new(&script);
        assert_eq!(*vm.state(), VmState::Ready);

        // 运行到第一句对白
        let dlg = vm.next_dialogue().expect("VM执行失败").expect("未提取到对白");
        assert_eq!(dlg.index, 1);
        assert_eq!(dlg.speaker.as_deref(), Some("步海"));
        assert_eq!(dlg.text, "哥哥，步海想玩过家家");
        assert_eq!(dlg.voice.as_deref(), Some("AY_0202"));
        assert_eq!(dlg.bg.as_deref(), Some("white"));
        assert_eq!(dlg.bgm.as_deref(), Some("PC_BGM_17"));

        // 验证当前状态机处于 WaitingForInput 断点
        assert_eq!(*vm.state(), VmState::WaitingForInput);

        // 再次获取对白应返回 None (因为脚本结束)
        let next = vm.next_dialogue().expect("VM后续执行失败");
        assert!(next.is_none());
        assert_eq!(*vm.state(), VmState::Terminated);
    }

    #[test]
    fn test_dialogue_alt_and_narration() {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let code_len = 100usize;
        let mut strings = Vec::new();

        let mut add_str = |s: &str| -> u32 {
            let rel_offset = (code_len + strings.len()) as u32;
            let (gbk, _, _) = encoding_rs::GBK.encode(s);
            strings.extend_from_slice(&gbk);
            strings.push(0);
            rel_offset
        };

        let s_narration = add_str("这是通过0x143触发的无角色纯旁白台词");

        let mut code = Vec::new();
        // 单个字符串压栈 + 0x143
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&s_narration.to_le_bytes());
        code.extend_from_slice(&0x143u32.to_le_bytes());

        code.resize(code_len, 0);
        data.extend_from_slice(&code);
        data.extend_from_slice(&strings);

        let script = BgiScript::parse(&data).expect("解析失败");
        let mut vm = VmEngine::new(&script);

        let dlg = vm
            .next_dialogue()
            .expect("执行失败")
            .expect("未能获取到对白");
        assert_eq!(dlg.index, 1);
        assert!(dlg.speaker.is_none(), "旁白不应含有说话人");
        assert_eq!(dlg.text, "这是通过0x143触发的无角色纯旁白台词");
        assert_eq!(*vm.state(), VmState::WaitingForInput);

        vm.resume();
        assert!(vm.next_dialogue().expect("执行失败").is_none());
        assert_eq!(*vm.state(), VmState::Terminated);
    }

    #[test]
    fn test_empty_dialogue_marker_is_skipped() {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let code_len = 100usize;
        let mut strings = Vec::new();

        let mut add_str = |s: &str| -> u32 {
            let rel_offset = (code_len + strings.len()) as u32;
            let (gbk, _, _) = encoding_rs::GBK.encode(s);
            strings.extend_from_slice(&gbk);
            strings.push(0);
            rel_offset
        };

        let s_real_text = add_str("这是紧随同步标记后的真实有效对白");

        let mut code = Vec::new();
        // 1. 模拟视听同步内部标记：前无 PushString，直接调用 0x140
        code.extend_from_slice(&0u32.to_le_bytes());
        code.extend_from_slice(&0x140u32.to_le_bytes());

        // 2. 真实对白：单条字符串压栈 + 0x140
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&s_real_text.to_le_bytes());
        code.extend_from_slice(&0x140u32.to_le_bytes());

        code.resize(code_len, 0);
        data.extend_from_slice(&code);
        data.extend_from_slice(&strings);

        let script = BgiScript::parse(&data).expect("解析失败");
        let mut vm = VmEngine::new(&script);

        // 验证内部标记被自动跳过，第一句直接返回真实对白，且 index 为 1
        let dlg = vm
            .next_dialogue()
            .expect("执行失败")
            .expect("未能获取到真实对白");
        assert_eq!(dlg.index, 1);
        assert_eq!(dlg.text, "这是紧随同步标记后的真实有效对白");
        assert_eq!(*vm.state(), VmState::WaitingForInput);

        vm.resume();
        assert!(vm.next_dialogue().expect("执行失败").is_none());
    }

    #[test]
    fn test_waiting_for_choice_state_machine() {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let code_len = 150usize;
        let mut strings = Vec::new();

        let mut add_str = |s: &str| -> u32 {
            let rel_offset = (code_len + strings.len()) as u32;
            let (gbk, _, _) = encoding_rs::GBK.encode(s);
            strings.extend_from_slice(&gbk);
            strings.push(0);
            rel_offset
        };

        let opt1 = add_str("去休息室");
        let opt2 = add_str("去事务所");
        let prompt = add_str("去哪里呢？");
        let fn_select = add_str("_SelectEx");
        let after_txt = add_str("分支选择后的台词");

        let mut code = Vec::new();
        // 压入分支选项
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&opt1.to_le_bytes());
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&opt2.to_le_bytes());
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&prompt.to_le_bytes());
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&fn_select.to_le_bytes());
        code.extend_from_slice(&0x1cu32.to_le_bytes()); // CALL_USER_FUNC

        // 分支后续对白
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&after_txt.to_le_bytes());
        code.extend_from_slice(&0x140u32.to_le_bytes());

        code.resize(code_len, 0);
        data.extend_from_slice(&code);
        data.extend_from_slice(&strings);

        let script = BgiScript::parse(&data).expect("解析失败");
        let mut vm = VmEngine::new(&script);

        // step 执行遇到分支选择
        let evt = vm.step().expect("step失败").expect("预期得到Choice事件");
        match evt {
            VmEvent::Choice {
                prompt: p, options, ..
            } => {
                assert_eq!(p.as_deref(), Some("去哪里呢？"));
                assert_eq!(options, vec!["去休息室", "去事务所"]);
            }
            other => panic!("预期 VmEvent::Choice，得到 {:?}", other),
        }
        assert!(matches!(*vm.state(), VmState::WaitingForChoice { .. }));

        // 再次 step 应保持 WaitingForChoice 状态并重发 Choice 事件，禁止静默切换为 Running
        let re_evt = vm.step().expect("step失败").expect("预期重复Choice");
        assert!(matches!(re_evt, VmEvent::Choice { .. }));
        assert!(matches!(*vm.state(), VmState::WaitingForChoice { .. }));

        // next_dialogue 在未做出选择时应返回 None
        assert!(vm.next_dialogue().expect("执行失败").is_none());

        // 用户做出选择后恢复
        vm.select_choice(0);
        assert_eq!(*vm.state(), VmState::Running);

        // 恢复后流式执行可到达下一句对白
        let dlg = vm
            .next_dialogue()
            .expect("执行失败")
            .expect("预期得到分支后对白");
        assert_eq!(dlg.text, "分支选择后的台词");
    }

    #[test]
    fn test_multi_slot_character_drawing() {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let code_len = 200usize;
        let mut strings = Vec::new();

        let mut add_str = |s: &str| -> u32 {
            let rel_offset = (code_len + strings.len()) as u32;
            let (gbk, _, _) = encoding_rs::GBK.encode(s);
            strings.extend_from_slice(&gbk);
            strings.push(0);
            rel_offset
        };

        let sp_so = add_str("SO01_L_S_A11");
        let fn_draw = add_str("_DrawBS");
        let sp_ay = add_str("AY01_F_H_SA");
        let txt = add_str("双角色同屏测试");

        let mut code = Vec::new();
        // 1. Slot 1 (Sora)
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&sp_so.to_le_bytes());
        code.extend_from_slice(&1u32.to_le_bytes()); // slot 1
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&fn_draw.to_le_bytes());
        code.extend_from_slice(&0x1cu32.to_le_bytes());

        // 2. Slot 3 (Ayumi)
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&sp_ay.to_le_bytes());
        code.extend_from_slice(&3u32.to_le_bytes()); // slot 3
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&fn_draw.to_le_bytes());
        code.extend_from_slice(&0x1cu32.to_le_bytes());

        // 3. Dialogue
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&txt.to_le_bytes());
        code.extend_from_slice(&0x140u32.to_le_bytes());

        code.resize(code_len, 0);
        data.extend_from_slice(&code);
        data.extend_from_slice(&strings);

        let script = BgiScript::parse(&data).expect("解析失败");
        let mut vm = VmEngine::new(&script);

        let dlg = vm.next_dialogue().expect("执行失败").expect("未提取到对白");
        assert_eq!(dlg.slots.len(), 2, "应同时存在两个槽位的角色");
        assert_eq!(dlg.slots.get(&1).unwrap().sprites, vec!["SO01_L_S_A11"]);
        assert_eq!(dlg.slots.get(&3).unwrap().sprites, vec!["AY01_F_H_SA"]);
    }

    #[test]
    fn test_character_introduction_subscript_and_stack_isolation() {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let code_len = 200usize;
        let mut strings = Vec::new();

        let mut add_str = |s: &str| -> u32 {
            let rel_offset = (code_len + strings.len()) as u32;
            let (gbk, _, _) = encoding_rs::GBK.encode(s);
            strings.extend_from_slice(&gbk);
            strings.push(0);
            rel_offset
        };

        let sub_name = add_str("introduction_sora");
        let txt = add_str("空登场后的第一句台词");
        let spk = add_str("空的朋友A");

        let mut code = Vec::new();
        // 1. 模拟 BGI 0x895c 处的 0x00F0 登场特写调用
        code.extend_from_slice(&3u32.to_le_bytes()); // PushString
        code.extend_from_slice(&sub_name.to_le_bytes());
        code.extend_from_slice(&0u32.to_le_bytes()); // PUSH_ZERO
        code.extend_from_slice(&0u32.to_le_bytes()); // PUSH_ZERO
        code.extend_from_slice(&0x00f6u32.to_le_bytes()); // 0x00F6
        code.extend_from_slice(&0x00f0u32.to_le_bytes()); // 0x00F0 (CALL_SUBSCRIPT)

        // 2. 后续台词 (0x0140)
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&spk.to_le_bytes());
        code.extend_from_slice(&3u32.to_le_bytes());
        code.extend_from_slice(&txt.to_le_bytes());
        code.extend_from_slice(&0x140u32.to_le_bytes());

        code.resize(code_len, 0);
        data.extend_from_slice(&code);
        data.extend_from_slice(&strings);

        let script = BgiScript::parse(&data).expect("解析失败");
        let mut vm = VmEngine::new(&script);

        // step 推进触发 CharacterIntro 事件
        let mut got_intro = false;
        while let Some(evt) = vm.step().expect("step失败") {
            match evt {
                VmEvent::CharacterIntro { character, voice_jingle } => {
                    assert_eq!(character, "sora");
                    assert_eq!(voice_jingle.as_deref(), Some("so_title"));
                    got_intro = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(got_intro, "必须成功捕获 0x00F0 introduction_sora 登场特写事件");

        // 后续台词提取：验证 character_intro 被正确赋给对白，且入参已被清退，台词文本与说话人未受污染
        let dlg = vm.next_dialogue().expect("推进失败").expect("未提取到对白");
        assert_eq!(dlg.speaker.as_deref(), Some("空的朋友A"));
        assert_eq!(dlg.text, "空登场后的第一句台词");
        assert_eq!(dlg.character_intro.as_deref(), Some("sora"));
        assert!(dlg.slots.is_empty(), "introduction 字符串绝不应被误判为立绘差分");
    }

    #[test]
    fn test_standard_bgi_bg_pan_and_slot9_isolation() {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let code_len = 800usize;
        let mut strings = Vec::new();
        let mut add_str = |s: &str| -> u32 {
            let rel_offset = (code_len + strings.len()) as u32;
            let (gbk, _, _) = encoding_rs::GBK.encode(s);
            strings.extend_from_slice(&gbk);
            strings.push(0);
            rel_offset
        };

        let s_bg = add_str("pc_bg_04B_9");
        let s_draw_scene = add_str("_DrawScene");
        let s_move_sight = add_str("_MoveSight");
        let s_mo_base = add_str("MO01PC_F");
        let s_mo_face = add_str("MO01_F004");
        let s_si_base = add_str("SI02AA_N");
        let s_si_expr = add_str("SI01_N013");
        let s_draw_bs = add_str("_DrawBS");
        let s_fade_bs = add_str("_FadeBS");
        let s_spk = add_str("萌美");
        let s_txt1 = add_str("什锦烧好啦");
        let s_txt2 = add_str("第二句台词");

        let mut code = Vec::new();
        let push_int = |c: &mut Vec<u8>, v: i32| {
            c.extend_from_slice(&0u32.to_le_bytes());
            c.extend_from_slice(&(v as u32).to_le_bytes());
        };
        let push_str = |c: &mut Vec<u8>, off: u32| {
            c.extend_from_slice(&3u32.to_le_bytes());
            c.extend_from_slice(&off.to_le_bytes());
        };
        let stmt_end = |c: &mut Vec<u8>| c.extend_from_slice(&0x003fu32.to_le_bytes());
        let call_fn = |c: &mut Vec<u8>| c.extend_from_slice(&0x001cu32.to_le_bytes());

        // 1. _DrawScene("pc_bg_04B_9", 0, 2, 0, 0 - 1480, 0 - 340, 100, ..., 1000, 0)
        push_str(&mut code, s_bg);
        push_int(&mut code, 0);
        push_int(&mut code, 2);
        push_int(&mut code, 0);
        push_int(&mut code, 0);
        push_int(&mut code, 1480);
        code.extend_from_slice(&0x0021u32.to_le_bytes()); // SUB -> -1480
        push_int(&mut code, 0);
        push_int(&mut code, 340);
        code.extend_from_slice(&0x0021u32.to_le_bytes()); // SUB -> -340
        push_int(&mut code, 100);
        push_int(&mut code, 0);
        push_int(&mut code, 32767);
        push_int(&mut code, 32767);
        push_int(&mut code, 100);
        push_int(&mut code, 1);
        push_int(&mut code, 0);
        push_int(&mut code, 1000);
        push_int(&mut code, 0);
        push_str(&mut code, s_draw_scene);
        call_fn(&mut code);
        stmt_end(&mut code);

        // 2. _MoveSight(268434536, 0, 100, 40000, 0) -> target (-920, 0)
        push_int(&mut code, 268434536);
        push_int(&mut code, 0);
        push_int(&mut code, 100);
        push_int(&mut code, 40000);
        push_int(&mut code, 0);
        push_str(&mut code, s_move_sight);
        call_fn(&mut code);
        stmt_end(&mut code);

        // 3. _DrawBS slot 4 = SI02AA_N + SI01_N013
        push_int(&mut code, 4);
        push_str(&mut code, s_si_base);
        for _ in 0..3 { push_int(&mut code, 0); }
        push_int(&mut code, 2);
        push_int(&mut code, 0x1000_0000);
        push_int(&mut code, 200);
        push_int(&mut code, 0x1000_0000);
        for _ in 0..3 { push_int(&mut code, 0); }
        push_int(&mut code, 100);
        for _ in 0..9 { push_int(&mut code, 0); }
        push_int(&mut code, 200);
        push_int(&mut code, 0);
        push_int(&mut code, -256);
        push_str(&mut code, s_si_expr);
        push_str(&mut code, s_draw_bs);
        call_fn(&mut code);
        stmt_end(&mut code);

        // 4. _DrawBS slot 9 = MO01PC_F + MO01_F004 at (-541, 451)
        push_int(&mut code, 9);
        push_str(&mut code, s_mo_base);
        for _ in 0..3 { push_int(&mut code, 0); }
        push_int(&mut code, 2);
        push_int(&mut code, 0);
        push_int(&mut code, 541);
        code.extend_from_slice(&0x0021u32.to_le_bytes()); // -541
        push_int(&mut code, 451);
        push_int(&mut code, 0x1000_0000);
        for _ in 0..3 { push_int(&mut code, 0); }
        push_int(&mut code, 100);
        for _ in 0..9 { push_int(&mut code, 0); }
        push_int(&mut code, 200);
        push_int(&mut code, 0);
        push_int(&mut code, -256);
        push_str(&mut code, s_mo_face);
        push_str(&mut code, s_draw_bs);
        call_fn(&mut code);
        stmt_end(&mut code);

        // 5. Dialogue 1
        push_str(&mut code, s_spk);
        push_str(&mut code, s_txt1);
        code.extend_from_slice(&0x140u32.to_le_bytes());
        stmt_end(&mut code);

        // 6. _FadeBS(4) 应只清 slot 4 而不清 slot 9 头像
        push_int(&mut code, 4);
        push_str(&mut code, s_fade_bs);
        call_fn(&mut code);
        stmt_end(&mut code);

        // 7. Dialogue 2
        push_str(&mut code, s_spk);
        push_str(&mut code, s_txt2);
        code.extend_from_slice(&0x140u32.to_le_bytes());
        stmt_end(&mut code);

        assert!(code.len() <= code_len);
        code.resize(code_len, 0);
        data.extend_from_slice(&code);
        data.extend_from_slice(&strings);

        let script = BgiScript::parse(&data).expect("解析失败");
        let mut vm = VmEngine::new(&script);

        let d1 = vm.next_dialogue().expect("推进1失败").expect("缺少句1");
        assert_eq!(d1.bg.as_deref(), Some("pc_bg_04B_9"));
        let pose = d1.bg_pose.expect("应存在 BgPose");
        assert_eq!(pose.x, Some(-1480));
        assert_eq!(pose.y, Some(-340));
        assert_eq!(pose.scale, 100);
        assert_eq!(pose.transition_ms, 1000);
        assert_eq!(d1.bg_motions.len(), 1);
        assert_eq!(d1.bg_motions[0].target_x, Some(-920));
        assert_eq!(d1.bg_motions[0].target_y, Some(0));
        assert_eq!(d1.bg_motions[0].duration_ms, 40000);

        assert_eq!(d1.slots.len(), 1, "slot 9 绝不应污染舞台 slots");
        assert!(!d1.slots.contains_key(&9));
        assert_eq!(d1.slots.get(&4).unwrap().sprites, vec!["SI02AA_N", "SI01_N013"]);
        assert_eq!(d1.face_sprites, vec!["MO01PC_F", "MO01_F004"]);
        assert_eq!(d1.face_offset, (-541, 451));

        let d2 = vm.next_dialogue().expect("推进2失败").expect("缺少句2");
        assert!(d2.slots.is_empty(), "_FadeBS(4) 应清除舞台 slot 4");
        assert_eq!(d2.face_sprites, vec!["MO01PC_F", "MO01_F004"], "_FadeBS(4) 不应清除 slot 9 头像");
        assert!(d2.bg_motions.is_empty(), "bg_motions 应在上一句被消费");
    }

    #[test]
    fn test_draw_scene_clears_previous_scene_slots_but_preserves_batch_slots() {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let code_len = 900usize;
        let mut strings = Vec::new();
        let mut add_str = |s: &str| -> u32 {
            let rel_offset = (code_len + strings.len()) as u32;
            let (gbk, _, _) = encoding_rs::GBK.encode(s);
            strings.extend_from_slice(&gbk);
            strings.push(0);
            rel_offset
        };

        let s_bg1 = add_str("pc_bg_04B");
        let s_bg2 = add_str("pc_bg_44A");
        let s_black = add_str("black");
        let s_draw_scene = add_str("_DrawScene");
        let s_draw_bs = add_str("_DrawBS");
        let s_si = add_str("SI02AA_N");
        let s_ay = add_str("AY03PD_N");
        let s_spk = add_str("角色");
        let s_txt1 = add_str("场景一角色台词");
        let s_txt2 = add_str("场景二新场景开场白");
        let s_txt3 = add_str("场景三同批预置立绘台词");

        let mut code = Vec::new();
        let push_int = |c: &mut Vec<u8>, v: i32| {
            c.extend_from_slice(&0u32.to_le_bytes());
            c.extend_from_slice(&(v as u32).to_le_bytes());
        };
        let push_str = |c: &mut Vec<u8>, off: u32| {
            c.extend_from_slice(&3u32.to_le_bytes());
            c.extend_from_slice(&off.to_le_bytes());
        };
        let stmt_end = |c: &mut Vec<u8>| c.extend_from_slice(&0x003fu32.to_le_bytes());
        let call_fn = |c: &mut Vec<u8>| c.extend_from_slice(&0x001cu32.to_le_bytes());

        let push_bs_standard = |c: &mut Vec<u8>, slot_id: i32, sprite_off: u32| {
            push_int(c, slot_id);
            push_str(c, sprite_off);
            for _ in 0..3 { push_int(c, 0); }
            push_int(c, 2);
            push_int(c, 0x1000_0000);
            push_int(c, 200);
            push_int(c, 0x1000_0000);
            for _ in 0..3 { push_int(c, 0); }
            push_int(c, 100);
            push_str(c, s_draw_bs);
            call_fn(c);
            stmt_end(c);
        };

        // 场景一：_DrawScene(pc_bg_04B) + _DrawBS(4, SI02AA_N) + 对白1
        push_str(&mut code, s_bg1);
        push_str(&mut code, s_draw_scene);
        call_fn(&mut code);
        stmt_end(&mut code);

        push_bs_standard(&mut code, 4, s_si);

        push_str(&mut code, s_spk);
        push_str(&mut code, s_txt1);
        code.extend_from_slice(&0x140u32.to_le_bytes());
        stmt_end(&mut code);

        // 场景二：直接 _DrawScene(black) -> _DrawScene(pc_bg_44A)（未调用 clear_bs）+ 对白2
        push_str(&mut code, s_black);
        push_str(&mut code, s_draw_scene);
        call_fn(&mut code);
        stmt_end(&mut code);

        push_str(&mut code, s_bg2);
        push_str(&mut code, s_draw_scene);
        call_fn(&mut code);
        stmt_end(&mut code);

        push_str(&mut code, s_spk);
        push_str(&mut code, s_txt2);
        code.extend_from_slice(&0x140u32.to_le_bytes());
        stmt_end(&mut code);

        // 场景三：同批次先 _DrawBS(5, AY03PD_N) 再紧接 _DrawScene(pc_bg_04B) + 对白3
        push_bs_standard(&mut code, 5, s_ay);

        push_str(&mut code, s_bg1);
        push_str(&mut code, s_draw_scene);
        call_fn(&mut code);
        stmt_end(&mut code);

        push_str(&mut code, s_spk);
        push_str(&mut code, s_txt3);
        code.extend_from_slice(&0x140u32.to_le_bytes());
        stmt_end(&mut code);

        assert!(code.len() <= code_len);
        code.resize(code_len, 0);
        data.extend_from_slice(&code);
        data.extend_from_slice(&strings);

        let script = BgiScript::parse(&data).expect("解析失败");
        let mut vm = VmEngine::new(&script);

        let d1 = vm.next_dialogue().expect("句1失败").unwrap();
        assert_eq!(d1.slots.len(), 1);
        assert!(d1.slots.contains_key(&4));

        let d2 = vm.next_dialogue().expect("句2失败").unwrap();
        assert!(
            d2.slots.is_empty(),
            "_DrawScene 切换场景后必须清空上一幕遗留的 Slot 4，防止新旧立绘重叠"
        );

        let d3 = vm.next_dialogue().expect("句3失败").unwrap();
        assert_eq!(d3.slots.len(), 1);
        assert!(
            d3.slots.contains_key(&5),
            "与 _DrawScene 同批次预置的 Slot 5 立绘必须保留"
        );
    }

    #[test]
    fn test_draw_scene_non_pc_bg_and_move_bs_duration() {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let code_len = 800usize;
        let mut strings = Vec::new();
        let mut add_str = |s: &str| -> u32 {
            let rel_offset = (code_len + strings.len()) as u32;
            let (gbk, _, _) = encoding_rs::GBK.encode(s);
            strings.extend_from_slice(&gbk);
            strings.push(0);
            rel_offset
        };

        let s_cg = add_str("arev01A");
        let s_eff = add_str("EFFECT02");
        let s_draw_scene = add_str("_DrawScene");
        let s_draw_bs = add_str("_DrawBS");
        let s_move_bs = add_str("_MoveBS");
        let s_so = add_str("SO01PB_M");
        let s_spk = add_str("空");
        let s_txt = add_str("事件CG与立绘平移动画测试");

        let mut code = Vec::new();
        let push_int = |c: &mut Vec<u8>, v: i32| {
            c.extend_from_slice(&0u32.to_le_bytes());
            c.extend_from_slice(&(v as u32).to_le_bytes());
        };
        let push_str = |c: &mut Vec<u8>, off: u32| {
            c.extend_from_slice(&3u32.to_le_bytes());
            c.extend_from_slice(&off.to_le_bytes());
        };
        let stmt_end = |c: &mut Vec<u8>| c.extend_from_slice(&0x003fu32.to_le_bytes());
        let call_fn = |c: &mut Vec<u8>| c.extend_from_slice(&0x001cu32.to_le_bytes());

        // 1. _DrawScene("arev01A", "EFFECT02", ...)
        push_str(&mut code, s_cg);
        push_str(&mut code, s_eff);
        for _ in 0..2 { push_int(&mut code, 1); }
        push_int(&mut code, 32767);
        push_int(&mut code, 32767);
        push_int(&mut code, 100);
        for _ in 0..6 { push_int(&mut code, 0); }
        push_int(&mut code, 1000);
        push_str(&mut code, s_draw_scene);
        call_fn(&mut code);
        stmt_end(&mut code);

        // 2. _DrawBS(5, "SO01PB_M", x=-1100, y=200)
        push_int(&mut code, 5);
        push_str(&mut code, s_so);
        for _ in 0..4 { push_int(&mut code, 0); }
        push_int(&mut code, -1100);
        push_int(&mut code, 200);
        for _ in 0..4 { push_int(&mut code, 0); }
        push_int(&mut code, 100);
        push_str(&mut code, s_draw_bs);
        call_fn(&mut code);
        stmt_end(&mut code);

        // 3. _MoveBS(5, dx=+720, y=200, dur=150 at idx 12, 0 at idx 13, -256 at idx 14)
        push_int(&mut code, 5);
        for _ in 0..3 { push_int(&mut code, 0); }
        push_int(&mut code, 268436176); // 0x1000_0000 + 720
        push_int(&mut code, 200);
        push_int(&mut code, 0x1000_0000);
        push_int(&mut code, 0);
        push_int(&mut code, 0x1000_0000);
        push_int(&mut code, -1);
        push_int(&mut code, 1);
        push_int(&mut code, 0);
        push_int(&mut code, 150);
        push_int(&mut code, 0);
        push_int(&mut code, -256);
        push_str(&mut code, s_move_bs);
        call_fn(&mut code);
        stmt_end(&mut code);

        push_str(&mut code, s_spk);
        push_str(&mut code, s_txt);
        code.extend_from_slice(&0x140u32.to_le_bytes());
        stmt_end(&mut code);

        assert!(code.len() <= code_len);
        code.resize(code_len, 0);
        data.extend_from_slice(&code);
        data.extend_from_slice(&strings);

        let script = BgiScript::parse(&data).expect("解析失败");
        let mut vm = VmEngine::new(&script);
        let d1 = vm.next_dialogue().expect("推进失败").unwrap();
        assert_eq!(d1.bg.as_deref(), Some("arev01A"), "事件CG场景名 arev01A 必须被识别");
        assert_eq!(d1.slot_motions.len(), 1);
        let sm = &d1.slot_motions[0];
        assert_eq!(sm.slot_id, 5);
        assert!(!sm.is_repeat);
        assert_eq!(sm.from_x, -1100);
        assert_eq!(sm.to_x, -380);
        assert_eq!(sm.duration_ms, 150, "_MoveBS 耗时必须解析 args[12]=150 而非 args[13]=0");
    }

    fn build_standard_test_script(code: &[u8], strings: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(code);
        data.extend_from_slice(strings);
        data
    }

    fn emit_u32(c: &mut Vec<u8>, v: u32) {
        c.extend_from_slice(&v.to_le_bytes());
    }

    fn emit_op_arg(c: &mut Vec<u8>, op: u32, arg: i32) {
        c.extend_from_slice(&op.to_le_bytes());
        c.extend_from_slice(&(arg as u32).to_le_bytes());
    }

    fn emit_syscall(c: &mut Vec<u8>, op: u32, sys_id: u32) {
        c.extend_from_slice(&0x003fu32.to_le_bytes());
        c.extend_from_slice(&op.to_le_bytes());
        c.extend_from_slice(&sys_id.to_le_bytes());
    }

    #[test]
    fn test_nested_flag_expression_mail_chapter1_arisa() {
        // 复现 mail_chapter1_arisa @ 0x01ec4..0x01ef8:
        // flag[53] = flag[53] + flag[10]
        let mut code = Vec::new();
        // 先设置 flag[53] = 12, flag[10] = 30
        emit_op_arg(&mut code, 0x0000, 53);
        emit_op_arg(&mut code, 0x0000, 12);
        emit_syscall(&mut code, 2, 0x00E0); // SetFlag(53, 12)

        emit_op_arg(&mut code, 0x0000, 10);
        emit_op_arg(&mut code, 0x0000, 30);
        emit_syscall(&mut code, 2, 0x00E0); // SetFlag(10, 30)

        // 嵌套表达式: push 53 -> push 53 -> GetFlag(53) -> push 10 -> GetFlag(10) -> ADD -> SetFlag(53, sum)
        emit_op_arg(&mut code, 0x0000, 53);
        emit_op_arg(&mut code, 0x0000, 53);
        emit_syscall(&mut code, 1, 0x00E1);
        emit_op_arg(&mut code, 0x0000, 10);
        emit_syscall(&mut code, 1, 0x00E1);
        emit_u32(&mut code, 0x0020); // ADD
        emit_syscall(&mut code, 2, 0x00E0); // SetFlag
        emit_u32(&mut code, 0x001B); // RET

        let data = build_standard_test_script(&code, &[]);
        let script = BgiScript::parse(&data).unwrap();
        let mut vm = VmEngine::new(&script);
        assert!(vm.next_dialogue().unwrap().is_none());
        assert_eq!(*vm.state(), VmState::Terminated);
        assert_eq!(vm.get_flag(53), 42, "嵌套 0x003F 严禁误清栈，flag[53] 应为 12 + 30 = 42");
        assert_eq!(vm.eval_stack_len(), 0);
    }

    #[test]
    fn test_sub_ge_eq_and_logical_and_operand_order() {
        // 1. 0x0021 (-): 5 - 12 = -7 (验证 lhs - rhs 顺序及有符号负数)
        // 2. 0x0033 (>=): 10 >= 20 -> 0; 20 >= 10 -> 1; 10 >= 10 -> 1
        // 3. 0x0038 (&&): (1, 0)->0, (0, 1)->0, (0, 0)->0, (2, 3)->1 (证明非 ||)
        let mut code = Vec::new();
        // flag[1] = 5 - 12 = -7
        emit_op_arg(&mut code, 0x0000, 1);
        emit_op_arg(&mut code, 0x0000, 5);
        emit_op_arg(&mut code, 0x0000, 12);
        emit_u32(&mut code, 0x0021);
        emit_syscall(&mut code, 2, 0x00E0);

        // flag[2] = (10 >= 20) = 0
        emit_op_arg(&mut code, 0x0000, 2);
        emit_op_arg(&mut code, 0x0000, 10);
        emit_op_arg(&mut code, 0x0000, 20);
        emit_u32(&mut code, 0x0033);
        emit_syscall(&mut code, 2, 0x00E0);

        // flag[3] = (20 >= 10) = 1
        emit_op_arg(&mut code, 0x0000, 3);
        emit_op_arg(&mut code, 0x0000, 20);
        emit_op_arg(&mut code, 0x0000, 10);
        emit_u32(&mut code, 0x0033);
        emit_syscall(&mut code, 2, 0x00E0);

        // flag[4] = (10 >= 10) = 1
        emit_op_arg(&mut code, 0x0000, 4);
        emit_op_arg(&mut code, 0x0000, 10);
        emit_op_arg(&mut code, 0x0000, 10);
        emit_u32(&mut code, 0x0033);
        emit_syscall(&mut code, 2, 0x00E0);

        // flag[5] = (1 && 0) = 0
        emit_op_arg(&mut code, 0x0000, 5);
        emit_op_arg(&mut code, 0x0000, 1);
        emit_op_arg(&mut code, 0x0000, 0);
        emit_u32(&mut code, 0x0038);
        emit_syscall(&mut code, 2, 0x00E0);

        // flag[6] = (0 && 1) = 0
        emit_op_arg(&mut code, 0x0000, 6);
        emit_op_arg(&mut code, 0x0000, 0);
        emit_op_arg(&mut code, 0x0000, 1);
        emit_u32(&mut code, 0x0038);
        emit_syscall(&mut code, 2, 0x00E0);

        // flag[7] = (2 && 3) = 1
        emit_op_arg(&mut code, 0x0000, 7);
        emit_op_arg(&mut code, 0x0000, 2);
        emit_op_arg(&mut code, 0x0000, 3);
        emit_u32(&mut code, 0x0038);
        emit_syscall(&mut code, 2, 0x00E0);

        emit_u32(&mut code, 0x00F4);

        let data = build_standard_test_script(&code, &[]);
        let script = BgiScript::parse(&data).unwrap();
        let mut vm = VmEngine::new(&script);
        assert!(vm.next_dialogue().unwrap().is_none());
        assert_eq!(vm.get_flag(1), -7);
        assert_eq!(vm.get_flag(2), 0);
        assert_eq!(vm.get_flag(3), 1);
        assert_eq!(vm.get_flag(4), 1);
        assert_eq!(vm.get_flag(5), 0);
        assert_eq!(vm.get_flag(6), 0);
        assert_eq!(vm.get_flag(7), 1);
        assert_eq!(vm.eval_stack_len(), 0);
    }

    #[test]
    fn test_select_ex_frame_guard_no_prompt_and_choice_seq() {
        let code_len = 400usize;
        let mut strings = Vec::new();
        let mut add_str = |s: &str| -> u32 {
            let rel = (code_len + strings.len()) as u32;
            let (gbk, _, _) = encoding_rs::GBK.encode(s);
            strings.extend_from_slice(&gbk);
            strings.push(0);
            rel
        };

        let s_opt0 = add_str("选项A");
        let s_opt1 = add_str("选项B");
        let s_select = add_str("_SelectEx");
        let s_branch0 = add_str("走了A分支");
        let s_branch1 = add_str("走了B分支");

        let mut code = Vec::new();
        // 严格复刻 03_prologue3 / 05_chapter1_2 实证字节码布局 (无标题 prompt = Int(0))
        emit_op_arg(&mut code, 0x0000, 2); // N = 2
        emit_op_arg(&mut code, 0x0002, 0); // local array base
        // opt0
        emit_u32(&mut code, 0x0010);
        emit_op_arg(&mut code, 0x0000, 4);
        emit_u32(&mut code, 0x0020);
        emit_u32(&mut code, 0x0011);
        emit_op_arg(&mut code, 0x0002, 4);
        emit_op_arg(&mut code, 0x0003, s_opt0 as i32);
        emit_op_arg(&mut code, 0x0009, 2);
        // opt1
        emit_u32(&mut code, 0x0010);
        emit_op_arg(&mut code, 0x0000, 4);
        emit_u32(&mut code, 0x0020);
        emit_u32(&mut code, 0x0011);
        emit_op_arg(&mut code, 0x0002, 4);
        emit_op_arg(&mut code, 0x0003, s_opt1 as i32);
        emit_op_arg(&mut code, 0x0009, 2);
        // 无标题 Int(0)
        emit_op_arg(&mut code, 0x0000, 0);
        emit_op_arg(&mut code, 0x0003, s_select as i32);
        emit_u32(&mut code, 0x001C);
        // 出 _SelectEx 后帧恢复: 0x0010 -> 0x0000 8 -> 0x0021 -> 0x0011
        emit_u32(&mut code, 0x0010);
        emit_op_arg(&mut code, 0x0000, 8);
        emit_u32(&mut code, 0x0021);
        emit_u32(&mut code, 0x0011);

        // if flags[0] == 1 { branch1 } else { branch0 }
        emit_op_arg(&mut code, 0x0000, 0);
        emit_syscall(&mut code, 1, 0x00E1); // GetFlag(0)
        emit_op_arg(&mut code, 0x0000, 1);
        emit_u32(&mut code, 0x0030); // ==
        let jz_patch_pos = code.len() + 4;
        emit_op_arg(&mut code, 0x0001, 0); // target_rel placeholder
        emit_op_arg(&mut code, 0x0019, 0); // JZ

        // branch 1 (flags[0] == 1)
        emit_op_arg(&mut code, 0x0003, s_branch1 as i32);
        emit_u32(&mut code, 0x0140);
        emit_u32(&mut code, 0x001B);

        // branch 0
        let branch0_rel = code.len() as u32;
        code[jz_patch_pos..jz_patch_pos + 4].copy_from_slice(&branch0_rel.to_le_bytes());
        emit_op_arg(&mut code, 0x0003, s_branch0 as i32);
        emit_u32(&mut code, 0x0140);
        emit_u32(&mut code, 0x001B);

        assert!(code.len() <= code_len);
        code.resize(code_len, 0);
        let data = build_standard_test_script(&code, &strings);
        let script = BgiScript::parse(&data).unwrap();
        let mut vm = VmEngine::new(&script);

        assert!(vm.next_dialogue().unwrap().is_none());
        match vm.state() {
            VmState::WaitingForChoice {
                prompt, options, ..
            } => {
                assert!(prompt.is_none(), "无标题 Int(0) 时 prompt 应为 None，不可吞末项");
                assert_eq!(options, &vec!["选项A".to_string(), "选项B".to_string()]);
            }
            other => panic!("预期 WaitingForChoice，实际 {:?}", other),
        }
        let seq = vm.choice_seq();
        assert_eq!(seq, 1);
        assert!(!vm.submit_choice(seq + 1, 1), "错误 choice_seq 必须被拒绝");
        assert!(vm.submit_choice(seq, 1), "合法 choice_seq=1 选 1 必须成功");
        assert!(!vm.submit_choice(seq, 0), "重入/连点同一 choice_seq 必须被拒绝");
        assert_eq!(vm.get_flag(0), 1, "选中结果必须写入 flags[0]");

        let dlg = vm.next_dialogue().unwrap().expect("应命中 branch1 对白");
        assert_eq!(dlg.text, "走了B分支");
        assert_eq!(vm.eval_stack_len(), 0);
        assert!(vm.next_dialogue().unwrap().is_none());
        assert_eq!(*vm.state(), VmState::Terminated);
    }

    #[test]
    fn test_halt_prevents_trailing_table_scan_and_loop_breaker() {
        // 1. 0x001B 后紧随未知假操作码/字符串标签表，验证停机防穿表
        let code_len = 120usize;
        let mut strings = Vec::new();
        let rel = code_len as u32;
        strings.extend_from_slice(b"trailing_junk\0");

        let mut code = Vec::new();
        emit_syscall(&mut code, 0, 0x0139);
        emit_u32(&mut code, 0x001B); // RET
        emit_u32(&mut code, 0xDEAD_BEEFu32); // 若穿表会触发未知 opcode panic!
        emit_op_arg(&mut code, 0x0003, rel as i32);
        emit_u32(&mut code, 0x0140);
        code.resize(code_len, 0);

        let data = build_standard_test_script(&code, &strings);
        let script = BgiScript::parse(&data).unwrap();
        let mut vm = VmEngine::new(&script);
        assert!(vm.next_dialogue().unwrap().is_none());
        assert_eq!(*vm.state(), VmState::Terminated);

        // 2. 死循环熔断 MAX_OPS_PER_TICK (50_000) 并返回最近 20 步轨迹
        let mut loop_code = Vec::new();
        emit_syscall(&mut loop_code, 0, 0x0139);
        let loop_rel = loop_code.len() as u32;
        emit_op_arg(&mut loop_code, 0x0001, loop_rel as i32);
        emit_u32(&mut loop_code, 0x0018); // JMP self
        let loop_data = build_standard_test_script(&loop_code, &[]);
        let loop_script = BgiScript::parse(&loop_data).unwrap();
        let mut loop_vm = VmEngine::new(&loop_script);
        let err = loop_vm.step().expect_err("死循环必须触发 MAX_OPS_PER_TICK 熔断错误");
        let msg = format!("{}", err);
        assert!(msg.contains("50000"), "报错应包含熔断上限 50000: {}", msg);
        assert!(msg.contains("最近 20 步轨迹"), "报错应包含最近 20 步轨迹: {}", msg);
    }

    #[test]
    fn test_config_regs_isolated_from_sys_regs_and_hscene_bounds() {
        let mut code = Vec::new();
        // flag[1] = 0xE3(20)
        emit_op_arg(&mut code, 0x0000, 1);
        emit_op_arg(&mut code, 0x0000, 20);
        emit_syscall(&mut code, 1, 0x00E3);
        emit_syscall(&mut code, 2, 0x00E0);

        // flag[2] = 0xE5(0)
        emit_op_arg(&mut code, 0x0000, 2);
        emit_op_arg(&mut code, 0x0000, 0);
        emit_syscall(&mut code, 1, 0x00E5);
        emit_syscall(&mut code, 2, 0x00E0);

        // flag[3] = 0xE5(20) (sys_regs 不含 20，应为 0)
        emit_op_arg(&mut code, 0x0000, 3);
        emit_op_arg(&mut code, 0x0000, 20);
        emit_syscall(&mut code, 1, 0x00E5);
        emit_syscall(&mut code, 2, 0x00E0);

        // flag[4] = 0xE3(0) (config_regs 不含 0，应为 0)
        emit_op_arg(&mut code, 0x0000, 4);
        emit_op_arg(&mut code, 0x0000, 0);
        emit_syscall(&mut code, 1, 0x00E3);
        emit_syscall(&mut code, 2, 0x00E0);

        emit_u32(&mut code, 0x001B);

        let data = build_standard_test_script(&code, &[]);
        let script = BgiScript::parse(&data).unwrap();
        let mut vm = VmEngine::new(&script);

        // 越界值必须拒绝且不污染已有值
        assert!(vm.set_hscene_mode(2));
        assert!(!vm.set_hscene_mode(-1));
        assert!(!vm.set_hscene_mode(3));
        assert_eq!(vm.config_regs().get(&20).copied(), Some(2));

        // 即使写入 sys_regs[20] 也不得影响 0xE3(20)
        vm.set_sys_reg(20, 1);
        // 即使写入 config_regs[0] 也不得影响 0xE5(0)
        vm.set_config_reg(0, 99);

        assert!(vm.next_dialogue().unwrap().is_none());
        assert_eq!(vm.get_flag(1), 2, "0xE3(20) 必须读自 config_regs");
        assert_eq!(
            vm.get_flag(2),
            DEFAULT_IS_DEFAULT_NAME,
            "0xE5(0) 必须读自 sys_regs (默认=0 自定义主角名「李骏」)"
        );
        assert_eq!(vm.get_flag(3), 1, "0xE5(20) 读自 sys_regs");
        assert_eq!(vm.get_flag(4), 99, "0xE3(0) 读自 config_regs");
    }

    #[test]
    fn test_apply_protagonist_name_and_custom_name_branch() {
        // 1. 说话人栏与正文中各级主角名占位符替换（长串优先）
        assert_eq!(apply_protagonist_name("主人公（名）", true), "李骏");
        assert_eq!(apply_protagonist_name("主人公（名）＆空", true), "李骏＆空");
        assert_eq!(
            apply_protagonist_name("「我是主人公（姓）\u{3000}主人公（名），大家请多指教」", true),
            "「我是李骏，大家请多指教」"
        );
        assert_eq!(
            apply_protagonist_name("主人公（姓） 主人公（名） / 主人公（姓）主人公（名）", true),
            "李骏 / 李骏"
        );
        assert_eq!(
            apply_protagonist_name("「不愧是主人公（姓）同学」", true),
            "「不愧是李同学」"
        );

        // 2. 4 处汉化组在 sys_regs[0] == 0 分支漏改台词精准替换（仅在 is_custom_name=true 生效）
        assert_eq!(
            apply_protagonist_name("「我要更加更加喜欢前辈，想要一直呆在隼人君的身边」", true),
            "「我要更加更加喜欢前辈，想要一直呆在前辈的身边」"
        );
        assert_eq!(
            apply_protagonist_name(
                "「不知怎的……今天一整天都能和最喜欢的宇佐喵一起约会……我感觉非常幸福」",
                true
            ),
            "「不知怎的……今天一整天都能和最喜欢的你一起约会……我感觉非常幸福」"
        );
        assert_eq!(
            apply_protagonist_name("「我也是，在家的时候一直都在在意隼人的事……嗯，嗯……！」", true),
            "「我也是，在家的时候一直都在在意你的事……嗯，嗯……！」"
        );
        assert_eq!(
            apply_protagonist_name("「嘿嘿，总觉得隼人像小孩子一样」", true),
            "「嘿嘿，总觉得你像小孩子一样」"
        );
        // is_custom_name=false 时不替换这 4 处短语，且其他包含“隼人”的台词绝不误伤
        assert_eq!(
            apply_protagonist_name("「嘿嘿，总觉得隼人像小孩子一样」", false),
            "「嘿嘿，总觉得隼人像小孩子一样」"
        );
        assert_eq!(
            apply_protagonist_name("「其它台词里的隼人不受影响」", true),
            "「其它台词里的隼人不受影响」"
        );
    }
}
