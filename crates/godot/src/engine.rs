//! Godot 原生引擎节点 PureConnectEngine
//!
//! 继承 Godot `Node` 类，提供完整的 VFS 挂载、剧本驱动、单步/流式对白流与状态查询。
//! 内置 std::panic::catch_unwind 防火墙，严禁跨 FFI Panic。

use std::collections::{HashMap, VecDeque};

use godot::classes::{INode, ImageTexture, Node};
use godot::prelude::*;

use crate::factory::MainThreadFactory;
use crate::protocol::keys;
use crate::runtime::{BgUpdate, PureConnectRuntime, SprUpdate, StepResult};

/// PureConnectEngine 持有 UI 纹理缓存上限（防止 iOS Jetsam 内存超限）
const MAX_UI_TEX_CACHE_ENTRIES: usize = 128;
/// 舞台/立绘/背景纹理缓存上限（避免重复切换立绘表情与背景时反复上传 GPU）
const MAX_STAGE_TEX_CACHE_ENTRIES: usize = 48;

struct BoundedTexCache {
    map: HashMap<String, Gd<ImageTexture>>,
    order: VecDeque<String>,
    cap: usize,
}

impl BoundedTexCache {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::with_capacity(cap),
            cap,
        }
    }

    fn get(&self, key: &str) -> Option<Gd<ImageTexture>> {
        self.map.get(key).cloned()
    }

    fn insert(&mut self, key: String, tex: Gd<ImageTexture>) {
        if self.map.contains_key(&key) {
            self.map.insert(key, tex);
            return;
        }
        while self.map.len() >= self.cap {
            if let Some(old_k) = self.order.pop_front() {
                self.map.remove(&old_k);
            } else {
                break;
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, tex);
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

#[derive(GodotClass)]
#[class(base=Node)]
pub struct PureConnectEngine {
    base: Base<Node>,
    runtime: PureConnectRuntime,
    /// 已上传至 Godot 的 UI/转场纹理缓存，避免界面切换与重复拉取时反复解码和拷贝 RGBA
    ui_tex_cache: BoundedTexCache,
    /// 舞台背景/立绘/头像 GPU 纹理缓存
    stage_tex_cache: BoundedTexCache,
}

#[godot_api]
impl INode for PureConnectEngine {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            runtime: PureConnectRuntime::new(),
            ui_tex_cache: BoundedTexCache::new(MAX_UI_TEX_CACHE_ENTRIES),
            stage_tex_cache: BoundedTexCache::new(MAX_STAGE_TEX_CACHE_ENTRIES),
        }
    }
}

#[godot_api]
impl PureConnectEngine {
    /// 初始化虚拟文件系统 (VFS)，挂载基础资产目录与汉化补丁目录
    #[func]
    pub fn init_vfs(&mut self, base_dir: GString, patch_dir: GString) -> bool {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.ui_tex_cache.clear();
            self.stage_tex_cache.clear();
            self.runtime
                .init_vfs(&base_dir.to_string(), &patch_dir.to_string())
        }));

        match res {
            Ok(Ok(val)) => val,
            Ok(Err(e)) => {
                godot_error!("PureConnectEngine::init_vfs failed: {:?}", e);
                false
            }
            Err(payload) => {
                godot_error!("PureConnectEngine::init_vfs panic caught: {:?}", payload);
                false
            }
        }
    }

    /// 从 VFS 加载指定 BGI 剧本
    #[func]
    pub fn load_script(&mut self, name: GString) -> bool {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.load_script(&name.to_string())
        }));

        match res {
            Ok(Ok(val)) => val,
            Ok(Err(e)) => {
                godot_error!("PureConnectEngine::load_script failed: {:?}", e);
                false
            }
            Err(payload) => {
                godot_error!("PureConnectEngine::load_script panic caught: {:?}", payload);
                false
            }
        }
    }

    /// 流式运行至下一句对白，返回增量 Delta 快照字典
    #[func]
    pub fn next_dialogue(&mut self) -> Dictionary {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.inner_next_dialogue()
        }));

        match res {
            Ok(dict) => dict,
            Err(payload) => {
                godot_error!("PureConnectEngine::next_dialogue panic caught: {:?}", payload);
                Self::fallback_dictionary("Error")
            }
        }
    }

    /// 带单调序列号校验提交分支选择 (0-based)，防止连点或读档前旧 UI 回调误消费
    #[func]
    pub fn submit_choice(&mut self, choice_seq: i64, index: i64) -> bool {
        if choice_seq < 0 {
            return false;
        }
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.submit_choice(choice_seq as u64, index)
        }));

        match res {
            Ok(ok) => ok,
            Err(payload) => {
                godot_error!("PureConnectEngine::submit_choice panic caught: {:?}", payload);
                false
            }
        }
    }

    /// 分支选择输入 (0-based，兼容旧接口)
    #[func]
    pub fn select_choice(&mut self, index: i64) -> bool {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.select_choice(index)
        }));

        match res {
            Ok(ok) => ok,
            Err(payload) => {
                godot_error!("PureConnectEngine::select_choice panic caught: {:?}", payload);
                false
            }
        }
    }

    /// 清空单周目档内变量表（仅在“从头开始”时调用；切下一脚本时保留）
    #[func]
    pub fn reset_session_flags(&mut self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.reset_session_flags();
        }));
    }

    /// 设置系统寄存器（供 0x00E5 / 0x00E7 查询）
    #[func]
    pub fn set_sys_reg(&mut self, id: i64, value: i64) {
        if id < 0 {
            return;
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.set_sys_reg(id as u32, value as i32);
        }));
    }

    /// 专用接口：设置 H 场景射精选项 (0=由选项决定, 1=中出, 2=外射)，内建 0..=2 边界校验
    #[func]
    pub fn set_hscene_mode(&mut self, val: i64) -> bool {
        if !(0..=2).contains(&val) {
            return false;
        }
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.set_hscene_mode(val as i32)
        }));
        match res {
            Ok(ok) => ok,
            Err(payload) => {
                godot_error!("PureConnectEngine::set_hscene_mode panic caught: {:?}", payload);
                false
            }
        }
    }

    /// 获取当前 H 场景选项配置 (0..=2)
    #[func]
    pub fn get_hscene_mode(&self) -> i64 {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.get_hscene_mode() as i64
        }));
        res.unwrap_or(0)
    }

    /// 检查全局 CG 解锁集合是否有新增变更
    #[func]
    pub fn consume_system_flags_dirty(&mut self) -> bool {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.consume_system_flags_dirty()
        }));
        res.unwrap_or(false)
    }

    /// 导出全局系统标志 JSON (user://system_flags.json)
    #[func]
    pub fn get_system_flags_json(&mut self) -> GString {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.export_system_flags_json()
        }));
        match res {
            Ok(s) => GString::from(s.as_str()),
            Err(_) => GString::from("{}"),
        }
    }

    /// 加载全局系统标志 JSON (user://system_flags.json)
    #[func]
    pub fn load_system_flags_json(&mut self, json: GString) -> bool {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.import_system_flags_json(&json.to_string())
        }));
        res.unwrap_or(false)
    }

    /// 获取当前执行引擎状态字符串
    #[func]
    pub fn get_state(&self) -> GString {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.get_state()
        }));

        match res {
            Ok(st) => GString::from(st),
            Err(payload) => {
                godot_error!("PureConnectEngine::get_state panic caught: {:?}", payload);
                GString::from("Error")
            }
        }
    }

    /// 分页拉取历史对白列表 (供 Backlog 弹窗使用，避免表现层双份存储)
    #[func]
    pub fn get_history(&self, offset: i64, limit: i64) -> Array<Dictionary> {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let entries = self.runtime.get_history(offset.max(0) as usize, limit.max(0) as usize);
            let mut arr = Array::<Dictionary>::new();
            for entry in entries {
                let mut d = Dictionary::new();
                d.set("index", entry.index as i64);
                d.set("speaker", entry.speaker.as_deref().unwrap_or(""));
                d.set("text", entry.text.as_str());
                d.set("voice", entry.voice.as_deref().unwrap_or(""));
                arr.push(&d);
            }
            arr
        }));

        match res {
            Ok(arr) => arr,
            Err(payload) => {
                godot_error!("PureConnectEngine::get_history panic caught: {:?}", payload);
                Array::<Dictionary>::new()
            }
        }
    }

    /// 重听指定历史语音，返回 AudioStream
    #[func]
    pub fn play_history_voice(&mut self, voice_asset: GString) -> Variant {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(raw) = self.runtime.get_voice_data(&voice_asset.to_string()) {
                if let Some(stream) = MainThreadFactory::create_ogg_stream(&raw) {
                    return Variant::from(stream);
                }
            }
            Variant::nil()
        }));

        match res {
            Ok(v) => v,
            Err(payload) => {
                godot_error!("PureConnectEngine::play_history_voice panic caught: {:?}", payload);
                Variant::nil()
            }
        }
    }

    /// 按需解码并加载 UI 纹理（带引擎内 Godot Texture 引用缓存，避免重复上传 GPU）
    #[func]
    pub fn load_ui_texture(&mut self, asset_name: GString) -> Variant {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let key = asset_name.to_string().trim().to_ascii_lowercase();
            if let Some(cached) = self.ui_tex_cache.get(&key) {
                return Variant::from(cached);
            }
            if let Some(img) = self.runtime.load_ui_image(&key) {
                if let Some(tex) = MainThreadFactory::create_texture_from_image(&img) {
                    self.ui_tex_cache.insert(key, tex.clone());
                    return Variant::from(tex);
                }
            }
            Variant::nil()
        }));

        match res {
            Ok(v) => v,
            Err(payload) => {
                godot_error!("PureConnectEngine::load_ui_texture panic caught: {:?}", payload);
                Variant::nil()
            }
        }
    }

    /// 通用音频按需加载接口 (如 title_bgm, jingle 等)
    #[func]
    pub fn load_audio(&mut self, asset_name: GString) -> Variant {
        self.play_history_voice(asset_name)
    }

    /// 方案 4A：打开共同线彩蛋信封独立子引擎 (window_Mail)
    #[func]
    pub fn open_mail_window(&mut self) -> Dictionary {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let step = self.runtime.open_mail_window();
            let choice_seq = self.runtime.current_choice_seq();
            Self::step_result_to_dict(step, choice_seq, Some(&mut self.stage_tex_cache))
        }));
        match res {
            Ok(d) => d,
            Err(payload) => {
                godot_error!("PureConnectEngine::open_mail_window panic caught: {:?}", payload);
                Self::fallback_dictionary("Error")
            }
        }
    }

    /// 查询共同线彩蛋信封独立子引擎 (mail_vm) 是否处于打开状态
    #[func]
    pub fn is_mail_window_open(&self) -> bool {
        self.runtime.is_mail_window_open()
    }

    /// 查询当前右下角信封图标四态 (FLAG[200]: 0=熄灭, 1=可发信, 2=未读回信, 3=个人线隐藏)
    #[func]
    pub fn get_mail_btn_state(&self) -> i64 {
        self.runtime.mail_btn_state() as i64
    }

    /// 读取运行期档内标志值（用于验证与状态观测）
    #[func]
    pub fn get_flag(&self, id: i64) -> i64 {
        if id < 0 {
            return 0;
        }
        self.runtime.get_flag(id as u32) as i64
    }

    /// 设置运行期档内标志值（用于自动化验证脚本精准构造共同线信封测试态）
    #[func]
    pub fn set_flag(&mut self, id: i64, value: i64) {
        if id >= 0 {
            self.runtime.set_flag(id as u32, value as i32);
        }
    }

    /// 获取当前可供持久化的完整存档元数据 (Schema v2；mail_vm 活跃期间强拦截返回空字典)
    #[func]
    pub fn get_save_state(&mut self) -> Dictionary {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let Some(v2) = self.runtime.try_build_save_v2() else {
                return Dictionary::new();
            };
            let v2_json = serde_json::to_string(&v2).unwrap_or_else(|_| "{}".to_string());
            let mut dict = Dictionary::new();
            dict.set("save_version", v2.save_version as i64);
            dict.set("script_name", v2.script_name.as_str());
            dict.set("dialogue_index", v2.dialogue_index as i64);
            let mut ch_arr = Array::<i64>::new();
            for c in &v2.choices {
                ch_arr.push(*c as i64);
            }
            dict.set("choices", ch_arr);
            dict.set("speaker", v2.speaker.as_deref().unwrap_or(""));
            if let Some(ref pc_choice) = v2.pending_choice {
                let preview = if v2.text.is_empty() {
                    format!(
                        "[选择中] {}",
                        pc_choice.prompt.as_deref().unwrap_or("请选择分支")
                    )
                } else {
                    format!("[选择中] {}", v2.text)
                };
                dict.set("text", preview.as_str());
            } else {
                dict.set("text", v2.text.as_str());
            }
            dict.set("save_json", v2_json.as_str());
            dict
        }));

        match res {
            Ok(d) => d,
            Err(payload) => {
                godot_error!("PureConnectEngine::get_save_state panic caught: {:?}", payload);
                Dictionary::new()
            }
        }
    }

    /// 根据 Schema v1/v2 JSON 字符串还原完整视听与对白/选项快照字典
    #[func]
    pub fn load_save_state_json(&mut self, json_str: GString) -> Dictionary {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (step, visuals) = self.runtime.load_save_json(&json_str.to_string());
            let choice_seq = self.runtime.current_choice_seq();
            Self::step_with_visuals_to_dict(
                step,
                visuals,
                choice_seq,
                Some(&mut self.stage_tex_cache),
            )
        }));

        match res {
            Ok(d) => d,
            Err(payload) => {
                godot_error!("PureConnectEngine::load_save_state_json panic caught: {:?}", payload);
                Self::fallback_dictionary("Error")
            }
        }
    }

    /// 根据存档快照精确快进并还原完整视听与对白快照字典 (兼容 v1 参数签名)
    #[func]
    pub fn load_save_state(
        &mut self,
        script_name: GString,
        dialogue_index: i64,
        choices: Array<i64>,
    ) -> Dictionary {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let ch_vec: Vec<usize> = choices
                .iter_shared()
                .filter(|&v| v >= 0)
                .map(|v| v as usize)
                .collect();
            let (step, visuals) = self.runtime.load_save_state_with_visuals(
                &script_name.to_string(),
                dialogue_index.max(0) as usize,
                &ch_vec,
            );
            let choice_seq = self.runtime.current_choice_seq();
            Self::step_with_visuals_to_dict(
                step,
                visuals,
                choice_seq,
                Some(&mut self.stage_tex_cache),
            )
        }));

        match res {
            Ok(d) => d,
            Err(payload) => {
                godot_error!("PureConnectEngine::load_save_state panic caught: {:?}", payload);
                Self::fallback_dictionary("Error")
            }
        }
    }
}

impl PureConnectEngine {
    fn write_spr_and_se_to_dict(
        dict: &mut Dictionary,
        spr_update: &SprUpdate,
        se_plays: &[(String, std::sync::Arc<Vec<u8>>)],
        hide_window_when_selecting: bool,
        tex_cache: &mut Option<&mut BoundedTexCache>,
    ) {
        let k = keys();
        match spr_update {
            SprUpdate::Unchanged => {
                dict.set(k.spr_changed.clone(), false);
                dict.set(k.spr_texture.clone(), Variant::nil());
                dict.set(k.spr_asset.clone(), "");
                dict.set(k.spr_fade_ms.clone(), 0i64);
            }
            SprUpdate::Cleared { fade_ms } => {
                dict.set(k.spr_changed.clone(), true);
                dict.set(k.spr_texture.clone(), Variant::nil());
                dict.set(k.spr_asset.clone(), "");
                dict.set(k.spr_fade_ms.clone(), *fade_ms as i64);
            }
            SprUpdate::New {
                asset,
                image,
                fade_ms,
            } => {
                dict.set(k.spr_changed.clone(), true);
                dict.set(k.spr_asset.clone(), asset.as_str());
                dict.set(k.spr_fade_ms.clone(), *fade_ms as i64);
                let cache_key = format!("spr:{}", asset.trim().to_ascii_lowercase());
                let cached = tex_cache.as_deref().and_then(|c| c.get(&cache_key));
                if let Some(tex) = cached {
                    dict.set(k.spr_texture.clone(), tex);
                } else if let Some(tex) = MainThreadFactory::create_texture_from_image(image) {
                    if let Some(c) = tex_cache.as_deref_mut() {
                        c.insert(cache_key, tex.clone());
                    }
                    dict.set(k.spr_texture.clone(), tex);
                } else {
                    dict.set(k.spr_texture.clone(), Variant::nil());
                }
            }
        }

        let mut se_streams = Array::<Variant>::new();
        let mut se_names = Array::<GString>::new();
        for (name, ogg_bytes) in se_plays {
            if let Some(stream) = MainThreadFactory::create_ogg_stream(ogg_bytes) {
                se_streams.push(&Variant::from(stream));
                let s = GString::from(name.as_str());
                se_names.push(&s);
            }
        }
        dict.set(k.se_streams.clone(), se_streams);
        dict.set(k.se_names.clone(), se_names);
        dict.set(k.hide_window_when_selecting.clone(), hide_window_when_selecting);
    }

    fn step_with_visuals_to_dict(
        step: StepResult,
        visuals: Option<crate::runtime::DialogueSnapshot>,
        choice_seq: u64,
        mut tex_cache: Option<&mut BoundedTexCache>,
    ) -> Dictionary {
        if let StepResult::Choice {
            prompt,
            options,
            disabled_options,
            spr_update,
            se_plays,
            hide_window_when_selecting,
            mail_btn_state,
            mail_mode,
        } = step
        {
            let mut dict = if let Some(snap) = visuals {
                Self::step_result_to_dict(
                    StepResult::Dialogue(snap),
                    choice_seq,
                    tex_cache.as_deref_mut(),
                )
            } else {
                Self::fallback_dictionary("WaitingForChoice")
            };
            let k = keys();
            dict.set(k.state.clone(), "WaitingForChoice");
            dict.set(k.choice_prompt.clone(), prompt.as_deref().unwrap_or(""));
            dict.set(k.choice_seq.clone(), choice_seq as i64);
            let mut opt_arr = Array::<GString>::new();
            for opt in options {
                let s = GString::from(opt.as_str());
                opt_arr.push(&s);
            }
            dict.set(k.choice_options.clone(), opt_arr);
            let mut dis_arr = Array::<i64>::new();
            for idx in disabled_options {
                dis_arr.push(idx as i64);
            }
            dict.set(k.disabled_options.clone(), dis_arr);
            dict.set(k.mail_btn_state.clone(), mail_btn_state as i64);
            dict.set(k.mail_mode.clone(), mail_mode);
            dict.set(k.mail_closed.clone(), false);
            if !matches!(spr_update, SprUpdate::Unchanged) || !se_plays.is_empty() {
                Self::write_spr_and_se_to_dict(
                    &mut dict,
                    &spr_update,
                    &se_plays,
                    hide_window_when_selecting,
                    &mut tex_cache,
                );
            } else {
                dict.set(
                    k.hide_window_when_selecting.clone(),
                    hide_window_when_selecting,
                );
            }
            dict
        } else {
            Self::step_result_to_dict(step, choice_seq, tex_cache)
        }
    }

    fn inner_next_dialogue(&mut self) -> Dictionary {
        let step = self.runtime.next_step();
        let choice_seq = self.runtime.current_choice_seq();
        Self::step_result_to_dict(step, choice_seq, Some(&mut self.stage_tex_cache))
    }

    fn step_result_to_dict(
        step: StepResult,
        choice_seq: u64,
        mut tex_cache: Option<&mut BoundedTexCache>,
    ) -> Dictionary {
        let k = keys();
        let mut dict = Dictionary::new();

        match step {
            StepResult::Dialogue(dlg) => {
                dict.set(k.state.clone(), "WaitingForInput");
                dict.set(k.index.clone(), dlg.index as i64);
                dict.set(k.text.clone(), dlg.text.as_str());

                if let Some(ref spk) = dlg.speaker {
                    dict.set(k.speaker.clone(), spk.as_str());
                } else {
                    dict.set(k.speaker.clone(), Variant::nil());
                }

                // 语音流主线程工厂转换
                if let Some(ref ogg_bytes) = dlg.voice_data {
                    if let Some(stream) = MainThreadFactory::create_ogg_stream(ogg_bytes) {
                        dict.set(k.voice_stream.clone(), stream);
                    } else {
                        dict.set(k.voice_stream.clone(), Variant::nil());
                    }
                } else {
                    dict.set(k.voice_stream.clone(), Variant::nil());
                }

                // 背景音乐增量转换
                dict.set(k.bgm_changed.clone(), dlg.bgm_changed);
                if let Some(ref bn) = dlg.bgm_name {
                    dict.set(k.bgm_name.clone(), bn.as_str());
                } else {
                    dict.set(k.bgm_name.clone(), Variant::nil());
                }
                if let Some(ref bdata) = dlg.bgm_data {
                    if let Some(stream) = MainThreadFactory::create_ogg_stream(bdata) {
                        dict.set(k.bgm_stream.clone(), stream);
                    } else {
                        dict.set(k.bgm_stream.clone(), Variant::nil());
                    }
                } else {
                    dict.set(k.bgm_stream.clone(), Variant::nil());
                }

                // 背景纹理主线程工厂转换（带 GPU 缓存与 Mipmap，避免运镜闪烁和重复上传）
                match dlg.bg_update {
                    BgUpdate::New(ref img) => {
                        let cache_k = dlg.bg_key.as_deref().map(|k| format!("bg:{}", k));
                        let cached_tex = cache_k
                            .as_deref()
                            .and_then(|k| tex_cache.as_ref().and_then(|c| c.get(k)));
                        if let Some(tex) = cached_tex {
                            dict.set(k.bg_texture.clone(), tex);
                        } else if let Some(tex) = MainThreadFactory::create_bg_texture_from_image(img) {
                            if let (Some(ck), Some(ref mut c)) = (cache_k, tex_cache.as_deref_mut()) {
                                c.insert(ck, tex.clone());
                            }
                            dict.set(k.bg_texture.clone(), tex);
                        } else {
                            dict.set(k.bg_texture.clone(), Variant::nil());
                        }
                    }
                    BgUpdate::Cleared => {
                        // 场景清空：提供 1x1 全透明纹理，通知表现层擦除旧背景
                        if let Some(tex) =
                            MainThreadFactory::create_solid_color_texture(1, 1, 0, 0, 0, 0)
                        {
                            dict.set(k.bg_texture.clone(), tex);
                        } else {
                            dict.set(k.bg_texture.clone(), Variant::nil());
                        }
                    }
                    BgUpdate::Unchanged => {
                        dict.set(k.bg_texture.clone(), Variant::nil());
                    }
                }

                // 背景位姿与平滑运镜列表
                if let Some(ref pose) = dlg.bg_pose {
                    let mut pd = Dictionary::new();
                    if let Some(px) = pose.x {
                        pd.set("x", px as i64);
                    } else {
                        pd.set("x", Variant::nil());
                    }
                    if let Some(py) = pose.y {
                        pd.set("y", py as i64);
                    } else {
                        pd.set("y", Variant::nil());
                    }
                    pd.set("scale", pose.scale as i64);
                    pd.set("transition_ms", pose.transition_ms as i64);
                    dict.set(k.bg_pose.clone(), pd);
                } else {
                    dict.set(k.bg_pose.clone(), Variant::nil());
                }

                let mut motions_arr = Array::<Dictionary>::new();
                for m in dlg.bg_motions {
                    let mut md = Dictionary::new();
                    if let Some(tx) = m.target_x {
                        md.set("target_x", tx as i64);
                    } else {
                        md.set("target_x", Variant::nil());
                    }
                    if let Some(ty) = m.target_y {
                        md.set("target_y", ty as i64);
                    } else {
                        md.set("target_y", Variant::nil());
                    }
                    md.set("keep_x", m.keep_x);
                    md.set("keep_y", m.keep_y);
                    if let Some(ts) = m.target_scale {
                        md.set("target_scale", ts as i64);
                    } else {
                        md.set("target_scale", Variant::nil());
                    }
                    md.set("duration_ms", m.duration_ms as i64);
                    motions_arr.push(&md);
                }
                dict.set(k.bg_motions.clone(), motions_arr);

                // 立绘槽位主线程工厂转换（命中 GPU 纹理缓存时免拷贝上传）
                let mut changed_dict = Dictionary::new();
                for (slot_id, img) in dlg.changed_slots {
                    let cache_k = dlg.slot_keys.get(&slot_id).map(|s| format!("slot:{}", s));
                    let cached_tex = cache_k
                        .as_deref()
                        .and_then(|ck| tex_cache.as_ref().and_then(|c| c.get(ck)));
                    if let Some(tex) = cached_tex {
                        changed_dict.set(slot_id as i64, tex);
                    } else if let Some(tex) = MainThreadFactory::create_texture_from_image(&img) {
                        if let (Some(ck), Some(ref mut c)) = (cache_k, tex_cache.as_deref_mut()) {
                            c.insert(ck, tex.clone());
                        }
                        changed_dict.set(slot_id as i64, tex);
                    }
                }
                dict.set(k.changed_slots.clone(), changed_dict);

                let mut cleared_arr = Array::<i64>::new();
                for slot_id in dlg.cleared_slots {
                    cleared_arr.push(slot_id as i64);
                }
                dict.set(k.cleared_slots.clone(), cleared_arr);

                let mut slot_tr_dict = Dictionary::new();
                for (sid, tr) in dlg.slot_transforms {
                    let mut td = Dictionary::new();
                    td.set("x", tr.x as i64);
                    td.set("y", tr.y as i64);
                    td.set("scale", tr.scale as i64);
                    td.set("fade_ms", tr.fade_ms as i64);
                    slot_tr_dict.set(sid as i64, td);
                }
                dict.set(k.slot_transforms.clone(), slot_tr_dict);

                let mut slot_mot_arr = Array::<Dictionary>::new();
                for sm in dlg.slot_motions {
                    let mut smd = Dictionary::new();
                    smd.set("slot_id", sm.slot_id as i64);
                    smd.set("is_repeat", sm.is_repeat);
                    if let Some(tx) = sm.target_x {
                        smd.set("target_x", tx as i64);
                    } else {
                        smd.set("target_x", Variant::nil());
                    }
                    if let Some(ty) = sm.target_y {
                        smd.set("target_y", ty as i64);
                    } else {
                        smd.set("target_y", Variant::nil());
                    }
                    smd.set("dx", sm.dx as i64);
                    smd.set("dy", sm.dy as i64);
                    smd.set("from_x", sm.from_x as i64);
                    smd.set("from_y", sm.from_y as i64);
                    smd.set("to_x", sm.to_x as i64);
                    smd.set("to_y", sm.to_y as i64);
                    smd.set("duration_ms", sm.duration_ms as i64);
                    slot_mot_arr.push(&smd);
                }
                dict.set(k.slot_motions.clone(), slot_mot_arr);

                // 头像 FaceAvatar 主线程工厂转换
                dict.set(k.face_changed.clone(), dlg.face_changed);
                if let Some(ref img) = dlg.face_texture {
                    let cached_tex = dlg
                        .face_key
                        .as_deref()
                        .and_then(|fk| tex_cache.as_ref().and_then(|c| c.get(fk)));
                    if let Some(tex) = cached_tex {
                        dict.set(k.face_texture.clone(), tex);
                    } else if let Some(tex) = MainThreadFactory::create_texture_from_image(img) {
                        if let (Some(fk), Some(ref mut c)) = (dlg.face_key, tex_cache.as_deref_mut()) {
                            c.insert(fk, tex.clone());
                        }
                        dict.set(k.face_texture.clone(), tex);
                    } else {
                        dict.set(k.face_texture.clone(), Variant::nil());
                    }
                } else {
                    dict.set(k.face_texture.clone(), Variant::nil());
                }
                let mut f_off = Dictionary::new();
                f_off.set("x", dlg.face_offset.0 as i64);
                f_off.set("y", dlg.face_offset.1 as i64);
                dict.set(k.face_offset.clone(), f_off);

                // 语音资产名称
                if let Some(ref v) = dlg.voice_asset {
                    dict.set(k.voice_asset.clone(), v.as_str());
                } else {
                    dict.set(k.voice_asset.clone(), Variant::nil());
                }

                // 登场特写 (character_intro) 5 图层 + 音效 + jingle 转换
                if let Some(ref intro) = dlg.character_intro {
                    let mut intro_dict = Dictionary::new();
                    intro_dict.set("character", intro.character.as_str());
                    if let Some(ref jingle_bytes) = intro.jingle_data {
                        if let Some(stream) = MainThreadFactory::create_ogg_stream(jingle_bytes) {
                            intro_dict.set("jingle_stream", stream);
                        } else {
                            intro_dict.set("jingle_stream", Variant::nil());
                        }
                    } else {
                        intro_dict.set("jingle_stream", Variant::nil());
                    }
                    if let Some(ref se_bytes) = intro.se_data {
                        if let Some(stream) = MainThreadFactory::create_ogg_stream(se_bytes) {
                            intro_dict.set("se_stream", stream);
                        } else {
                            intro_dict.set("se_stream", Variant::nil());
                        }
                    } else {
                        intro_dict.set("se_stream", Variant::nil());
                    }
                    if let Some(ref card_img) = intro.card_image {
                        if let Some(tex) = MainThreadFactory::create_texture_from_image(card_img) {
                            intro_dict.set("card_texture", tex);
                        } else {
                            intro_dict.set("card_texture", Variant::nil());
                        }
                    } else {
                        intro_dict.set("card_texture", Variant::nil());
                    }
                    let mut layers_arr = Array::<Variant>::new();
                    for layer_opt in &intro.layers {
                        if let Some(ref img) = layer_opt {
                            if let Some(tex) = MainThreadFactory::create_texture_from_image(img) {
                                layers_arr.push(&Variant::from(tex));
                            } else {
                                layers_arr.push(&Variant::nil());
                            }
                        } else {
                            layers_arr.push(&Variant::nil());
                        }
                    }
                    intro_dict.set("layers", layers_arr);
                    dict.set(k.character_intro.clone(), intro_dict);
                } else {
                    dict.set(k.character_intro.clone(), Variant::nil());
                }

                Self::write_spr_and_se_to_dict(
                    &mut dict,
                    &dlg.spr_update,
                    &dlg.se_plays,
                    dlg.hide_window_when_selecting,
                    &mut tex_cache,
                );

                dict.set(k.choice_prompt.clone(), "");
                dict.set(k.choice_options.clone(), Array::<GString>::new());
                dict.set(k.choice_seq.clone(), 0i64);
                dict.set(k.disabled_options.clone(), Array::<i64>::new());
                dict.set(k.mail_btn_state.clone(), dlg.mail_btn_state as i64);
                dict.set(k.mail_on_notify.clone(), dlg.mail_on_notify);
                dict.set(k.mail_mode.clone(), false);
                dict.set(k.mail_closed.clone(), false);
            }
            StepResult::WaitClick(wait_snap) => {
                dict = Self::fallback_dictionary("WaitingForWaitClick");
                Self::write_spr_and_se_to_dict(
                    &mut dict,
                    &wait_snap.spr_update,
                    &wait_snap.se_plays,
                    wait_snap.hide_window_when_selecting,
                    &mut tex_cache,
                );
                dict.set(k.mail_btn_state.clone(), wait_snap.mail_btn_state as i64);
                dict.set(k.mail_mode.clone(), wait_snap.mail_mode);
                dict.set(k.mail_closed.clone(), false);
            }
            StepResult::Choice {
                prompt,
                options,
                disabled_options,
                spr_update,
                se_plays,
                hide_window_when_selecting,
                mail_btn_state,
                mail_mode,
            } => {
                dict = Self::fallback_dictionary("WaitingForChoice");
                dict.set(k.choice_prompt.clone(), prompt.as_deref().unwrap_or(""));
                dict.set(k.choice_seq.clone(), choice_seq as i64);

                let mut opt_arr = Array::<GString>::new();
                for opt in options {
                    let s = GString::from(opt.as_str());
                    opt_arr.push(&s);
                }
                dict.set(k.choice_options.clone(), opt_arr);
                let mut dis_arr = Array::<i64>::new();
                for idx in disabled_options {
                    dis_arr.push(idx as i64);
                }
                dict.set(k.disabled_options.clone(), dis_arr);
                dict.set(k.mail_btn_state.clone(), mail_btn_state as i64);
                dict.set(k.mail_mode.clone(), mail_mode);
                dict.set(k.mail_closed.clone(), false);
                Self::write_spr_and_se_to_dict(
                    &mut dict,
                    &spr_update,
                    &se_plays,
                    hide_window_when_selecting,
                    &mut tex_cache,
                );
            }
            StepResult::MailClosed {
                mail_btn_state,
                spr_fade_ms,
                se_plays,
            } => {
                dict = Self::fallback_dictionary("MailClosed");
                dict.set(k.mail_btn_state.clone(), mail_btn_state as i64);
                dict.set(k.mail_mode.clone(), false);
                dict.set(k.mail_closed.clone(), true);
                dict.set(k.spr_changed.clone(), true);
                dict.set(k.spr_texture.clone(), Variant::nil());
                dict.set(k.spr_asset.clone(), "");
                dict.set(k.spr_fade_ms.clone(), spr_fade_ms as i64);
                let mut se_arr = Array::<Variant>::new();
                for (_name, bytes) in &se_plays {
                    if let Some(s) = MainThreadFactory::create_ogg_stream(bytes) {
                        se_arr.push(&Variant::from(s));
                    }
                }
                dict.set(k.se_streams.clone(), se_arr);
            }
            StepResult::Terminated => {
                dict = Self::fallback_dictionary("Terminated");
            }
            StepResult::Unloaded => {
                dict = Self::fallback_dictionary("NotInitialized");
            }
            StepResult::Ready => {
                dict = Self::fallback_dictionary("Ready");
            }
            StepResult::Error(msg) => {
                godot_error!("PureConnectEngine: 推进出错: {}", msg);
                dict = Self::fallback_dictionary("Error");
            }
        }

        dict
    }

    fn fallback_dictionary(state: &str) -> Dictionary {
        let k = keys();
        let mut dict = Dictionary::new();
        dict.set(k.state.clone(), state);
        dict.set(k.index.clone(), 0i64);
        dict.set(k.text.clone(), "");
        dict.set(k.speaker.clone(), "");
        dict.set(k.voice_stream.clone(), Variant::nil());
        dict.set(k.bgm_changed.clone(), false);
        dict.set(k.bgm_name.clone(), Variant::nil());
        dict.set(k.bgm_stream.clone(), Variant::nil());
        dict.set(k.bg_texture.clone(), Variant::nil());
        dict.set(k.bg_pose.clone(), Variant::nil());
        dict.set(k.bg_motions.clone(), Array::<Dictionary>::new());
        dict.set(k.changed_slots.clone(), Dictionary::new());
        dict.set(k.cleared_slots.clone(), Array::<i64>::new());
        dict.set(k.slot_transforms.clone(), Dictionary::new());
        dict.set(k.slot_motions.clone(), Array::<Dictionary>::new());
        dict.set(k.face_changed.clone(), false);
        dict.set(k.face_texture.clone(), Variant::nil());
        dict.set(k.face_offset.clone(), Variant::nil());
        dict.set(k.voice_asset.clone(), Variant::nil());
        dict.set(k.character_intro.clone(), Variant::nil());
        dict.set(k.choice_prompt.clone(), "");
        dict.set(k.choice_options.clone(), Array::<GString>::new());
        dict.set(k.choice_seq.clone(), 0i64);
        dict.set(k.spr_changed.clone(), false);
        dict.set(k.spr_texture.clone(), Variant::nil());
        dict.set(k.spr_asset.clone(), "");
        dict.set(k.spr_fade_ms.clone(), 0i64);
        dict.set(k.se_streams.clone(), Array::<Variant>::new());
        dict.set(k.se_names.clone(), Array::<GString>::new());
        dict.set(k.hide_window_when_selecting.clone(), false);
        dict.set(k.disabled_options.clone(), Array::<i64>::new());
        dict.set(k.mail_btn_state.clone(), 0i64);
        dict.set(k.mail_on_notify.clone(), false);
        dict.set(k.mail_mode.clone(), false);
        dict.set(k.mail_closed.clone(), false);
        dict
    }
}
