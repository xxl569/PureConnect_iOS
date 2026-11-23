# main.gd
# 《PURE×CONNECT》Galgame 原生移植 Godot 4.3 表现层驱动脚本
# 采用单向数据流与增量 Delta 快照架构，驱动舞台背景、立绘槽位、UI对白与音频播放。
# 本地网页协同定判定型：Step 1~5 冰雪磨砂底图、Quick Menu、FaceAvatar 差分平滑淡入、打字机四态锁、Backlog 分页与 Audio Ducking。

extends Control

const TitleScreenScript = preload("res://scripts/title_screen.gd")
const SettingsPanelScript = preload("res://scripts/settings_panel.gd")
const CharacterIntroScript = preload("res://scripts/character_intro_layer.gd")

# UI 与舞台节点引用
@onready var background: TextureRect = $Background
@onready var character_slots: Control = $CharacterSlots
@onready var dialogue_box: PanelContainer = $DialogueBox
@onready var margin_container: MarginContainer = $DialogueBox/Margin
@onready var speaker_label: Label = find_child("SpeakerLabel", true, false)
@onready var text_label: RichTextLabel = find_child("TextLabel", true, false)
@onready var choice_box: PanelContainer = $ChoiceBox
@onready var choice_prompt_label: Label = $ChoiceBox/Margin/ChoiceVBox/PromptLabel
@onready var choice_options_container: VBoxContainer = $ChoiceBox/Margin/ChoiceVBox/OptionsContainer
@onready var voice_player: AudioStreamPlayer = $VoicePlayer
@onready var bgm_player: AudioStreamPlayer = $BgmPlayer
@onready var se_player: AudioStreamPlayer = get_node_or_null("SePlayer")
@onready var amb_player: AudioStreamPlayer = get_node_or_null("AmbiencePlayer")
@onready var bgm_osd_label: Label = get_node_or_null("BgmOsdLabel")

# 扩展视觉与交互组件
@onready var spr_overlay_layer: Control = get_node_or_null("SprOverlayLayer")
@onready var spr_overlay: TextureRect = get_node_or_null("SprOverlayLayer/SprOverlay")
@onready var spr_overlay_prev: TextureRect = get_node_or_null("SprOverlayLayer/SprOverlayPrev")
@onready var transition_overlay: ColorRect = get_node_or_null("TransitionOverlay")
@onready var face_avatar_layer: Control = get_node_or_null("FaceAvatarLayer")
@onready var face_avatar: TextureRect = get_node_or_null("FaceAvatarLayer/FaceAvatar")
@onready var face_avatar_prev: TextureRect = get_node_or_null("FaceAvatarLayer/FaceAvatarPrev")
@onready var click_icon: TextureRect = get_node_or_null("DialogueBox/OverlayLayer/ClickIcon")
@onready var backlog_panel: Control = get_node_or_null("BacklogPanel")
@onready var history_list: VBoxContainer = get_node_or_null("BacklogPanel/ScrollContainer/HistoryList")
@onready var close_backlog_btn: Button = get_node_or_null("BacklogPanel/HeaderPanel/HeaderMargin/HeaderHBox/CloseBacklogBtn")
@onready var quick_notify: Label = get_node_or_null("QuickNotify")

# Quick Menu 按钮引用
@onready var btn_qsave: TextureButton = find_child("BtnQSave", true, false)
@onready var btn_qload: TextureButton = find_child("BtnQLoad", true, false)
@onready var btn_log: TextureButton = find_child("BtnLog", true, false)
@onready var btn_auto: TextureButton = find_child("BtnAuto", true, false)
@onready var btn_skip: TextureButton = find_child("BtnSkip", true, false)
@onready var btn_config: TextureButton = find_child("BtnConfig", true, false)
@onready var btn_hide: TextureButton = find_child("BtnHide", true, false)

# 原生 GDExtension 引擎节点
var engine: PureConnectEngine = null

# 立绘 Slot 映射 (1..10 -> TextureRect) 与同级双缓冲 SlotPrev 映射；extra_slots 支持 Slot 0 (如 Shovel_03)
var slots: Dictionary = {}
var slot_prevs: Dictionary = {}
var extra_slots: Dictionary = {}
var extra_slot_prevs: Dictionary = {}

# 系统设置面板与持久化配置 (user://settings.cfg)
const SETTINGS_PATH: String = "user://settings.cfg"
const SYSTEM_FLAGS_PATH: String = "user://system_flags.json"
var settings_panel: Control = null
var _setting_bgm_vol: float = 0.85
var _setting_se_vol: float = 0.90
var _setting_amb_vol: float = 0.80
var _setting_voice_vol: float = 1.0
var _setting_char_vols: Dictionary = {}
var _setting_text_speed: float = 1.0
var _setting_auto_speed: float = 1.0
var _setting_resolution_idx: int = 1
var _setting_screen_mode: int = 0
var _setting_full_ratio: int = 0
var _setting_fullscreen: bool = false
var _setting_window_alpha: float = 0.88
var _setting_bgm_osd_mode: int = 1
var _setting_effect_mode: int = 0
var _setting_font_idx: int = 0
var _setting_auto_stop_on_click: bool = true
var _setting_skip_continue_mode: int = 0
var _setting_skip_cond: int = 0
var _setting_skip_stop_on_choice: bool = true
var _setting_hscene_mode: int = 0
var _setting_voice_stop_on_next: bool = false
var _setting_mouse_hide: int = 2
var _setting_right_click_action: int = 0
var _setting_wheel_up_action: int = 0
var _setting_wheel_down_action: int = 0
var _setting_bg_run: bool = true
var _setting_qsave_confirm: bool = false
var _setting_qload_confirm: bool = true
var _setting_text_color: Color = Color.WHITE
var _setting_shortcuts: Array = ["Ctrl", "H", "R", "A", "F", "O", "K", "Tab", "S", "L", "T", "E"]
var _font_cache: Dictionary = {}
var _cached_scene_thumb: Image = null
var _mouse_idle_timer: float = 0.0
var _mouse_hidden_by_idle: bool = false

# 状态标记
var _is_waiting_for_choice: bool = false
var _is_waiting_for_wait_click: bool = false
var _wait_click_seq: int = 0
var _pending_choice_seq: int = 0
var _is_terminated: bool = false
var _is_ready: bool = false
var _sys_font: Font = null
var _custom_font: Font = null

# 独立 SPR 覆盖层（手机短信等 slot 0）与方案 3B 右侧避让排版状态
var _spr_tween: Tween = null
var _spr_tween_seq: int = 0
var _active_spr_asset: String = ""
var _hide_window_when_selecting: bool = false
var _choice_box_right_shifted: bool = false

# 共同线彩蛋信封 HUD (48x48 四态图标 + mail_on_A 5帧气泡) 与方案 4A 独立子引擎状态
const MAIL_BTN_POS_16_9: Vector2 = Vector2(1216.0, 616.0)
const MAIL_BTN_POS_4_3: Vector2 = Vector2(1064.0, 616.0)
const MAIL_BUBBLE_OFFSET: Vector2 = Vector2(-147.0, -58.0)
const MAIL_BUBBLE_DURATIONS: Array[float] = [5.0, 0.1, 0.1, 0.1, 0.1]
var _is_mail_mode: bool = false
var _mail_btn_state: int = 0
var _mail_hud_layer: Control = null
var _mail_btn: TextureButton = null
var _mail_on_bubble: TextureRect = null
var _mail_bubble_timer: Timer = null
var _mail_bubble_frame_idx: int = -1
var _mail_btn_textures: Dictionary = {}
var _mail_on_frames: Array[Texture2D] = []
var _mail_pulse_tween: Tween = null
var _mail_bubble_seq: int = 0
var _mail_btn_hovered: bool = false

# 主标题画面 TitleScreen
var title_screen: Control = null
var _is_in_title_screen: bool = false

# 新角色登场介绍特写层 CharacterIntroLayer (0x00F0)
var char_intro_layer: Control = null
var intro_card_rect: TextureRect = null
var intro_name_lbl: Label = null
var intro_romaji_lbl: Label = null
var intro_motto_lbl: Label = null
var intro_tag_lbl: Label = null
var intro_card_panel: Control = null
var intro_jingle_player: AudioStreamPlayer = null
var _is_in_character_intro: bool = false
var _intro_start_frame: int = -1
var _seen_character_intros: Dictionary = {}
var _intro_tween: Tween = null
var _current_intro_snapshot: Dictionary = {}

# 打字机流式动效与四态输入锁
var _typing_tween: Tween = null
var _is_typing: bool = false
var _current_full_text: String = ""

# 自动模式与快进模式
var _is_auto_active: bool = false
var _is_skip_active: bool = false

# 纯净赏析隐藏 UI 模式
var _is_ui_hidden: bool = false

# 双播放器乒乓交替语音短淡出 (物理隔绝 0ms 硬切咔哒声与旧回调反杀竞态)
const VOICE_FADE_OUT_SEC: float = 0.02
var voice_player_alt: AudioStreamPlayer = null
var _voice_players: Array[AudioStreamPlayer] = []
var _active_voice_idx: int = 0
var _voice_fade_tweens: Array = [null, null]
var _current_voice_asset: String = ""

# 失焦暂停保护 (FocusGuard 常驻 PROCESS_MODE_ALWAYS 接收回焦通知，绝不被 paused=true 阻塞)
class FocusGuardNode extends Node:
	var owner_main: Control = null
	func _ready() -> void:
		process_mode = Node.PROCESS_MODE_ALWAYS
	func _notification(what: int) -> void:
		if owner_main == null:
			return
		if what == NOTIFICATION_APPLICATION_FOCUS_OUT:
			owner_main._on_app_focus_out()
		elif what == NOTIFICATION_APPLICATION_FOCUS_IN:
			owner_main._on_app_focus_in()

var _focus_guard: Node = null
var _is_window_focused: bool = true
var _is_focus_paused: bool = false
var _focus_paused_players: Array[AudioStreamPlayer] = []
var _focus_paused_tweens: Array[Tween] = []

# 4:3 模式左右纯黑遮罩层 (1280x720 底层下裁出中央 960x720 可见区并拦截黑边点击)
var _pillarbox_layer: Control = null
var _pillarbox_left: ColorRect = null
var _pillarbox_right: ColorRect = null

# Audio Ducking 状态管理
var _duck_tween: Tween = null
var _normal_bgm_volume_db: float = 0.0
var _ducked_bgm_volume_db: float = -6.0
var _current_bgm_name: String = ""
var _bgm_osd_tween: Tween = null

# 快速保存状态标识
var _quick_saved_index: int = -1

# 预加载的运行时离线贴图缓存
var _default_moemi_texture: Texture2D = null

# PC 原版实测对齐动效常量 (Sprint 6A 毫秒级规范)
const SCENE_FADE_OUT_SEC: float = 1.0
const SCENE_HOLD_BLACK_SEC: float = 0.5
const SCENE_FADE_IN_SEC: float = 0.5
const SPRITE_FADE_SEC: float = 0.2

# 场景黑场转场、背景平移动画与立绘 Tween 管理
var _scene_tween: Tween = null
var _bg_pan_tween: Tween = null
var _face_tween: Tween = null
var _notify_tween: Tween = null
var _title_fade_tween: Tween = null
var _is_transitioning: bool = false
var _pending_snapshot: Dictionary = {}
var _slot_tweens: Dictionary = {}
var _slot_move_tweens: Dictionary = {}

# 默认路径与剧本常量
const DEFAULT_ARCHIVE_DIR: String = "U:/anti/projects/PURExCONNECT/Archive"
const DEFAULT_PATCH_DIR: String = "U:/anti/temp/exe_unpacked"
const DEFAULT_SCRIPT: String = "01_prologue1"

var _boot_config: Dictionary = {}
var _is_vfs_initialized: bool = false
var _byoa_guide_overlay: Control = null
var _byoa_status_label: Label = null

func _ready() -> void:
	if _is_ready:
		return
	_is_ready = true

	# 初始化跨平台中文字体
	_init_system_font()

	# 加载并应用系统设置 (user://settings.cfg)
	_load_settings()

	# 运行时动态装配高清磨砂冰雪贴图与 QuickMenu 资源
	_setup_ui_textures()

	# 初始化立绘 Slot 槽位映射
	_setup_character_slots()

	# 确保 TransitionOverlay 全屏转场遮罩节点就绪 (位于舞台之上、UI层之下)
	if transition_overlay == null:
		transition_overlay = get_node_or_null("TransitionOverlay")
	if transition_overlay == null:
		transition_overlay = ColorRect.new()
		transition_overlay.name = "TransitionOverlay"
		transition_overlay.color = Color(0, 0, 0, 1)
		transition_overlay.mouse_filter = Control.MOUSE_FILTER_IGNORE
		transition_overlay.visible = false
		transition_overlay.modulate.a = 0.0
		transition_overlay.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		add_child(transition_overlay)
		if dialogue_box:
			move_child(transition_overlay, dialogue_box.get_index())
	else:
		transition_overlay.mouse_filter = Control.MOUSE_FILTER_IGNORE
		transition_overlay.visible = false
		transition_overlay.modulate.a = 0.0

	# 初始化 Quick Menu 信号与交互绑定
	_setup_quick_menu()

	# 初始化新角色登场特写层
	_setup_character_intro_layer()

	# 初始化 ClickIcon 呼吸动效
	_setup_click_icon_animation()

	# 确保 BacklogPanel 层级位于 4:3 遮罩之上
	if backlog_panel:
		backlog_panel.z_index = 20

	# 初始化 4:3 左右黑边遮罩与常驻失焦监控节点
	_setup_pillarbox_overlay()
	_setup_focus_guard()

	# 初始化双播放器乒乓语音通道与 Audio Ducking 信号绑定
	_setup_voice_players()

	# 检查 GDExtension 类是否存在
	if not ClassDB.class_exists("PureConnectEngine"):
		var err_msg = "错误：PureConnectEngine GDExtension 原生类未加载！\n请检查 target/debug/pure_connect_godot.dll 是否已成功编译。"
		printerr(err_msg)
		if text_label:
			text_label.text = "[color=red]" + err_msg + "[/color]"
		if speaker_label:
			speaker_label.visible = false
		return

	# 实例化并挂载引擎原生节点
	engine = PureConnectEngine.new()
	add_child(engine)
	print("[PureConnect] PureConnectEngine 节点挂载成功，初始状态: ", engine.get_state())

	# 解析命令行参数（支持快速直跳剧本或自定义路径调试）
	_boot_config = _parse_command_line_args()

	# 初始化虚拟文件系统 (VFS)，支持 iOS user:// (Documents) 外置资产自动探测与缺失指引
	if not _try_init_vfs_and_boot():
		_show_byoa_guide_overlay()
		return

func _dir_has_arc_files(dir_path: String) -> bool:
	if dir_path.is_empty() or not DirAccess.dir_exists_absolute(dir_path):
		return false
	var da := DirAccess.open(dir_path)
	if da == null:
		return false
	da.list_dir_begin()
	var fname := da.get_next()
	while fname != "":
		if not da.current_is_dir() and fname.to_lower().ends_with(".arc"):
			da.list_dir_end()
			return true
		fname = da.get_next()
	da.list_dir_end()
	return false

func _probe_byoa_root(root_dir: String) -> Dictionary:
	if root_dir.is_empty() or not DirAccess.dir_exists_absolute(root_dir):
		return {}
	var arch := root_dir.path_join("Archive")
	if not _dir_has_arc_files(arch):
		var arch_lower := root_dir.path_join("archive")
		if _dir_has_arc_files(arch_lower):
			arch = arch_lower
		elif _dir_has_arc_files(root_dir):
			arch = root_dir
		else:
			return {}
	var patch := ""
	for pname in ["patch", "Patch", "exe_unpacked"]:
		var cand := root_dir.path_join(pname)
		if DirAccess.dir_exists_absolute(cand):
			patch = cand
			break
	return {"archive_dir": arch, "patch_dir": patch}

func _resolve_byoa_paths(config: Dictionary) -> Dictionary:
	# 1. 命令行显式指定了路径时优先直用
	if config.get("explicit_paths", false):
		return {
			"archive_dir": str(config.get("archive_dir", "")),
			"patch_dir": str(config.get("patch_dir", "")),
		}

	# 2. 环境变量覆盖（便于本地模拟 iOS BYOA 根目录与缺失指引闭环）
	var env_root := OS.get_environment("PURE_CONNECT_BYOA_ROOT").strip_edges()
	if not env_root.is_empty():
		var env_hit := _probe_byoa_root(env_root)
		if not env_hit.is_empty():
			return env_hit
		return {"archive_dir": env_root.path_join("Archive"), "patch_dir": env_root.path_join("patch")}

	# 3. iOS（或桌面默认目录不存在时）按严格优先级探测 user://（在 iOS 上即 <App沙盒>/Documents）
	var user_dir := OS.get_user_data_dir()
	var is_ios := OS.get_name() == "iOS"
	if is_ios or not _dir_has_arc_files(str(config.get("archive_dir", ""))):
		var roots: Array[String] = [
			user_dir,
			user_dir.path_join("PURExCONNECT_iOS_BYOA"),
			user_dir.path_join("PURExCONNECT"),
			user_dir.path_join("Documents"),
		]
		for r in roots:
			var hit := _probe_byoa_root(r)
			if not hit.is_empty():
				return hit
		if is_ios:
			return {"archive_dir": "", "patch_dir": ""}

	# 4. PC 桌面默认路径
	return {
		"archive_dir": str(config.get("archive_dir", DEFAULT_ARCHIVE_DIR)),
		"patch_dir": str(config.get("patch_dir", DEFAULT_PATCH_DIR)),
	}

func _try_init_vfs_and_boot() -> bool:
	if engine == null or _is_vfs_initialized:
		return _is_vfs_initialized

	var paths := _resolve_byoa_paths(_boot_config)
	var archive_dir := str(paths.get("archive_dir", ""))
	var patch_dir := str(paths.get("patch_dir", ""))
	print("[PureConnect] 正在初始化 VFS: Archive=%s, Patch=%s" % [archive_dir, patch_dir])
	if archive_dir.is_empty() or not _dir_has_arc_files(archive_dir):
		printerr("[PureConnect] 未检测到有效外置资产包（Archive/*.arc）")
		if _byoa_status_label and is_instance_valid(_byoa_status_label):
			_byoa_status_label.text = "暂未检测到 Archive/*.arc，请确认已将资产导入应用目录后重试。"
		return false

	var vfs_ok = engine.init_vfs(archive_dir, patch_dir)
	if not vfs_ok:
		var err_msg = "错误：初始化 VFS 失败！\n请确认基础资产目录与汉化补丁目录路径是否存在。"
		printerr(err_msg)
		if _byoa_status_label and is_instance_valid(_byoa_status_label):
			_byoa_status_label.text = err_msg
		return false

	_is_vfs_initialized = true
	print("[PureConnect] VFS 挂载成功！")

	if _byoa_guide_overlay and is_instance_valid(_byoa_guide_overlay):
		_byoa_guide_overlay.queue_free()
		_byoa_guide_overlay = null
		_byoa_status_label = null

	# 将已就绪的 VFS 引擎注入设置与标题子控制器，确保能直接解码 sysgrp.arc / syssnd.arc 原版皮肤
	_setup_mail_hud()
	_setup_settings_panel()
	_apply_settings()
	_load_global_system_flags()

	# 加载初始主线剧本
	var script_name := str(_boot_config.get("script_name", DEFAULT_SCRIPT))
	print("[PureConnect] 正在加载剧本: %s" % script_name)
	var load_ok = engine.load_script(script_name)
	if not load_ok:
		var err_msg = "错误：加载剧本 [%s] 失败！\n请确认该剧本是否存在于补丁或封包中。" % script_name
		printerr(err_msg)
		if text_label:
			text_label.text = "[color=red]" + err_msg + "[/color]"
		if speaker_label:
			speaker_label.visible = false
		return false
	print("[PureConnect] 剧本 [%s] 加载成功，进入主循环！" % script_name)

	# 检查是否直跳演播 (命令行指定剧本或无头自动化测试)，否则展示主标题画面
	if _boot_config.get("is_auto_play", false):
		advance_dialogue()
	else:
		_show_title_screen()
	return true

func _show_byoa_guide_overlay() -> void:
	if _byoa_guide_overlay and is_instance_valid(_byoa_guide_overlay):
		_byoa_guide_overlay.visible = true
		return
	var overlay := ColorRect.new()
	overlay.name = "ByoaGuideOverlay"
	overlay.color = Color(0.06, 0.08, 0.14, 0.96)
	overlay.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	overlay.z_index = 900

	var center := CenterContainer.new()
	center.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	overlay.add_child(center)

	var panel := PanelContainer.new()
	panel.custom_minimum_size = Vector2(920, 460)
	center.add_child(panel)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 36)
	margin.add_theme_constant_override("margin_right", 36)
	margin.add_theme_constant_override("margin_top", 28)
	margin.add_theme_constant_override("margin_bottom", 28)
	panel.add_child(margin)

	var vbox := VBoxContainer.new()
	vbox.add_theme_constant_override("separation", 18)
	margin.add_child(vbox)

	var title := Label.new()
	title.text = "《PURE×CONNECT》外置游戏资产未就绪 (BYOA)"
	title.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	title.add_theme_font_size_override("font_size", 30)
	vbox.add_child(title)

	var body := Label.new()
	body.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	body.add_theme_font_size_override("font_size", 21)
	body.text = (
		"请将整理好的 PURExCONNECT_iOS_BYOA 内文件导入本应用目录后点击下方按钮：\n\n"
		+ "1. 手机自带「文件」App →「我的 iPhone」→「PureConnect」\n"
		+ "   （或电脑爱思助手 → 应用游戏 → PureConnect → 浏览 → Documents/）\n"
		+ "2. 放入 Archive/（58 个 .arc）、patch/（汉化补丁）与 5 个 sys*.arc\n"
		+ "   （也支持直接放入整个 PURExCONNECT_iOS_BYOA 文件夹）\n"
		+ "3. 导入完成后切回本应用将自动重扫，或手动点击下方按钮挂载。"
	)
	vbox.add_child(body)

	var status := Label.new()
	status.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	status.add_theme_font_size_override("font_size", 18)
	status.modulate = Color(1.0, 0.85, 0.45, 1.0)
	status.text = "当前扫描路径: " + OS.get_user_data_dir()
	vbox.add_child(status)
	_byoa_status_label = status

	var btn_box := CenterContainer.new()
	vbox.add_child(btn_box)

	var retry_btn := Button.new()
	retry_btn.name = "ByoaRetryButton"
	retry_btn.text = "重新扫描并挂载资产"
	retry_btn.custom_minimum_size = Vector2(300, 56)
	retry_btn.add_theme_font_size_override("font_size", 22)
	retry_btn.pressed.connect(_try_init_vfs_and_boot)
	btn_box.add_child(retry_btn)

	add_child(overlay)
	_byoa_guide_overlay = overlay

func _load_runtime_texture(res_path: String) -> Texture2D:
	if ResourceLoader.exists(res_path):
		var res = load(res_path)
		if res is Texture2D:
			return res
	var global_path = ProjectSettings.globalize_path(res_path)
	if FileAccess.file_exists(global_path):
		var img = Image.load_from_file(global_path)
		if img and not img.is_empty():
			return ImageTexture.create_from_image(img)
	return null

func _setup_ui_textures() -> void:
	# 1. 对话框底图：原版 1280x225 冰雪磨砂渐变底图
	var dlg_bg = _load_runtime_texture("res://assets/ui/dialogue_box_bg.png")
	if dlg_bg and dialogue_box:
		var style = StyleBoxTexture.new()
		style.texture = dlg_bg
		dialogue_box.add_theme_stylebox_override("panel", style)

	# 2. 点击等待指示光标
	if click_icon:
		var icon_tex = _load_runtime_texture("res://assets/ui/click_icon.png")
		if icon_tex:
			click_icon.texture = icon_tex

	# 3. 预载萌美半身像回退资产
	_default_moemi_texture = _load_runtime_texture("res://assets/ui/face_moemi.png")

	# 4. Quick Menu 7 按钮三态材质装配
	_apply_btn_textures(btn_qsave, "btn_qsave")
	_apply_btn_textures(btn_qload, "btn_qload")
	_apply_btn_textures(btn_log, "btn_log")
	_apply_btn_textures(btn_auto, "btn_auto")
	_apply_btn_textures(btn_skip, "btn_skip")
	_apply_btn_textures(btn_config, "btn_config")
	_apply_btn_textures(btn_hide, "btn_hide")

func _apply_btn_textures(btn: TextureButton, btn_name: String) -> void:
	if btn == null:
		return
	var tex_n = _load_runtime_texture("res://assets/ui/%s_normal.png" % btn_name)
	var tex_h = _load_runtime_texture("res://assets/ui/%s_hover.png" % btn_name)
	var tex_p = _load_runtime_texture("res://assets/ui/%s_pressed.png" % btn_name)
	if tex_n:
		btn.texture_normal = tex_n
	if tex_h:
		btn.texture_hover = tex_h
	if tex_p:
		btn.texture_pressed = tex_p

func _init_system_font() -> void:
	# 优先加载嵌入式开源中文字体 (NotoSansCJKsc-Regular.otf，保障 iOS / 移动端完全杜绝豆腐块与系统字体缺失)
	var font_res_path = "res://assets/fonts/NotoSansCJKsc-Regular.otf"
	var global_font_path = ProjectSettings.globalize_path(font_res_path)
	if FileAccess.file_exists(font_res_path) or FileAccess.file_exists(global_font_path):
		var dynamic_font = FontFile.new()
		var err = dynamic_font.load_dynamic_font(font_res_path)
		if err == OK:
			_custom_font = dynamic_font
		else:
			var err_global = dynamic_font.load_dynamic_font(global_font_path)
			if err_global == OK:
				_custom_font = dynamic_font

	if _custom_font == null:
		var sf = SystemFont.new()
		sf.font_names = PackedStringArray([
			"Microsoft YaHei",
			"SimHei",
			"PingFang SC",
			"Noto Sans CJK SC",
			"sans-serif"
		])
		_custom_font = sf

	_sys_font = _custom_font
	_font_cache[0] = _custom_font

	var sf_yahei := SystemFont.new()
	sf_yahei.font_names = PackedStringArray(["Microsoft YaHei", "SimHei", "PingFang SC", "sans-serif"])
	_font_cache[1] = sf_yahei

	var sf_serif := SystemFont.new()
	sf_serif.font_names = PackedStringArray(["SimSun", "NSimSun", "Noto Serif CJK SC", "Songti SC", "serif"])
	_font_cache[2] = sf_serif

	_apply_dialogue_font()
	if quick_notify and _custom_font:
		quick_notify.add_theme_font_override("font", _custom_font)

func _apply_dialogue_font() -> void:
	var target_font: Font = _font_cache.get(_setting_font_idx, _custom_font)
	if target_font == null:
		target_font = _custom_font
	if speaker_label and target_font:
		speaker_label.add_theme_font_override("font", target_font)
	if text_label and target_font:
		text_label.add_theme_font_override("normal_font", target_font)

func _wake_mouse_cursor() -> void:
	_mouse_idle_timer = 0.0
	if _mouse_hidden_by_idle or Input.get_mouse_mode() == Input.MOUSE_MODE_HIDDEN:
		Input.set_mouse_mode(Input.MOUSE_MODE_VISIBLE)
		_mouse_hidden_by_idle = false

func _should_suspend_mouse_hide() -> bool:
	if _setting_mouse_hide == 2:
		return true
	if _is_in_title_screen or _is_waiting_for_choice or _is_focus_paused or not _is_window_focused:
		return true
	if settings_panel != null and settings_panel.visible:
		return true
	if backlog_panel != null and backlog_panel.visible:
		return true
	return false

func _input(event: InputEvent) -> void:
	if event == null or event.device == InputEvent.DEVICE_ID_EMULATION:
		return
	if event is InputEventMouseMotion or event is InputEventMouseButton:
		_wake_mouse_cursor()

func _process(delta: float) -> void:
	if DisplayServer.get_name() == "headless" or OS.get_name() in ["iOS", "Android"]:
		return
	if _should_suspend_mouse_hide():
		_wake_mouse_cursor()
		return
	var hide_limit: float = 5.0 if _setting_mouse_hide == 0 else 10.0
	_mouse_idle_timer += delta
	if _mouse_idle_timer >= hide_limit and not _mouse_hidden_by_idle:
		Input.set_mouse_mode(Input.MOUSE_MODE_HIDDEN)
		_mouse_hidden_by_idle = true

func _get_slot_node(slot_id: int) -> TextureRect:
	if slots.has(slot_id):
		return slots[slot_id] as TextureRect
	return extra_slots.get(slot_id, null) as TextureRect

func _get_slot_prev_node(slot_id: int) -> TextureRect:
	if slot_prevs.has(slot_id):
		return slot_prevs[slot_id] as TextureRect
	return extra_slot_prevs.get(slot_id, null) as TextureRect

func _all_slot_ids() -> Array:
	var ids: Array = extra_slots.keys()
	for k in slots.keys():
		if not (k in ids):
			ids.append(k)
	return ids

func _setup_character_slots() -> void:
	slots.clear()
	slot_prevs.clear()
	extra_slots.clear()
	extra_slot_prevs.clear()
	_slot_tweens.clear()
	_slot_move_tweens.clear()

	# 装配 Slot 0 (如 Shovel_03 全屏前景)，保持 slots.size() == 10 兼容性
	var slot0 = character_slots.get_node_or_null("Slot0") as TextureRect
	if slot0 == null:
		slot0 = TextureRect.new()
		slot0.name = "Slot0"
		slot0.mouse_filter = Control.MOUSE_FILTER_IGNORE
		slot0.layout_mode = 1
		slot0.anchors_preset = 7
		slot0.anchor_left = 0.5
		slot0.anchor_top = 1.0
		slot0.anchor_right = 0.5
		slot0.anchor_bottom = 1.0
		slot0.offset_left = -640.0
		slot0.offset_top = -720.0
		slot0.offset_right = 640.0
		slot0.offset_bottom = 0.0
		slot0.grow_horizontal = Control.GROW_DIRECTION_BOTH
		slot0.grow_vertical = Control.GROW_DIRECTION_BEGIN
		slot0.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
		slot0.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
		character_slots.add_child(slot0)
		character_slots.move_child(slot0, 0)
	slot0.visible = false
	slot0.texture = null
	slot0.modulate.a = 1.0
	extra_slots[0] = slot0

	var prev0 = character_slots.get_node_or_null("Slot0Prev") as TextureRect
	if prev0 == null:
		prev0 = TextureRect.new()
		prev0.name = "Slot0Prev"
		prev0.mouse_filter = Control.MOUSE_FILTER_IGNORE
		prev0.layout_mode = 1
		prev0.anchor_left = 0.5
		prev0.anchor_top = 1.0
		prev0.anchor_right = 0.5
		prev0.anchor_bottom = 1.0
		prev0.offset_left = slot0.offset_left
		prev0.offset_top = slot0.offset_top
		prev0.offset_right = slot0.offset_right
		prev0.offset_bottom = slot0.offset_bottom
		prev0.grow_horizontal = slot0.grow_horizontal
		prev0.grow_vertical = slot0.grow_vertical
		prev0.expand_mode = slot0.expand_mode
		prev0.stretch_mode = slot0.stretch_mode
		character_slots.add_child(prev0)
		character_slots.move_child(prev0, slot0.get_index())
	prev0.visible = false
	prev0.texture = null
	prev0.modulate.a = 0.0
	extra_slot_prevs[0] = prev0

	for i in range(1, 11):
		var slot_node = character_slots.get_node_or_null("Slot" + str(i))
		if slot_node is TextureRect:
			slots[i] = slot_node
			# 所有立绘槽位统一以屏幕水平中线 (0.5) 为原点，避免与 BGI x 偏移叠加导致重叠
			slot_node.anchor_left = 0.5
			slot_node.anchor_right = 0.5
			slot_node.visible = false
			slot_node.texture = null
			slot_node.modulate.a = 1.0
			# 清理旧版本误挂在 Slot 内部的 SlotPrev 子节点，避免父节点透明度归零时连带底层变透明
			var old_child_prev = slot_node.get_node_or_null("SlotPrev")
			if old_child_prev:
				old_child_prev.queue_free()

			# 在 character_slots 下装配同级双缓冲节点 Slot{i}Prev（严格位于 Slot{i} 之下）
			var prev_name = "Slot" + str(i) + "Prev"
			var prev = character_slots.get_node_or_null(prev_name) as TextureRect
			if prev == null:
				prev = TextureRect.new()
				prev.name = prev_name
				prev.mouse_filter = Control.MOUSE_FILTER_IGNORE
				prev.layout_mode = 1
				prev.anchor_left = slot_node.anchor_left
				prev.anchor_top = slot_node.anchor_top
				prev.anchor_right = slot_node.anchor_right
				prev.anchor_bottom = slot_node.anchor_bottom
				prev.offset_left = slot_node.offset_left
				prev.offset_top = slot_node.offset_top
				prev.offset_right = slot_node.offset_right
				prev.offset_bottom = slot_node.offset_bottom
				prev.grow_horizontal = slot_node.grow_horizontal
				prev.grow_vertical = slot_node.grow_vertical
				prev.expand_mode = slot_node.expand_mode
				prev.stretch_mode = slot_node.stretch_mode
				prev.visible = false
				prev.modulate.a = 0.0
				character_slots.add_child(prev)
				character_slots.move_child(prev, slot_node.get_index())
			slot_prevs[i] = prev

# 同级双层槽位立绘更新核心函数 (对齐 PC 录像实测 200ms 与师父四大优雅避坑铁律)
func _set_slot_sprite(slot_id: int, tex: Texture2D, instant: bool = false, fade_sec: float = SPRITE_FADE_SEC) -> void:
	var slot: TextureRect = _get_slot_node(slot_id)
	if slot == null:
		return
	var prev: TextureRect = _get_slot_prev_node(slot_id)

	if _slot_tweens.has(slot_id) and _slot_tweens[slot_id] is Tween and _slot_tweens[slot_id].is_valid():
		_slot_tweens[slot_id].kill()
	_slot_tweens.erase(slot_id)

	var eff_fade := maxf(0.0, fade_sec)

	# 1. 瞬切模式 (黑幕原子替换、fade_ms==0、SKIP 模式或无头自动化测试)
	if instant or eff_fade <= 0.005 or DisplayServer.get_name() == "headless":
		if prev:
			prev.visible = false
			prev.texture = null
			prev.modulate.a = 0.0
		if tex == null:
			slot.visible = false
			slot.texture = null
			slot.modulate.a = 0.0
		else:
			slot.texture = tex
			slot.modulate.a = 1.0
			slot.visible = true
		return

	# 2. 退场 (有图 -> 空槽，200ms 平滑淡出)
	if tex == null:
		if prev:
			prev.visible = false
			prev.texture = null
		if not slot.visible or slot.texture == null:
			slot.visible = false
			return
		var tw = create_tween()
		_slot_tweens[slot_id] = tw
		tw.tween_property(slot, "modulate:a", 0.0, eff_fade)
		tw.finished.connect(func():
			slot.visible = false
			slot.texture = null
			slot.modulate.a = 0.0
			_slot_tweens.erase(slot_id)
		)
		return

	# 3. 进场 (空槽 -> 有图，200ms 单层淡入)
	var old_tex = slot.texture
	if old_tex == null or not slot.visible:
		if prev:
			prev.visible = false
			prev.texture = null
		slot.texture = tex
		slot.modulate.a = 0.0
		slot.visible = true
		var tw = create_tween()
		_slot_tweens[slot_id] = tw
		tw.tween_property(slot, "modulate:a", 1.0, eff_fade)
		tw.finished.connect(func():
			_slot_tweens.erase(slot_id)
		)
		return

	# 4. 替换表情 / 切换角色
	if old_tex == tex:
		return

	if prev:
		prev.texture = old_tex
		prev.position = slot.position
		prev.size = slot.size
		prev.modulate.a = 1.0
		prev.visible = true

	slot.texture = tex
	slot.modulate.a = 0.0
	slot.visible = true

	var tw = create_tween()
	_slot_tweens[slot_id] = tw

	# 判定同角色换表情：尺寸一致时，底层保持 alpha=1.0 锁定，顶层 200ms 淡入覆盖，杜绝 25% 鬼影！
	var same_size = (old_tex.get_size() == tex.get_size())
	if same_size:
		tw.tween_property(slot, "modulate:a", 1.0, eff_fade)
		tw.finished.connect(func():
			if prev:
				prev.visible = false
				prev.texture = null
				prev.modulate.a = 0.0
			_slot_tweens.erase(slot_id)
		)
	else:
		# 换角色或大姿态切换：底层延迟后淡出，顶层淡入
		tw.set_parallel(true)
		tw.tween_property(slot, "modulate:a", 1.0, eff_fade)
		if prev:
			tw.tween_property(prev, "modulate:a", 0.0, eff_fade * 0.7).set_delay(eff_fade * 0.3)
		tw.chain().tween_callback(func():
			if prev:
				prev.visible = false
				prev.texture = null
				prev.modulate.a = 0.0
			_slot_tweens.erase(slot_id)
		)

func _reset_all_slots() -> void:
	for slot_id in _all_slot_ids():
		var slot: TextureRect = _get_slot_node(slot_id)
		if slot:
			slot.texture = null
			slot.visible = false
			slot.modulate.a = 0.0
		var prev: TextureRect = _get_slot_prev_node(slot_id)
		if prev:
			prev.texture = null
			prev.visible = false
			prev.modulate.a = 0.0
	_flush_all_slots()

func _flush_all_slots() -> void:
	for slot_id in _slot_tweens.keys():
		var tw = _slot_tweens[slot_id]
		if tw is Tween and tw.is_valid():
			tw.kill()
	_slot_tweens.clear()
	for slot_id in _slot_move_tweens.keys():
		var mtw = _slot_move_tweens[slot_id]
		if mtw is Tween and mtw.is_valid():
			mtw.kill()
	_slot_move_tweens.clear()
	for slot_id in _all_slot_ids():
		var slot: TextureRect = _get_slot_node(slot_id)
		var prev: TextureRect = _get_slot_prev_node(slot_id)
		if prev:
			prev.visible = false
			prev.texture = null
			prev.modulate.a = 0.0
		if slot and slot.texture != null:
			slot.modulate.a = 1.0
			slot.visible = true
		elif slot:
			slot.visible = false

func _setup_quick_menu() -> void:
	# 锁定 TopBar 最小高度 36px 与 QuickMenu 垂直居中，杜绝有无说话人切换时 7 按钮被撑高或跳动
	var top_bar := get_node_or_null("DialogueBox/Margin/VBox/TopBar") as Control
	if top_bar:
		top_bar.custom_minimum_size = Vector2(0, 36)
	var quick_menu := get_node_or_null("DialogueBox/Margin/VBox/TopBar/QuickMenu") as Control
	if quick_menu:
		quick_menu.size_flags_vertical = Control.SIZE_SHRINK_CENTER

	# 装配 7 功能按钮中文文字与悬停/按下微动效 (解决 iOS 端按钮不显文字与文字偏下缺陷，并对齐 4argon 师父裁决)
	var btn_specs = [
		[btn_qsave, "快存", _on_quick_save],
		[btn_qload, "快读", _on_quick_load],
		[btn_log, "履历", _open_backlog],
		[btn_auto, "自动", _toggle_auto_mode],
		[btn_skip, "快进", _toggle_skip_mode],
		[btn_config, "设置", _on_config],
		[btn_hide, "隐藏", _hide_ui],
	]
	for spec in btn_specs:
		var btn: TextureButton = spec[0]
		var btn_title: String = spec[1]
		var cb: Callable = spec[2]
		if btn == null:
			continue
		btn.custom_minimum_size = Vector2(58, 22)
		btn.size_flags_vertical = Control.SIZE_SHRINK_CENTER
		if not btn.pressed.is_connected(cb):
			btn.pressed.connect(cb)
		_attach_button_label(btn, btn_title)

	if close_backlog_btn and not close_backlog_btn.pressed.is_connected(_close_backlog):
		close_backlog_btn.pressed.connect(_close_backlog)

func _attach_button_label(btn: TextureButton, text: String) -> void:
	var lbl = btn.get_node_or_null("BtnLabel") as Label
	if lbl == null:
		lbl = Label.new()
		lbl.name = "BtnLabel"
		# 师父铁律：必须设置 MOUSE_FILTER_IGNORE，防止盖住父级 TextureButton 点击事件
		lbl.mouse_filter = Control.MOUSE_FILTER_IGNORE
		lbl.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
		lbl.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
		if _custom_font:
			lbl.add_theme_font_override("font", _custom_font)
		lbl.add_theme_font_size_override("font_size", 12)
		lbl.add_theme_color_override("font_color", Color(0.92, 0.96, 1.0, 0.95))
		lbl.add_theme_color_override("font_outline_color", Color(0.04, 0.08, 0.18, 0.9))
		lbl.add_theme_constant_override("outline_size", 2)
		btn.add_child(lbl)
		lbl.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	else:
		lbl.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	lbl.text = text

	# 悬停与按下微动效反馈
	if not btn.mouse_entered.is_connected(_on_btn_mouse_entered.bind(lbl)):
		btn.mouse_entered.connect(_on_btn_mouse_entered.bind(lbl))
	if not btn.mouse_exited.is_connected(_on_btn_mouse_exited.bind(lbl)):
		btn.mouse_exited.connect(_on_btn_mouse_exited.bind(lbl))
	if not btn.button_down.is_connected(_on_btn_down.bind(lbl)):
		btn.button_down.connect(_on_btn_down.bind(lbl))
	if not btn.button_up.is_connected(_on_btn_up.bind(lbl)):
		btn.button_up.connect(_on_btn_up.bind(lbl))

func _on_btn_mouse_entered(lbl: Label) -> void:
	if lbl:
		lbl.modulate = Color(1.2, 1.2, 1.2, 1.0)

func _on_btn_mouse_exited(lbl: Label) -> void:
	if lbl:
		lbl.modulate = Color(1.0, 1.0, 1.0, 1.0)
		lbl.offset_top = 0
		lbl.offset_bottom = 0

func _on_btn_down(lbl: Label) -> void:
	if lbl:
		lbl.offset_top = 1
		lbl.offset_bottom = 1
		lbl.modulate = Color(0.85, 0.85, 0.95, 1.0)

func _on_btn_up(lbl: Label) -> void:
	if lbl:
		lbl.offset_top = 0
		lbl.offset_bottom = 0
		lbl.modulate = Color(1.2, 1.2, 1.2, 1.0)

func _reset_quick_menu_label_offsets() -> void:
	for btn in [btn_qsave, btn_qload, btn_log, btn_auto, btn_skip, btn_config, btn_hide]:
		if btn == null:
			continue
		var lbl := btn.get_node_or_null("BtnLabel") as Label
		if lbl:
			lbl.offset_top = 0
			lbl.offset_bottom = 0
			lbl.modulate = Color(1.0, 1.0, 1.0, 1.0)

func _setup_click_icon_animation() -> void:
	if click_icon == null:
		return
	# 呼吸渐隐微动效
	var icon_tween = create_tween().set_loops()
	icon_tween.tween_property(click_icon, "modulate:a", 0.35, 0.6).set_trans(Tween.TRANS_SINE)
	icon_tween.tween_property(click_icon, "modulate:a", 1.0, 0.6).set_trans(Tween.TRANS_SINE)

func _parse_command_line_args() -> Dictionary:
	var archive_dir = DEFAULT_ARCHIVE_DIR
	var patch_dir = DEFAULT_PATCH_DIR
	var script_name = DEFAULT_SCRIPT
	var explicit_paths: bool = false
	# 无头模式 (自动化单测) 默认直跳剧情
	var is_auto_play: bool = (DisplayServer.get_name() == "headless")

	var args = OS.get_cmdline_args()
	var user_args = OS.get_cmdline_user_args()

	# 先解析引擎侧参数（仅识别 --scenario / --play；避免与 Godot 自身 --script res://*.gd 冲突）
	var i = 0
	while i < args.size():
		var arg = args[i]
		if (arg == "--scenario" or arg == "--play") and i + 1 < args.size():
			script_name = args[i + 1]
			is_auto_play = true
			i += 1
		elif arg == "--script" and i + 1 < args.size():
			var next_val = args[i + 1]
			if not next_val.ends_with(".gd") and not next_val.ends_with(".tscn") and not next_val.begins_with("res://") and not next_val.begins_with("user://") and not ("/" in next_val) and not ("\\" in next_val):
				script_name = next_val
				is_auto_play = true
			i += 1
		elif arg == "--archive-dir" and i + 1 < args.size():
			archive_dir = args[i + 1]
			explicit_paths = true
			i += 1
		elif arg == "--patch-dir" and i + 1 < args.size():
			patch_dir = args[i + 1]
			explicit_paths = true
			i += 1
		i += 1

	# 再解析 -- 之后的用户参数（支持 --script / --scenario / --play）
	i = 0
	while i < user_args.size():
		var arg = user_args[i]
		if (arg == "--script" or arg == "--scenario" or arg == "--play") and i + 1 < user_args.size():
			script_name = user_args[i + 1]
			is_auto_play = true
			i += 1
		elif arg == "--archive-dir" and i + 1 < user_args.size():
			archive_dir = user_args[i + 1]
			explicit_paths = true
			i += 1
		elif arg == "--patch-dir" and i + 1 < user_args.size():
			patch_dir = user_args[i + 1]
			explicit_paths = true
			i += 1
		i += 1

	return {
		"archive_dir": archive_dir,
		"patch_dir": patch_dir,
		"script_name": script_name,
		"is_auto_play": is_auto_play,
		"explicit_paths": explicit_paths,
	}

# 推进流式对白核心逻辑
func advance_dialogue() -> void:
	if engine == null or _is_terminated or _is_waiting_for_choice:
		return

	# 若正处于新角色登场介绍特写中，点击即熔断跳过介绍，显示当前首句台词，绝不跳过该角色台词！
	if _is_in_character_intro:
		_skip_character_intro()
		return

	# 若黑幕转场正在进行，点击即熔断加速转场（直跳目标场景），绝不让玩家干等卡手感
	if _is_transitioning:
		_skip_active_transition()
		return

	# 若履历面板或系统设置面板已打开，禁止推进对白
	if (backlog_panel and backlog_panel.visible) or (settings_panel and settings_panel.visible):
		return

	# 若正处于隐藏 UI 状态，仅唤醒 UI，不跳剧情
	if _is_ui_hidden:
		_show_ui()
		return

	# 当设定“单击鼠标停止语音”时，即使在打字机显现提前 return 前也必须柔和停音
	if _setting_voice_stop_on_next and _is_any_voice_playing():
		_stop_all_voices(false)

	# 若当前正处于打字机显现中，四态锁：初次点击即时全显，不跳下句
	if _is_typing:
		_instant_reveal_text()
		return

	# 从 Rust VM 获取下一句增量快照 (Delta Snapshot)
	var step: Dictionary = engine.next_dialogue()
	_flush_system_flags_if_dirty()
	_apply_snapshot(step)

# 解析并应用增量快照
func _apply_snapshot(step: Dictionary) -> void:
	var state = step.get("state", "")
	_hide_window_when_selecting = bool(step.get("hide_window_when_selecting", false))
	var step_mail_mode: bool = bool(step.get("mail_mode", false))
	if step.has("mail_btn_state"):
		_update_mail_btn_state(int(step.get("mail_btn_state", 0)))

	match state:
		"WaitingForInput":
			_is_waiting_for_wait_click = false
			_wait_click_seq += 1
			_restore_choice_box_layout()

			# 1. 检测新角色登场转场介绍特写 (0x00F0 Subscript 演出卡片与专属 Jingle)
			var intro = step.get("character_intro")
			if intro != null and intro is Dictionary and not intro.is_empty():
				var char_id = str(intro.get("character", "")).to_lower()
				if char_id != "" and not _seen_character_intros.has(char_id):
					_seen_character_intros[char_id] = true
					# 先提交当前帧舞台变化（如 _FadeBS 清退旧立绘），避免 90% 白雾下残留上一幕立绘
					_apply_stage_visuals_normal(step)
					_run_character_intro(intro, step)
					return

			# 2. 检测是否构成场景切换 (提供新背景纹理，且不同于当前已有背景)
			var new_bg = step.get("bg_texture")
			var is_scene_change: bool = (new_bg != null and background.texture != null and background.texture != new_bg)

			if is_scene_change and not _is_skip_active and DisplayServer.get_name() != "headless":
				_run_scene_transition(step)
			else:
				_apply_stage_visuals_normal(step)
				_apply_dialogue_and_audio(step)

		"WaitingForWaitClick":
			# _Wait(-1, 1) 独立手机翻页挂起态：隐藏对话框与半身像，露出完整居中手机，零污染对白与 Backlog
			_is_waiting_for_wait_click = true
			_wait_click_seq += 1
			_is_typing = false
			_is_waiting_for_choice = false
			if step_mail_mode:
				_is_mail_mode = true
			if choice_box:
				choice_box.visible = false
			_restore_choice_box_layout()
			if dialogue_box:
				dialogue_box.visible = false
			if face_avatar_layer:
				face_avatar_layer.visible = false
			_sync_mail_hud_visibility()
			_update_spr_overlay(step, false)
			_play_step_se_streams(step)
			var seq_cap := _wait_click_seq
			if not _is_mail_mode:
				if _is_skip_active:
					_schedule_wait_click_skip(seq_cap)
				elif _is_auto_active:
					_schedule_wait_click_auto(seq_cap)

		"WaitingForChoice":
			_is_waiting_for_wait_click = false
			_wait_click_seq += 1
			if step_mail_mode:
				_is_mail_mode = true
			_update_spr_overlay(step, false)
			_play_step_se_streams(step)
			var prompt = step.get("choice_prompt", "")
			var options = step.get("choice_options", [])
			var disabled_opts = step.get("disabled_options", [])
			var cseq = int(step.get("choice_seq", 0))
			var hide_win = bool(step.get("hide_window_when_selecting", false))
			_show_choices(prompt, options, cseq, hide_win, disabled_opts)
			_sync_mail_hud_visibility()

		"MailClosed":
			_on_mail_window_closed(step)

		"Terminated":
			_is_waiting_for_wait_click = false
			_wait_click_seq += 1
			_is_mail_mode = false
			_is_terminated = true
			_is_typing = false
			_is_auto_active = false
			_is_skip_active = false
			_clear_spr_overlay_instant()
			_stop_mail_on_bubble()
			_sync_mail_hud_visibility()
			text_label.text = "[color=#77ee77]【剧本演播完毕】[/color]"
			speaker_label.visible = false
			if click_icon:
				click_icon.visible = false
			_stop_all_voices(true)

		"Error":
			_is_waiting_for_wait_click = false
			_wait_click_seq += 1
			_is_mail_mode = false
			_is_terminated = true
			_is_typing = false
			_is_auto_active = false
			_is_skip_active = false
			_clear_spr_overlay_instant()
			_stop_mail_on_bubble()
			_sync_mail_hud_visibility()
			text_label.text = "[color=#ff5555]【剧本执行异常】[/color]"
			speaker_label.visible = false
			if click_icon:
				click_icon.visible = false
			_stop_all_voices(true)

		_:
			print("[PureConnect] 收到未知状态: ", state)

# 黑场转场控制器 (对齐 PC 录像实测 1000ms+500ms+500ms 节奏)
func _run_scene_transition(snapshot: Dictionary) -> void:
	var skip_effects: bool = (_setting_effect_mode == 2)
	if _is_skip_active or skip_effects or DisplayServer.get_name() == "headless":
		_finish_transition_immediately(snapshot)
		return

	if _scene_tween and _scene_tween.is_valid():
		_scene_tween.kill()

	_is_transitioning = true
	_pending_snapshot = snapshot

	# 转场开始先隐藏对话框，防止黑屏打字穿帮
	dialogue_box.visible = false
	_sync_mail_hud_visibility()
	if transition_overlay:
		transition_overlay.visible = true
		transition_overlay.modulate.a = 0.0

	var t_mul := 0.45 if _setting_effect_mode == 1 else 1.0
	_scene_tween = create_tween()
	if transition_overlay:
		_scene_tween.tween_property(transition_overlay, "modulate:a", 1.0, SCENE_FADE_OUT_SEC * t_mul)
	_scene_tween.tween_callback(func():
		_apply_stage_visuals_instant(_pending_snapshot)
	)
	_scene_tween.tween_interval(SCENE_HOLD_BLACK_SEC * t_mul)
	if transition_overlay:
		_scene_tween.tween_property(transition_overlay, "modulate:a", 0.0, SCENE_FADE_IN_SEC * t_mul)
	_scene_tween.tween_callback(func():
		_on_scene_transition_complete()
	)

func _skip_active_transition() -> void:
	if not _is_transitioning:
		return
	if _scene_tween and _scene_tween.is_valid():
		_scene_tween.kill()
	_finish_transition_immediately(_pending_snapshot)

func _finish_transition_immediately(snapshot: Dictionary) -> void:
	_is_transitioning = false
	if transition_overlay:
		transition_overlay.modulate.a = 0.0
		transition_overlay.visible = false
	_apply_stage_visuals_instant(snapshot)
	dialogue_box.visible = true
	_apply_dialogue_and_audio(snapshot)
	_pending_snapshot = {}

func _on_scene_transition_complete() -> void:
	_is_transitioning = false
	if transition_overlay:
		transition_overlay.visible = false
		transition_overlay.modulate.a = 0.0
	dialogue_box.visible = true
	_apply_dialogue_and_audio(_pending_snapshot)
	_pending_snapshot = {}

# 黑幕峰值瞬切应用（原子提交）
func _apply_stage_visuals_instant(step: Dictionary) -> void:
	_flush_all_slots()
	_apply_bg_pose_and_motions(step.get("bg_texture"), step.get("bg_pose"), step.get("bg_motions"), true)

	var changed_slots = step.get("changed_slots", {})
	for slot_key in changed_slots.keys():
		var slot_id = int(slot_key)
		_set_slot_sprite(slot_id, changed_slots[slot_key], true)

	var cleared_slots = step.get("cleared_slots", [])
	for slot_key in cleared_slots:
		var slot_id = int(slot_key)
		_set_slot_sprite(slot_id, null, true)

	_apply_slot_transforms_and_motions(step, true)

	var face_changed = step.get("face_changed", false)
	var face_tex = step.get("face_texture")
	var speaker = step.get("speaker")
	if face_changed:
		_update_face_avatar(face_tex, speaker, true, step.get("face_offset"))

	_update_spr_overlay(step, true)

# 正常同场景立绘与舞台应用 (对齐 BGI fade_ms，默认 200ms 柔和过渡)
func _apply_stage_visuals_normal(step: Dictionary) -> void:
	_apply_bg_pose_and_motions(step.get("bg_texture"), step.get("bg_pose"), step.get("bg_motions"), false)

	var transforms: Variant = step.get("slot_transforms")
	var changed_slots = step.get("changed_slots", {})
	for slot_key in changed_slots.keys():
		var slot_id = int(slot_key)
		var fsec := SPRITE_FADE_SEC
		if transforms is Dictionary:
			var tr_var: Variant = transforms.get(slot_id, transforms.get(str(slot_id), null))
			if tr_var is Dictionary and tr_var.has("fade_ms"):
				fsec = float(tr_var["fade_ms"]) / 1000.0
		_set_slot_sprite(slot_id, changed_slots[slot_key], false, fsec)

	var cleared_slots = step.get("cleared_slots", [])
	for slot_key in cleared_slots:
		var slot_id = int(slot_key)
		_set_slot_sprite(slot_id, null, false)

	_apply_slot_transforms_and_motions(step, false)

	var face_changed = step.get("face_changed", false)
	var face_tex = step.get("face_texture")
	var speaker = step.get("speaker")
	if face_changed:
		_update_face_avatar(face_tex, speaker, false, step.get("face_offset"))

	_update_spr_overlay(step, false)

# 独立 SPR 覆盖层更新（主线睡前短信等 slot 0，支持 300ms 淡入 / 200ms 淡出）
func _update_spr_overlay(step: Dictionary, instant: bool = false) -> void:
	if spr_overlay == null:
		return
	if not bool(step.get("spr_changed", false)):
		return

	_spr_tween_seq += 1
	var my_seq := _spr_tween_seq
	if _spr_tween and _spr_tween.is_valid():
		_spr_tween.kill()
	_spr_tween = null

	var spr_tex: Variant = step.get("spr_texture")
	var spr_asset: String = str(step.get("spr_asset", ""))
	var fade_ms: int = int(step.get("spr_fade_ms", 0))
	var fade_sec: float = maxf(0.0, float(fade_ms) / 1000.0)
	var skip_anim: bool = instant or DisplayServer.get_name() == "headless" or _is_skip_active or _setting_effect_mode == 2 or fade_sec <= 0.001

	if spr_tex != null and spr_tex is Texture2D:
		_active_spr_asset = spr_asset
		var old_tex: Texture2D = spr_overlay.texture if spr_overlay.visible else null
		if skip_anim:
			if spr_overlay_prev:
				spr_overlay_prev.visible = false
				spr_overlay_prev.texture = null
				spr_overlay_prev.modulate.a = 1.0
			spr_overlay.texture = spr_tex
			spr_overlay.modulate.a = 1.0
			spr_overlay.visible = true
			return

		if old_tex != null and spr_overlay_prev != null:
			spr_overlay_prev.texture = old_tex
			spr_overlay_prev.modulate.a = 1.0
			spr_overlay_prev.visible = true
		elif spr_overlay_prev != null:
			spr_overlay_prev.visible = false
			spr_overlay_prev.texture = null

		spr_overlay.texture = spr_tex
		spr_overlay.modulate.a = 0.0
		spr_overlay.visible = true
		_spr_tween = create_tween()
		_spr_tween.tween_property(spr_overlay, "modulate:a", 1.0, fade_sec)
		_spr_tween.finished.connect(func():
			if my_seq != _spr_tween_seq:
				return
			if spr_overlay_prev:
				spr_overlay_prev.visible = false
				spr_overlay_prev.texture = null
		)
	else:
		_active_spr_asset = ""
		_restore_choice_box_layout()
		if spr_overlay_prev:
			spr_overlay_prev.visible = false
			spr_overlay_prev.texture = null
		if skip_anim or not spr_overlay.visible:
			spr_overlay.visible = false
			spr_overlay.texture = null
			spr_overlay.modulate.a = 1.0
			return

		_spr_tween = create_tween()
		_spr_tween.tween_property(spr_overlay, "modulate:a", 0.0, fade_sec)
		_spr_tween.finished.connect(func():
			if my_seq != _spr_tween_seq:
				return
			spr_overlay.visible = false
			spr_overlay.texture = null
			spr_overlay.modulate.a = 1.0
		)

# 强制即时清空独立 SPR 覆盖层（切换大场景 / 读档 / 返回标题 / 新游戏）
func _clear_spr_overlay_instant() -> void:
	_spr_tween_seq += 1
	if _spr_tween and _spr_tween.is_valid():
		_spr_tween.kill()
	_spr_tween = null
	_active_spr_asset = ""
	_is_waiting_for_wait_click = false
	_wait_click_seq += 1
	_hide_window_when_selecting = false
	_restore_choice_box_layout()
	if spr_overlay_prev:
		spr_overlay_prev.visible = false
		spr_overlay_prev.texture = null
		spr_overlay_prev.modulate.a = 1.0
	if spr_overlay:
		spr_overlay.visible = false
		spr_overlay.texture = null
		spr_overlay.modulate.a = 1.0

# ==============================================================================
# 共同线彩蛋信封 HUD (48x48 四态图标 + mail_on_A 5帧气泡) 与方案 4A 独立子引擎调度
# ==============================================================================
func _setup_mail_hud() -> void:
	if _mail_hud_layer == null or not is_instance_valid(_mail_hud_layer):
		_mail_hud_layer = Control.new()
		_mail_hud_layer.name = "MailHudLayer"
		_mail_hud_layer.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		_mail_hud_layer.mouse_filter = Control.MOUSE_FILTER_IGNORE
		add_child(_mail_hud_layer)
		if dialogue_box and dialogue_box.get_parent() == self:
			move_child(_mail_hud_layer, dialogue_box.get_index() + 1)

		_mail_btn = TextureButton.new()
		_mail_btn.name = "MailButton"
		_mail_btn.size = Vector2(48.0, 48.0)
		_mail_btn.custom_minimum_size = Vector2(48.0, 48.0)
		_mail_btn.ignore_texture_size = true
		_mail_btn.stretch_mode = TextureButton.STRETCH_KEEP_ASPECT_CENTERED
		_mail_btn.focus_mode = Control.FOCUS_NONE
		_mail_btn.visible = false
		_mail_btn.pressed.connect(_on_mail_btn_pressed)
		_mail_btn.mouse_entered.connect(_on_mail_btn_mouse_entered)
		_mail_btn.mouse_exited.connect(_on_mail_btn_mouse_exited)
		_mail_hud_layer.add_child(_mail_btn)

		_mail_on_bubble = TextureRect.new()
		_mail_on_bubble.name = "MailOnBubble"
		_mail_on_bubble.size = Vector2(195.0, 48.0)
		_mail_on_bubble.custom_minimum_size = Vector2(195.0, 48.0)
		_mail_on_bubble.mouse_filter = Control.MOUSE_FILTER_IGNORE
		_mail_on_bubble.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
		_mail_on_bubble.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
		_mail_on_bubble.visible = false
		_mail_hud_layer.add_child(_mail_on_bubble)

		# 使用节点级可暂停 Timer 驱动 5 帧气泡状态机，杜绝悬空 SceneTreeTimer 协程导致退出 RID 泄漏
		_mail_bubble_timer = Timer.new()
		_mail_bubble_timer.name = "MailBubbleTimer"
		_mail_bubble_timer.one_shot = true
		_mail_bubble_timer.process_mode = Node.PROCESS_MODE_PAUSABLE
		_mail_bubble_timer.timeout.connect(_on_mail_bubble_timer_timeout)
		_mail_hud_layer.add_child(_mail_bubble_timer)

	# 通过已挂载的 VFS 原生解码 sysgrp.arc 信封三态图标与 mail_on_A 5 帧竖条气泡
	if engine != null:
		if not _mail_btn_textures.has(0):
			var t0 = engine.load_ui_texture("SGMsgWnd000200")
			if t0 is Texture2D:
				_mail_btn_textures[0] = t0
		if not _mail_btn_textures.has(1):
			var t1 = engine.load_ui_texture("SGMsgWnd000201")
			if t1 is Texture2D:
				_mail_btn_textures[1] = t1
		if not _mail_btn_textures.has(2):
			var t2 = engine.load_ui_texture("SGMsgWnd000202")
			if t2 is Texture2D:
				_mail_btn_textures[2] = t2
		if _mail_on_frames.is_empty():
			var strip = engine.load_ui_texture("mail_on_a")
			if strip is Texture2D:
				var strip_tex: Texture2D = strip as Texture2D
				var sz: Vector2 = strip_tex.get_size()
				var fw: float = sz.x
				var fh: float = floorf(sz.y / 5.0)
				if fw > 0.0 and fh > 0.0:
					for fi in range(5):
						var atlas := AtlasTexture.new()
						atlas.atlas = strip_tex
						atlas.region = Rect2(0.0, fi * fh, fw, fh)
						_mail_on_frames.append(atlas)

	_layout_mail_hud()
	if engine != null:
		_update_mail_btn_state(int(engine.get_mail_btn_state()))
	_sync_mail_hud_visibility()

func _layout_mail_hud() -> void:
	var btn_pos := MAIL_BTN_POS_4_3 if _setting_screen_mode == 1 else MAIL_BTN_POS_16_9
	if _mail_btn != null and is_instance_valid(_mail_btn):
		_mail_btn.position = btn_pos
		_mail_btn.size = Vector2(48.0, 48.0)
	if _mail_on_bubble != null and is_instance_valid(_mail_on_bubble):
		_mail_on_bubble.position = btn_pos + MAIL_BUBBLE_OFFSET
		_mail_on_bubble.size = Vector2(195.0, 48.0)

func _kill_mail_pulse_tween() -> void:
	if _mail_pulse_tween != null and _mail_pulse_tween.is_valid():
		_mail_pulse_tween.kill()
	_mail_pulse_tween = null
	if _mail_btn != null and is_instance_valid(_mail_btn):
		_mail_btn.modulate = Color(1.0, 1.0, 1.0, 1.0)

func _start_mail_pulse_tween() -> void:
	_kill_mail_pulse_tween()
	if _mail_btn == null or not is_instance_valid(_mail_btn) or not _mail_btn.visible:
		return
	if _mail_btn_state != 2 or _setting_effect_mode == 2:
		return
	_mail_btn.modulate = Color(1.0, 1.0, 1.0, 1.0)
	_mail_pulse_tween = create_tween().set_loops()
	_mail_pulse_tween.tween_property(_mail_btn, "modulate:a", 0.72, 0.75).set_trans(Tween.TRANS_SINE).set_ease(Tween.EASE_IN_OUT)
	_mail_pulse_tween.tween_property(_mail_btn, "modulate:a", 1.0, 0.75).set_trans(Tween.TRANS_SINE).set_ease(Tween.EASE_IN_OUT)

func _update_mail_btn_state(state: int) -> void:
	var clamped := clampi(state, 0, 3)
	var prev_state := _mail_btn_state
	_mail_btn_state = clamped
	if _mail_btn == null or not is_instance_valid(_mail_btn):
		return
	if clamped != 2 or prev_state != 2:
		_kill_mail_pulse_tween()

	match clamped:
		0:
			var t0: Texture2D = _mail_btn_textures.get(0, null)
			_mail_btn.texture_normal = t0
			_mail_btn.texture_hover = t0
			_mail_btn.texture_pressed = t0
			_mail_btn.texture_disabled = t0
			_mail_btn.disabled = true
			_mail_btn.focus_mode = Control.FOCUS_NONE
			_mail_btn.mouse_default_cursor_shape = Control.CURSOR_ARROW
			_mail_btn.modulate = Color(1.0, 1.0, 1.0, 1.0)
		1:
			var t1: Texture2D = _mail_btn_textures.get(1, null)
			_mail_btn.texture_normal = t1
			_mail_btn.texture_hover = t1
			_mail_btn.texture_pressed = t1
			_mail_btn.texture_disabled = t1
			_mail_btn.disabled = false
			_mail_btn.focus_mode = Control.FOCUS_NONE
			_mail_btn.mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND
			_mail_btn.modulate = Color(1.18, 1.18, 1.18, 1.0) if _mail_btn_hovered else Color(1.0, 1.0, 1.0, 1.0)
		2:
			var t2: Texture2D = _mail_btn_textures.get(2, null)
			_mail_btn.texture_normal = t2
			_mail_btn.texture_hover = t2
			_mail_btn.texture_pressed = t2
			_mail_btn.texture_disabled = t2
			_mail_btn.disabled = false
			_mail_btn.focus_mode = Control.FOCUS_NONE
			_mail_btn.mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND
			_mail_btn.modulate = Color(1.0, 1.0, 1.0, 1.0)
		3:
			_mail_btn.disabled = true
			_mail_btn.visible = false
			_mail_btn.mouse_default_cursor_shape = Control.CURSOR_ARROW

	_sync_mail_hud_visibility()

func _reset_mail_btn_hover_state() -> void:
	_mail_btn_hovered = false
	if _mail_btn != null and is_instance_valid(_mail_btn) and _mail_btn_state != 2:
		_mail_btn.modulate = Color(1.0, 1.0, 1.0, 1.0)

func _sync_mail_hud_visibility() -> void:
	if _mail_btn == null or not is_instance_valid(_mail_btn):
		return
	var can_show_btn: bool = (
		engine != null
		and _mail_btn_state != 3
		and not _is_in_title_screen
		and not _is_ui_hidden
		and not _is_in_character_intro
		and not _is_transitioning
		and not _is_waiting_for_wait_click
		and not _is_mail_mode
		and not _is_terminated
		and not _is_waiting_for_choice
		and not (settings_panel != null and settings_panel.visible)
		and not (backlog_panel != null and backlog_panel.visible)
		and dialogue_box != null and dialogue_box.visible
	)
	_mail_btn.visible = can_show_btn
	if not can_show_btn:
		_reset_mail_btn_hover_state()
		_kill_mail_pulse_tween()
		if _mail_on_bubble != null and is_instance_valid(_mail_on_bubble) and _mail_on_bubble.visible:
			_stop_mail_on_bubble()
	elif _mail_btn_state == 2:
		if _mail_pulse_tween == null or not _mail_pulse_tween.is_valid():
			_start_mail_pulse_tween()
	else:
		_kill_mail_pulse_tween()
		if _mail_btn_state == 1 and _mail_btn_hovered:
			_mail_btn.modulate = Color(1.18, 1.18, 1.18, 1.0)

func _on_mail_btn_mouse_entered() -> void:
	_mail_btn_hovered = true
	if _mail_btn != null and is_instance_valid(_mail_btn) and _mail_btn_state == 1 and not _mail_btn.disabled:
		_mail_btn.modulate = Color(1.18, 1.18, 1.18, 1.0)

func _on_mail_btn_mouse_exited() -> void:
	_mail_btn_hovered = false
	if _mail_btn != null and is_instance_valid(_mail_btn) and _mail_btn_state == 1:
		_mail_btn.modulate = Color(1.0, 1.0, 1.0, 1.0)

func _stop_mail_on_bubble() -> void:
	_mail_bubble_seq += 1
	_mail_bubble_frame_idx = -1
	if _mail_bubble_timer != null and is_instance_valid(_mail_bubble_timer):
		_mail_bubble_timer.stop()
	if _mail_on_bubble != null and is_instance_valid(_mail_on_bubble):
		_mail_on_bubble.visible = false

func _play_mail_on_bubble() -> void:
	if _mail_on_bubble == null or not is_instance_valid(_mail_on_bubble) or _mail_on_frames.size() < 5:
		return
	if _mail_bubble_timer == null or not is_instance_valid(_mail_bubble_timer):
		return
	if _is_skip_active or _is_mail_mode or _is_in_title_screen or _is_ui_hidden or not is_inside_tree():
		return
	_mail_bubble_seq += 1
	_mail_bubble_frame_idx = 0
	_layout_mail_hud()
	# 严格对齐原版 SetIntervalCountOfAnimation: 帧 0 = 5000ms, 帧 1..4 各 100ms
	# Timer 采用 PROCESS_MODE_PAUSABLE，确保 bg_run=false 失焦暂停时同步精确冻结
	_mail_on_bubble.texture = _mail_on_frames[0]
	_mail_on_bubble.visible = true
	_mail_bubble_timer.stop()
	_mail_bubble_timer.start(MAIL_BUBBLE_DURATIONS[0])

func _on_mail_bubble_timer_timeout() -> void:
	if _mail_bubble_frame_idx < 0 or _mail_on_bubble == null or not is_instance_valid(_mail_on_bubble):
		return
	var next_idx := _mail_bubble_frame_idx + 1
	if next_idx >= MAIL_BUBBLE_DURATIONS.size() or next_idx >= _mail_on_frames.size():
		_mail_bubble_frame_idx = -1
		_mail_on_bubble.visible = false
		return
	_mail_bubble_frame_idx = next_idx
	_mail_on_bubble.texture = _mail_on_frames[next_idx]
	_mail_on_bubble.visible = true
	if _mail_bubble_timer != null and is_instance_valid(_mail_bubble_timer):
		_mail_bubble_timer.start(MAIL_BUBBLE_DURATIONS[next_idx])

func _on_mail_btn_pressed() -> void:
	open_mail_window()

func open_mail_window() -> bool:
	if engine == null or _is_mail_mode or _is_in_title_screen or _is_terminated:
		return false
	if _is_waiting_for_choice or _is_waiting_for_wait_click or _is_in_character_intro or _is_transitioning or _is_ui_hidden:
		return false
	if (settings_panel != null and settings_panel.visible) or (backlog_panel != null and backlog_panel.visible):
		return false
	if _mail_btn_state != 1 and _mail_btn_state != 2:
		return false

	# 打开瞬间强制停掉主线 Auto / Skip，复位信封悬停态，清掉可能未结束的 SPR 淡出 Tween，中止气泡、柔和停掉主线语音、补齐当前打字机正文（绝不推进主线）
	_reset_mail_btn_hover_state()
	_clear_spr_overlay_instant()
	_is_auto_active = false
	_is_skip_active = false
	_stop_mail_on_bubble()
	_stop_all_voices(false)
	if _is_typing:
		_instant_reveal_text()

	var step: Dictionary = engine.open_mail_window()
	var st := str(step.get("state", ""))
	if st == "Error" or st == "":
		_show_notify("【当前无法打开邮件窗口】")
		return false
	if st == "MailClosed":
		_on_mail_window_closed(step)
		return true

	_is_mail_mode = true
	if dialogue_box:
		dialogue_box.visible = false
	if face_avatar_layer:
		face_avatar_layer.visible = false
	_sync_mail_hud_visibility()
	_apply_snapshot(step)
	return true

func _on_mail_window_closed(step: Dictionary) -> void:
	_is_mail_mode = false
	_is_waiting_for_wait_click = false
	_is_waiting_for_choice = false
	_wait_click_seq += 1
	var fade_ms: int = int(step.get("spr_fade_ms", 0))
	if fade_ms > 0 and spr_overlay != null and spr_overlay.visible:
		_update_spr_overlay({
			"spr_changed": true,
			"spr_texture": null,
			"spr_asset": "",
			"spr_fade_ms": fade_ms,
		}, false)
	else:
		_clear_spr_overlay_instant()
	if choice_box:
		choice_box.visible = false
	_restore_choice_box_layout()
	if dialogue_box:
		dialogue_box.visible = not _is_ui_hidden
	if face_avatar_layer and face_avatar and face_avatar.texture != null:
		face_avatar_layer.visible = not _is_ui_hidden
	_play_step_se_streams(step)
	var next_mail_state: int = int(step.get("mail_btn_state", engine.get_mail_btn_state() if engine else 0))
	_update_mail_btn_state(next_mail_state)
	_sync_mail_hud_visibility()

# 播放本步触发的 _PlaySE 音效流（接入 se_volume 与 bg_run=false 失焦暂停，连点/快进支持重触发）
func _play_step_se_streams(step: Dictionary) -> void:
	if se_player == null or not se_player.is_inside_tree():
		return
	var streams = step.get("se_streams", [])
	if not (streams is Array) or streams.is_empty():
		return
	var target_stream: AudioStream = null
	for item in streams:
		if item != null and item is AudioStream:
			target_stream = item
	if target_stream == null:
		return
	se_player.volume_db = -80.0 if _setting_se_vol <= 0.001 else linear_to_db(_setting_se_vol)
	if se_player.is_playing():
		se_player.stop()
	se_player.stream = target_stream
	se_player.stream_paused = false
	se_player.play()
	if _is_focus_paused and not _setting_bg_run:
		se_player.stream_paused = true
		if not _focus_paused_players.has(se_player):
			_focus_paused_players.append(se_player)

# 手机翻页期 Skip 快进节拍（约 120ms 翻一屏，严格受 tree.paused 失焦暂停控制）
func _schedule_wait_click_skip(seq: int) -> void:
	if not is_inside_tree():
		return
	var tree = get_tree()
	if tree:
		tree.create_timer(0.12, false).timeout.connect(func():
			if _is_skip_active and _is_waiting_for_wait_click and _wait_click_seq == seq and not _is_terminated:
				advance_dialogue()
		)

# 手机翻页期 Auto 自动翻页（停留 2.5s / auto_speed，严格受 tree.paused 失焦暂停控制，手动点击提前翻页自动失效旧计时器）
func _schedule_wait_click_auto(seq: int) -> void:
	if not is_inside_tree():
		return
	var tree = get_tree()
	if tree == null:
		return
	var auto_mul = maxf(0.4, _setting_auto_speed)
	var wait_time = 2.5 / auto_mul
	tree.create_timer(wait_time, false).timeout.connect(func():
		if _is_auto_active and _is_waiting_for_wait_click and _wait_click_seq == seq and not _is_terminated:
			advance_dialogue()
	)

# 背景几何位姿重置与缓慢平滑运镜驱动 (_DrawScene / _MoveSight / _ScrollBG)
func _apply_bg_pose_and_motions(bg_tex: Variant, pose_var: Variant, motions_var: Variant, instant: bool) -> void:
	if bg_tex != null:
		if _bg_pan_tween and _bg_pan_tween.is_valid():
			_bg_pan_tween.kill()
		_bg_pan_tween = null
		background.texture = bg_tex

		if bg_tex is Texture2D:
			var tex_sz: Vector2 = bg_tex.get_size()
			if tex_sz.x > 0.0 and tex_sz.y > 0.0:
				var scale_pct: float = float(pose_var.get("scale", 100)) if (pose_var is Dictionary) else 100.0
				var sc: float = maxf(0.1, scale_pct / 100.0)
				var target_sz: Vector2 = tex_sz * sc
				if target_sz.x < 1280.0 or target_sz.y < 720.0:
					var cover_k := maxf(1280.0 / target_sz.x, 720.0 / target_sz.y)
					target_sz *= cover_k
				var min_x := minf(0.0, 1280.0 - target_sz.x)
				var min_y := minf(0.0, 720.0 - target_sz.y)
				var px: float = (1280.0 - target_sz.x) * 0.5
				var py: float = (720.0 - target_sz.y) * 0.5
				if pose_var is Dictionary:
					if pose_var.get("x") != null:
						px = clampf(float(pose_var["x"]), min_x, 0.0)
					if pose_var.get("y") != null:
						py = clampf(float(pose_var["y"]), min_y, 0.0)
				background.size = target_sz
				background.position = Vector2(px, py)

	if motions_var is Array and not motions_var.is_empty() and background.texture != null:
		if _bg_pan_tween and _bg_pan_tween.is_valid():
			_bg_pan_tween.kill()
		var tex_sz := background.texture.get_size()
		if tex_sz.x <= 0.0 or tex_sz.y <= 0.0:
			return

		var cur_pos := background.position
		var cur_sz := background.size
		if cur_sz.x <= 1.0:
			cur_sz = Vector2(1280.0, 720.0)

		var no_anim: bool = _is_skip_active or (_setting_effect_mode == 2) or DisplayServer.get_name() == "headless"
		if no_anim:
			for m in motions_var:
				if not (m is Dictionary):
					continue
				var t_sc := float(m["target_scale"]) / 100.0 if m.get("target_scale") != null else (cur_sz.x / tex_sz.x)
				var t_sz := tex_sz * maxf(0.1, t_sc)
				if t_sz.x < 1280.0 or t_sz.y < 720.0:
					t_sz *= maxf(1280.0 / t_sz.x, 720.0 / t_sz.y)
				var min_x := minf(0.0, 1280.0 - t_sz.x)
				var min_y := minf(0.0, 720.0 - t_sz.y)
				var tx := cur_pos.x if (bool(m.get("keep_x", false)) or m.get("target_x") == null) else clampf(float(m["target_x"]), min_x, 0.0)
				var ty := cur_pos.y if (bool(m.get("keep_y", false)) or m.get("target_y") == null) else clampf(float(m["target_y"]), min_y, 0.0)
				cur_pos = Vector2(tx, ty)
				cur_sz = t_sz
			background.size = cur_sz
			background.position = cur_pos
			return

		_bg_pan_tween = create_tween()
		for m in motions_var:
			if not (m is Dictionary):
				continue
			var t_sc := float(m["target_scale"]) / 100.0 if m.get("target_scale") != null else (cur_sz.x / tex_sz.x)
			var t_sz := tex_sz * maxf(0.1, t_sc)
			if t_sz.x < 1280.0 or t_sz.y < 720.0:
				t_sz *= maxf(1280.0 / t_sz.x, 720.0 / t_sz.y)
			var min_x := minf(0.0, 1280.0 - t_sz.x)
			var min_y := minf(0.0, 720.0 - t_sz.y)
			var tx := cur_pos.x if (bool(m.get("keep_x", false)) or m.get("target_x") == null) else clampf(float(m["target_x"]), min_x, 0.0)
			var ty := cur_pos.y if (bool(m.get("keep_y", false)) or m.get("target_y") == null) else clampf(float(m["target_y"]), min_y, 0.0)
			var dur := maxf(0.0, float(m.get("duration_ms", 0)) / 1000.0)
			if dur <= 0.01:
				var snap_pos := Vector2(tx, ty)
				var snap_sz := t_sz
				_bg_pan_tween.tween_callback(func():
					background.size = snap_sz
					background.position = snap_pos
				)
			else:
				_bg_pan_tween.set_parallel(true)
				# 长距离运镜 (>=5s，如开场 40s 缓慢全景平移) 采用线性匀速确保首帧起即平稳不顿挫；短移采用正弦缓动
				var trans_type := Tween.TRANS_LINEAR if dur >= 5.0 else Tween.TRANS_SINE
				var ease_type := Tween.EASE_IN_OUT
				_bg_pan_tween.tween_property(background, "position", Vector2(tx, ty), dur).set_trans(trans_type).set_ease(ease_type)
				if not t_sz.is_equal_approx(cur_sz):
					_bg_pan_tween.tween_property(background, "size", t_sz, dur).set_trans(trans_type).set_ease(ease_type)
				_bg_pan_tween.set_parallel(false)
			cur_pos = Vector2(tx, ty)
			cur_sz = t_sz

func _slot_rect_from_bgi(slot_n: TextureRect, bgi_x: float, bgi_y: float, scale_pct: float) -> Dictionary:
	var tex_sz := slot_n.texture.get_size() if slot_n.texture != null else Vector2(680.0, 720.0)
	if tex_sz.x <= 0.0 or tex_sz.y <= 0.0:
		tex_sz = Vector2(680.0, 720.0)
	var sc := maxf(0.1, scale_pct / 100.0)
	var w := tex_sz.x * sc
	var h := tex_sz.y * sc
	return {
		"left": bgi_x - w * 0.5,
		"right": bgi_x + w * 0.5,
		"top": (360.0 + bgi_y - h * 0.5) - 720.0,
		"bottom": (360.0 + bgi_y + h * 0.5) - 720.0,
		"pos_x": 640.0 + bgi_x - w * 0.5,
		"pos_y": 360.0 + bgi_y - h * 0.5,
	}

# 舞台立绘几何定位与微动效果 (_DrawBS / _MoveBS / _MoveRepeatBS)
func _apply_slot_transforms_and_motions(step: Dictionary, instant: bool) -> void:
	var transforms: Variant = step.get("slot_transforms")
	if transforms is Dictionary:
		for skey in transforms.keys():
			var sid := int(skey)
			var slot_n: TextureRect = _get_slot_node(sid)
			if slot_n == null:
				continue
			if _slot_move_tweens.has(sid) and _slot_move_tweens[sid] is Tween and _slot_move_tweens[sid].is_valid():
				_slot_move_tweens[sid].kill()
			_slot_move_tweens.erase(sid)
			var tr_dict: Dictionary = transforms[skey]
			var bgi_x := float(tr_dict.get("x", 0))
			var bgi_y := float(tr_dict.get("y", 200))
			var bgi_sc := float(tr_dict.get("scale", 100))
			var rc := _slot_rect_from_bgi(slot_n, bgi_x, bgi_y, bgi_sc)
			slot_n.anchor_left = 0.5
			slot_n.anchor_right = 0.5
			slot_n.offset_left = rc["left"]
			slot_n.offset_right = rc["right"]
			slot_n.offset_top = rc["top"]
			slot_n.offset_bottom = rc["bottom"]
			var prev_n: TextureRect = _get_slot_prev_node(sid)
			if prev_n and not prev_n.visible:
				prev_n.anchor_left = 0.5
				prev_n.anchor_right = 0.5
				prev_n.offset_left = slot_n.offset_left
				prev_n.offset_right = slot_n.offset_right
				prev_n.offset_top = slot_n.offset_top
				prev_n.offset_bottom = slot_n.offset_bottom

	var motions: Variant = step.get("slot_motions")
	if not (motions is Array) or motions.is_empty():
		return
	if instant or _is_skip_active or _setting_effect_mode == 2 or DisplayServer.get_name() == "headless":
		return

	var move_groups: Dictionary = {}
	for sm in motions:
		if not (sm is Dictionary):
			continue
		var sid := int(sm.get("slot_id", 1))
		var slot_n: TextureRect = _get_slot_node(sid)
		if slot_n == null:
			continue
		if not move_groups.has(sid):
			move_groups[sid] = []
		move_groups[sid].append(sm)

	for sid in move_groups.keys():
		var slot_n: TextureRect = _get_slot_node(sid)
		var grp: Array = move_groups[sid]
		var tr_dict: Dictionary = transforms.get(sid, transforms.get(str(sid), {})) if (transforms is Dictionary) else {}
		var bgi_sc := float(tr_dict.get("scale", 100)) if (tr_dict is Dictionary) else 100.0
		var mtw := create_tween()
		_slot_move_tweens[sid] = mtw
		var initialized_move_start := false
		for sm in grp:
			if bool(sm.get("is_repeat", false)):
				var bounce_dy := float(sm.get("dy", 20))
				var dur := clampf(float(sm.get("duration_ms", 175)) / 1000.0, 0.05, 0.6)
				var base_y := slot_n.position.y
				mtw.tween_property(slot_n, "position:y", base_y + bounce_dy, dur * 0.5)
				mtw.tween_property(slot_n, "position:y", base_y, dur * 0.5)
			else:
				if not initialized_move_start:
					initialized_move_start = true
					var start_rc := _slot_rect_from_bgi(
						slot_n,
						float(sm.get("from_x", 0)),
						float(sm.get("from_y", 200)),
						bgi_sc
					)
					slot_n.position = Vector2(start_rc["pos_x"], start_rc["pos_y"])
				var end_rc := _slot_rect_from_bgi(
					slot_n,
					float(sm.get("to_x", 0)),
					float(sm.get("to_y", 200)),
					bgi_sc
				)
				var dur := clampf(float(sm.get("duration_ms", 100)) / 1000.0, 0.016, 15.0)
				mtw.tween_property(slot_n, "position", Vector2(end_rc["pos_x"], end_rc["pos_y"]), dur).set_trans(Tween.TRANS_SINE).set_ease(Tween.EASE_OUT)

# 对白文字与语音、BGM 应用
func _apply_dialogue_and_audio(step: Dictionary) -> void:
	dialogue_box.visible = true
	if face_avatar_layer:
		face_avatar_layer.visible = true
	choice_box.visible = false

	# 1. 说话人名称处理
	var speaker = step.get("speaker")
	if speaker != null and str(speaker).strip_edges() != "":
		speaker_label.text = str(speaker)
		speaker_label.visible = true
	else:
		speaker_label.text = ""
		speaker_label.visible = false

	# 2. 台词正文打字机流式动效
	var raw_text = step.get("text", "")
	_start_typewriter(raw_text)

	# 3. BGM 切换与 OSD 标题提示
	_apply_bgm_from_step(step)

	# 4. 原生语音流播放 (OGG Vorbis) + 双播放器乒乓短淡出 + 独立角色语音倍率 + Audio Ducking
	var voice = step.get("voice_stream")
	if voice != null and voice is AudioStream:
		var voice_asset := str(step.get("voice_asset", ""))
		_play_voice_stream(voice, voice_asset)

	# 5. _PlaySE 音效播放
	_play_step_se_streams(step)

	# 6. 右下角共同线彩蛋信封四态刷新与 mail_on_A 5 帧提示气泡（仅非 Skip 运行期触发）
	if step.has("mail_btn_state"):
		_update_mail_btn_state(int(step.get("mail_btn_state", 0)))
	_sync_mail_hud_visibility()
	if bool(step.get("mail_on_notify", false)) and not _is_skip_active and not _is_mail_mode:
		_play_mail_on_bubble()

# 统一应用 BGM 变更（目标 BGM 与当前在播 BGM 相同时免重头切歌停顿；目标静音时停止旧 BGM）
func _apply_bgm_from_step(step: Dictionary) -> void:
	if not bool(step.get("bgm_changed", false)) or bgm_player == null:
		return
	var bname := str(step.get("bgm_name", ""))
	var bstream = step.get("bgm_stream")
	if bstream != null and bstream is AudioStream:
		if _current_bgm_name != bname or not bgm_player.is_playing():
			_current_bgm_name = bname
			bgm_player.stream = bstream
			bgm_player.volume_db = _normal_bgm_volume_db
			if bgm_player.is_inside_tree():
				bgm_player.play()
				_show_bgm_osd(bname)
	else:
		if bgm_player.is_playing():
			bgm_player.stop()
		bgm_player.stream = null
		_current_bgm_name = ""

# 打字机流式动效
func _start_typewriter(target_text: String) -> void:
	if _typing_tween and _typing_tween.is_valid():
		_typing_tween.kill()

	_current_full_text = target_text
	text_label.text = target_text

	if click_icon:
		click_icon.visible = false

	# 无头模式 (Headless / 自动化验收测试)、快进模式或文本极速档下直接秒显
	if DisplayServer.get_name() == "headless" or _is_skip_active or _setting_text_speed >= 2.95:
		text_label.visible_ratio = 1.0
		_is_typing = false
		if click_icon:
			click_icon.visible = true
		if _is_skip_active:
			_schedule_skip_step()
		elif _is_auto_active:
			_schedule_auto_step()
		return

	# 计算打字流速 (基准约 30 字/秒，受系统设置 _setting_text_speed 调控)
	var text_len = target_text.length()
	var speed_mul = maxf(0.25, _setting_text_speed)
	var duration = clampf((text_len * 0.028) / speed_mul, 0.15, 1.8)

	text_label.visible_ratio = 0.0
	_is_typing = true

	_typing_tween = create_tween()
	_typing_tween.tween_property(text_label, "visible_ratio", 1.0, duration)
	_typing_tween.finished.connect(_on_typing_finished)

# 打字完成回调
func _on_typing_finished() -> void:
	_is_typing = false
	text_label.visible_ratio = 1.0
	if click_icon:
		click_icon.visible = true

	# 自动播放模式驱动
	if _is_auto_active:
		_schedule_auto_step()

# 即时展示全部文字
func _instant_reveal_text() -> void:
	if _typing_tween and _typing_tween.is_valid():
		_typing_tween.kill()
	_on_typing_finished()

# 调度快进模式下一步推进 (process_always = false，失焦暂停时不穿透触发)
func _schedule_skip_step() -> void:
	if not is_inside_tree():
		return
	var tree = get_tree()
	if tree:
		tree.create_timer(0.08, false).timeout.connect(func():
			if _is_skip_active and not _is_terminated and not _is_waiting_for_choice:
				advance_dialogue()
		)

# 调度自动模式下一步推进 (process_always = false，失焦暂停时不穿透触发)
func _schedule_auto_step() -> void:
	if not is_inside_tree():
		return
	var tree = get_tree()
	if tree == null:
		return
	# 检查语音是否在播放，语音播完后再延时 0.6s，若无语音则按字数延时 1.8s (受 _setting_auto_speed 调控)
	var auto_mul = maxf(0.4, _setting_auto_speed)
	var wait_time = 1.8 / auto_mul
	if voice_player and voice_player.is_playing() and voice_player.stream:
		var stream_len = voice_player.stream.get_length()
		var pos = voice_player.get_playback_position()
		wait_time = maxf(0.4, stream_len - pos + (0.5 / auto_mul))

	tree.create_timer(wait_time, false).timeout.connect(func():
		if _is_auto_active and not _is_typing and not _is_terminated and not _is_waiting_for_choice:
			advance_dialogue()
	)

# FaceAvatar 差分平滑淡入淡出逻辑 (独立不裁切图层，精确对齐 PC (-207, 272, 612, 828))
func _update_face_avatar(face_tex: Variant, _speaker: Variant, instant: bool = false, face_offset: Variant = null) -> void:
	if face_avatar == null or face_avatar_prev == null:
		return

	var target_tex: Texture2D = face_tex if (face_tex != null and face_tex is Texture2D) else null

	if target_tex != null:
		var fx: float = -541.0
		var fy: float = 451.0
		if face_offset is Dictionary:
			fx = float(face_offset.get("x", -541.0))
			fy = float(face_offset.get("y", 451.0))
		var target_pos := Vector2(-207.0 + (fx + 541.0), 272.0 + (fy - 451.0))
		var target_sz := Vector2(612.0, 828.0)
		face_avatar.position = target_pos
		face_avatar.size = target_sz
		face_avatar_prev.position = target_pos
		face_avatar_prev.size = target_sz
		if face_avatar_layer:
			face_avatar_layer.visible = not _is_ui_hidden

		if margin_container:
			margin_container.add_theme_constant_override("margin_left", 232)

		if _face_tween and _face_tween.is_valid():
			_face_tween.kill()
		_face_tween = null

		var old_tex = face_avatar.texture
		if instant or DisplayServer.get_name() == "headless":
			face_avatar_prev.visible = false
			face_avatar_prev.texture = null
			face_avatar_prev.modulate.a = 0.0
			face_avatar.texture = target_tex
			face_avatar.modulate.a = 1.0
			face_avatar.visible = true
			return

		if old_tex == target_tex and face_avatar.visible:
			return

		# 进场 (空槽 -> 有头像)
		if old_tex == null or not face_avatar.visible:
			face_avatar_prev.visible = false
			face_avatar_prev.texture = null
			face_avatar.texture = target_tex
			face_avatar.modulate.a = 0.0
			face_avatar.visible = true
			_face_tween = create_tween()
			_face_tween.tween_property(face_avatar, "modulate:a", 1.0, SPRITE_FADE_SEC)
			return

		# 换表情：底层 SlotPrev 锁定 1.0 不动，顶层 200ms 淡入覆盖，杜绝背景鬼影
		face_avatar_prev.texture = old_tex
		face_avatar_prev.modulate.a = 1.0
		face_avatar_prev.visible = true

		face_avatar.texture = target_tex
		face_avatar.modulate.a = 0.0
		face_avatar.visible = true

		_face_tween = create_tween()
		_face_tween.tween_property(face_avatar, "modulate:a", 1.0, SPRITE_FADE_SEC)
		_face_tween.finished.connect(func():
			face_avatar_prev.visible = false
			face_avatar_prev.texture = null
			face_avatar_prev.modulate.a = 0.0
		)
	else:
		if _face_tween and _face_tween.is_valid():
			_face_tween.kill()
		_face_tween = null
		# 无头像或头像清空 (_FadeBS(9) / _DrawScene)，平滑淡出并收窄文本左边界
		if instant or DisplayServer.get_name() == "headless":
			face_avatar.visible = false
			face_avatar.texture = null
			face_avatar.modulate.a = 0.0
			face_avatar_prev.visible = false
			face_avatar_prev.texture = null
			face_avatar_prev.modulate.a = 0.0
			if margin_container:
				margin_container.add_theme_constant_override("margin_left", 60)
			return

		if face_avatar.visible and face_avatar.texture != null:
			_face_tween = create_tween()
			_face_tween.tween_property(face_avatar, "modulate:a", 0.0, SPRITE_FADE_SEC)
			_face_tween.finished.connect(func():
				face_avatar.visible = false
				face_avatar.texture = null
				face_avatar.modulate.a = 0.0
				if margin_container:
					margin_container.add_theme_constant_override("margin_left", 60)
			)

# Audio Ducking: 语音播放时平滑压低 BGM 音量 -6dB
func _duck_bgm_volume() -> void:
	if bgm_player == null:
		return
	if _duck_tween and _duck_tween.is_valid():
		_duck_tween.kill()
	_duck_tween = create_tween()
	_duck_tween.tween_property(bgm_player, "volume_db", _ducked_bgm_volume_db, 0.2)

# 恢复 BGM 音量至标准电平
func _restore_bgm_volume() -> void:
	if bgm_player == null:
		return
	if _duck_tween and _duck_tween.is_valid():
		_duck_tween.kill()
	_duck_tween = create_tween()
	_duck_tween.tween_property(bgm_player, "volume_db", _normal_bgm_volume_db, 0.4)

func _setup_voice_players() -> void:
	if voice_player_alt == null:
		voice_player_alt = get_node_or_null("VoicePlayerAlt") as AudioStreamPlayer
	if voice_player_alt == null:
		voice_player_alt = AudioStreamPlayer.new()
		voice_player_alt.name = "VoicePlayerAlt"
		add_child(voice_player_alt)
	_voice_players = [voice_player, voice_player_alt]
	_active_voice_idx = 0
	for idx in range(_voice_players.size()):
		var p: AudioStreamPlayer = _voice_players[idx]
		if p and not p.finished.is_connected(_on_voice_slot_finished.bind(idx)):
			p.finished.connect(_on_voice_slot_finished.bind(idx))

func _extract_char_code_from_voice(voice_asset: String) -> String:
	var lower := voice_asset.strip_edges().to_lower()
	if lower.is_empty():
		return ""
	if lower.begins_with("voice_"):
		var suf := lower.trim_prefix("voice_")
		return "" if suf == "all" else suf
	if lower.length() >= 2 and (lower[0] == "w" or lower[0] == "m") and lower[1].is_valid_int():
		return lower.substr(0, 1)
	var vparts := lower.split("_")
	if not vparts.is_empty() and vparts[0] != "":
		return vparts[0]
	return lower.substr(0, 2) if lower.length() >= 2 else lower

func _calc_voice_target_db(voice_asset: String) -> float:
	var char_mul := 1.0
	var lower := voice_asset.strip_edges().to_lower()
	if not lower.is_empty():
		var char_code := _extract_char_code_from_voice(lower)
		if char_code != "" and _setting_char_vols.has(char_code):
			char_mul = float(_setting_char_vols[char_code])
		elif lower.length() >= 2 and _setting_char_vols.has(lower.substr(0, 2)):
			char_mul = float(_setting_char_vols[lower.substr(0, 2)])
	var eff_vol := clampf(_setting_voice_vol * char_mul, 0.0, 1.0)
	return -80.0 if eff_vol <= 0.001 else linear_to_db(eff_vol)

func _is_any_voice_playing() -> bool:
	for p in _voice_players:
		if p and p.is_inside_tree() and p.is_playing():
			return true
	return false

func _stop_voice_slot_soft(slot_idx: int, fade_sec: float = VOICE_FADE_OUT_SEC) -> void:
	if slot_idx < 0 or slot_idx >= _voice_players.size():
		return
	var old_tw = _voice_fade_tweens[slot_idx]
	if old_tw is Tween and old_tw.is_valid():
		old_tw.kill()
	_voice_fade_tweens[slot_idx] = null
	var p: AudioStreamPlayer = _voice_players[slot_idx]
	if p == null:
		return
	p.stream_paused = false
	if not p.is_inside_tree() or not p.is_playing():
		p.stop()
		p.stream = null
		return
	if DisplayServer.get_name() == "headless" or fade_sec <= 0.001:
		p.stop()
		p.stream = null
		return
	var tw := p.create_tween()
	_voice_fade_tweens[slot_idx] = tw
	tw.tween_property(p, "volume_db", -80.0, fade_sec)
	tw.tween_callback(func():
		p.stop()
		p.stream = null
		_voice_fade_tweens[slot_idx] = null
	)

func _stop_all_voices(immediate: bool = false) -> void:
	for idx in range(_voice_players.size()):
		if immediate:
			var old_tw = _voice_fade_tweens[idx]
			if old_tw is Tween and old_tw.is_valid():
				old_tw.kill()
			_voice_fade_tweens[idx] = null
			var p: AudioStreamPlayer = _voice_players[idx]
			if p:
				p.stream_paused = false
				if p.is_playing():
					p.stop()
				p.stream = null
				p.volume_db = _calc_voice_target_db(_current_voice_asset)
		else:
			_stop_voice_slot_soft(idx, VOICE_FADE_OUT_SEC)
	_restore_bgm_volume()

func _play_voice_stream(stream: AudioStream, voice_asset: String) -> void:
	if stream == null:
		return
	if _voice_players.is_empty():
		_setup_voice_players()
	var old_idx := _active_voice_idx
	var new_idx := 1 - old_idx if _voice_players.size() >= 2 else 0
	if new_idx != old_idx:
		_stop_voice_slot_soft(old_idx, VOICE_FADE_OUT_SEC)

	var old_tw = _voice_fade_tweens[new_idx]
	if old_tw is Tween and old_tw.is_valid():
		old_tw.kill()
	_voice_fade_tweens[new_idx] = null

	var np: AudioStreamPlayer = _voice_players[new_idx]
	if np == null:
		return
	if np.is_playing():
		np.stop()
	np.stream_paused = false
	_current_voice_asset = voice_asset
	np.volume_db = _calc_voice_target_db(voice_asset)
	np.stream = stream
	_active_voice_idx = new_idx
	voice_player = np
	if np.is_inside_tree():
		np.play()
		_duck_bgm_volume()

func _on_voice_slot_finished(slot_idx: int) -> void:
	if slot_idx == _active_voice_idx:
		_restore_bgm_volume()

func _on_voice_finished() -> void:
	_restore_bgm_volume()

# 纯净赏析隐藏 UI
func _hide_ui() -> void:
	if _is_mail_mode:
		return
	_reset_quick_menu_label_offsets()
	if _is_ui_hidden:
		return
	_is_ui_hidden = true
	dialogue_box.visible = false
	if face_avatar_layer:
		face_avatar_layer.visible = false
	if choice_box:
		choice_box.visible = false
	_sync_mail_hud_visibility()
	_show_notify("纯净赏析模式（点击或右键恢复 UI）")

# 恢复 UI 显示
func _show_ui() -> void:
	if _is_mail_mode:
		return
	_reset_quick_menu_label_offsets()
	if not _is_ui_hidden:
		return
	_is_ui_hidden = false
	var keep_dlg_hidden: bool = _is_waiting_for_wait_click or (_is_waiting_for_choice and _hide_window_when_selecting)
	dialogue_box.visible = not keep_dlg_hidden
	if face_avatar_layer:
		face_avatar_layer.visible = not keep_dlg_hidden
	if _is_waiting_for_choice and choice_box:
		choice_box.visible = true
	_sync_mail_hud_visibility()

# 应用/恢复 ChoiceBox 排版（方案 3B：手机在屏且 0x0167(1) 时右移避让，4:3 自动收窄防裁切）
func _apply_choice_box_layout(use_right_avoid: bool) -> void:
	if choice_box == null:
		return
	if use_right_avoid:
		_choice_box_right_shifted = true
		if dialogue_box:
			dialogue_box.visible = false
		if face_avatar_layer:
			face_avatar_layer.visible = false
		choice_box.anchor_left = 0.0
		choice_box.anchor_right = 0.0
		choice_box.anchor_top = 0.5
		choice_box.anchor_bottom = 0.5
		if _setting_screen_mode == 1:
			# 4:3 裁剪模式安全区：x = 805..1105（宽 300px，严守 1120px 右黑边内）
			choice_box.offset_left = 805.0
			choice_box.offset_right = 1105.0
			choice_box.offset_top = -140.0
			choice_box.offset_bottom = 140.0
		else:
			# 16:9 模式右侧避让区：x = 820..1240（宽 420px，垂直居中）
			choice_box.offset_left = 820.0
			choice_box.offset_right = 1240.0
			choice_box.offset_top = -150.0
			choice_box.offset_bottom = 150.0
		var btn_min_w := 240.0 if _setting_screen_mode == 1 else 360.0
		var btn_font_sz := 17 if _setting_screen_mode == 1 else 19
		if choice_options_container:
			for child in choice_options_container.get_children():
				if child is Button:
					child.custom_minimum_size = Vector2(btn_min_w, 48)
					child.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
					child.add_theme_font_size_override("font_size", btn_font_sz)
	else:
		_restore_choice_box_layout()

func _restore_choice_box_layout() -> void:
	if choice_box == null or not _choice_box_right_shifted:
		return
	_choice_box_right_shifted = false
	choice_box.anchor_left = 0.5
	choice_box.anchor_right = 0.5
	choice_box.anchor_top = 0.5
	choice_box.anchor_bottom = 0.5
	choice_box.offset_left = -320.0
	choice_box.offset_top = -180.0
	choice_box.offset_right = 320.0
	choice_box.offset_bottom = 180.0

# 显示分支选项弹窗（支持 0x0166 DisableItem 灰显禁用，绝不剔除选项以保证 FLAG[0] 下标精确）
func _show_choices(prompt: String, options: Array, choice_seq: int = 0, hide_window: bool = false, disabled_options: Array = []) -> void:
	_is_waiting_for_choice = true
	_pending_choice_seq = choice_seq
	_wake_mouse_cursor()
	choice_box.visible = true
	# 弹出选项瞬间强制停止 Auto 自动播放；Skip 是否中断读取设置“跳过持续范围”
	_is_auto_active = false
	if _setting_skip_continue_mode == 1 or _is_mail_mode:
		_is_skip_active = false

	var use_right_avoid: bool = (not _active_spr_asset.is_empty()) and hide_window
	if hide_window and not use_right_avoid:
		if dialogue_box:
			dialogue_box.visible = false
		if face_avatar_layer:
			face_avatar_layer.visible = false

	var disabled_set: Dictionary = {}
	if disabled_options is Array:
		for d in disabled_options:
			disabled_set[int(d)] = true

	if choice_prompt_label:
		if not prompt.is_empty():
			choice_prompt_label.text = prompt
			choice_prompt_label.visible = true
		else:
			choice_prompt_label.visible = false

	# 清空原有按钮
	for child in choice_options_container.get_children():
		child.queue_free()

	var btn_min_w: float = 560.0
	var btn_font_sz: int = 20
	if use_right_avoid:
		btn_min_w = 240.0 if _setting_screen_mode == 1 else 360.0
		btn_font_sz = 17 if _setting_screen_mode == 1 else 19

	# 动态生成分支选项按钮（保留全部原下标，禁用项灰显不可点）
	for i in range(options.size()):
		var btn = Button.new()
		btn.text = str(options[i])
		btn.custom_minimum_size = Vector2(btn_min_w, 48)
		btn.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		if use_right_avoid:
			btn.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		if _sys_font:
			btn.add_theme_font_override("font", _sys_font)
		btn.add_theme_font_size_override("font_size", btn_font_sz)
		if disabled_set.has(i):
			btn.disabled = true
			btn.focus_mode = Control.FOCUS_NONE
			btn.modulate = Color(0.65, 0.68, 0.74, 0.52)
			btn.add_theme_color_override("font_disabled_color", Color(0.72, 0.76, 0.82, 0.85))
		else:
			var choice_idx = i
			var seq_cap = choice_seq
			btn.pressed.connect(func(): _on_choice_selected(choice_idx, seq_cap))
		choice_options_container.add_child(btn)

	_apply_choice_box_layout(use_right_avoid)
	_sync_mail_hud_visibility()

# 处理分支选择点击 (带单调 choice_seq 防重入校验)
func _on_choice_selected(index: int, choice_seq: int = -1) -> void:
	if not _is_waiting_for_choice or engine == null:
		return
	if choice_seq >= 0 and choice_seq != _pending_choice_seq:
		return

	print("[PureConnect] 玩家选择分支: %d (seq=%d)" % [index, _pending_choice_seq])
	var ok: bool = false
	if _pending_choice_seq > 0:
		ok = engine.submit_choice(_pending_choice_seq, index)
	else:
		ok = engine.select_choice(index)
	if not ok:
		push_error("PureConnectEngine::submit_choice(%d, %d) 返回失败" % [_pending_choice_seq, index])
		return

	_pending_choice_seq += 1
	_is_waiting_for_choice = false
	choice_box.visible = false
	_restore_choice_box_layout()
	_flush_system_flags_if_dirty()
	if _is_typing:
		_instant_reveal_text()

	# 恢复推进后续对白
	advance_dialogue()

# Quick Menu 响应函数群
func _do_quick_save() -> void:
	if engine == null or _is_in_title_screen or _is_mail_mode:
		return
	if _is_waiting_for_wait_click:
		_show_notify("【短信翻页期间暂不可保存】")
		return
	_perform_save_entry("quicksave")
	_quick_saved_index = 1
	_show_notify("【快捷存档已保存】")

func _do_quick_load() -> void:
	if engine == null or _is_mail_mode:
		return
	if _is_waiting_for_wait_click:
		_show_notify("【短信翻页期间暂不可快读】")
		return
	if SettingsPanelScript.has_quicksave():
		var data := SettingsPanelScript.load_save_metadata("quicksave")
		if not data.is_empty():
			_perform_load_save_data(data)
			_show_notify("【快捷存档已恢复】")
			return
	_show_notify("【暂无可用快捷存档】")

func _on_quick_save() -> void:
	_reset_quick_menu_label_offsets()
	if engine == null or _is_in_title_screen or _is_mail_mode:
		return
	if _is_waiting_for_wait_click:
		_show_notify("【短信翻页期间暂不可保存】")
		return
	if DisplayServer.get_name() != "headless" and _setting_qsave_confirm:
		_setup_settings_panel()
		settings_panel.open_standalone_confirm("要快速保存吗？", _do_quick_save, "SGDialog990007")
		return
	_do_quick_save()

func _on_quick_load() -> void:
	_reset_quick_menu_label_offsets()
	if engine == null or _is_mail_mode:
		return
	if _is_waiting_for_wait_click:
		_show_notify("【短信翻页期间暂不可快读】")
		return
	if not SettingsPanelScript.has_quicksave():
		_show_notify("【暂无可用快捷存档】")
		return
	if DisplayServer.get_name() != "headless" and _setting_qload_confirm:
		_setup_settings_panel()
		settings_panel.open_standalone_confirm("要读取快捷存档吗？", _do_quick_load, "")
		return
	_do_quick_load()

func _perform_save_to_slot(global_slot: int) -> void:
	if _is_mail_mode:
		return
	if _is_waiting_for_wait_click:
		_show_notify("【短信翻页期间暂不可保存】")
		return
	var key := "slot_%03d" % global_slot
	_perform_save_entry(key)
	if settings_panel:
		settings_panel.refresh_saves_view()
	_show_notify("【已保存至存档槽 No.%03d】" % (global_slot + 1))

func _perform_save_entry(slot_key: String) -> void:
	if engine == null or _is_mail_mode:
		return
	var meta: Dictionary = engine.get_save_state()
	if meta.is_empty():
		return
	meta["timestamp"] = Time.get_datetime_string_from_system(false, true).replace("T", " ").substr(0, 16)
	var thumb_img: Image = null
	if settings_panel != null and settings_panel.visible and _cached_scene_thumb != null and not _cached_scene_thumb.is_empty():
		thumb_img = _cached_scene_thumb
	elif DisplayServer.get_name() != "headless" and get_viewport() and get_viewport().get_texture():
		var was_cfg_vis: bool = (settings_panel != null and settings_panel.visible)
		if was_cfg_vis:
			settings_panel.visible = false
		thumb_img = get_viewport().get_texture().get_image()
		if was_cfg_vis:
			settings_panel.visible = true
	SettingsPanelScript.write_save_entry(slot_key, meta, thumb_img)

func _perform_load_save_data(save_data: Dictionary) -> void:
	_reset_quick_menu_label_offsets()
	if engine == null or save_data.is_empty():
		return
	var sname := str(save_data.get("script_name", DEFAULT_SCRIPT))
	if sname.is_empty():
		sname = DEFAULT_SCRIPT
	var didx := int(save_data.get("dialogue_index", 0))
	var raw_choices = save_data.get("choices", [])
	var typed_choices: Array[int] = []
	if raw_choices is Array:
		for c in raw_choices:
			typed_choices.append(int(c))

	if _is_in_title_screen:
		_is_in_title_screen = false
		if title_screen:
			title_screen.visible = false

	if _scene_tween and _scene_tween.is_valid():
		_scene_tween.kill()
	if _bg_pan_tween and _bg_pan_tween.is_valid():
		_bg_pan_tween.kill()
	if _typing_tween and _typing_tween.is_valid():
		_typing_tween.kill()
	_is_transitioning = false
	_is_typing = false
	_is_in_character_intro = false
	_pending_snapshot = {}
	if transition_overlay:
		transition_overlay.visible = false
		transition_overlay.modulate.a = 0.0

	_seen_character_intros.clear()
	_is_terminated = false
	_is_mail_mode = false
	_is_waiting_for_wait_click = false
	_wait_click_seq += 1
	_is_waiting_for_choice = false
	_is_auto_active = false
	_is_skip_active = false
	_stop_mail_on_bubble()
	_kill_mail_pulse_tween()
	_reset_mail_btn_hover_state()
	_stop_all_voices(true)
	if se_player and se_player.is_playing():
		se_player.stop()
	_sync_engine_config_regs()
	if choice_box:
		choice_box.visible = false
	if background:
		background.texture = null
	_clear_spr_overlay_instant()
	_reset_all_slots()
	_update_face_avatar(null, null, true)
	_update_pillarbox_visibility()

	var restored: Dictionary
	if save_data.has("save_json") and str(save_data["save_json"]) != "":
		restored = engine.load_save_state_json(str(save_data["save_json"]))
	elif int(save_data.get("save_version", 1)) >= 2:
		restored = engine.load_save_state_json(JSON.stringify(save_data))
	else:
		restored = engine.load_save_state(sname, didx, typed_choices)

	var restored_state := str(restored.get("state", ""))
	if restored_state == "Error" or restored_state == "Terminated":
		_apply_snapshot(restored)
		return

	_apply_stage_visuals_instant(restored)
	if restored_state == "WaitingForChoice":
		var hide_win := bool(restored.get("hide_window_when_selecting", false))
		_hide_window_when_selecting = hide_win
		if int(restored.get("index", 0)) > 0 or str(restored.get("text", "")) != "":
			_apply_dialogue_and_audio(restored)
			if _is_typing:
				_instant_reveal_text()
		else:
			_apply_bgm_from_step(restored)
			if dialogue_box:
				dialogue_box.visible = not hide_win
			if speaker_label:
				speaker_label.text = ""
				speaker_label.visible = false
			if text_label:
				text_label.text = ""
				text_label.visible_ratio = 1.0
		_show_choices(
			str(restored.get("choice_prompt", "")),
			restored.get("choice_options", []),
			int(restored.get("choice_seq", 0)),
			hide_win,
			restored.get("disabled_options", [])
		)
	else:
		_apply_dialogue_and_audio(restored)
	_update_mail_btn_state(int(restored.get("mail_btn_state", engine.get_mail_btn_state())))
	_sync_mail_hud_visibility()

func _toggle_auto_mode() -> void:
	if _is_mail_mode:
		return
	_is_auto_active = not _is_auto_active
	if _is_auto_active:
		_is_skip_active = false
		_show_notify("【自动播放模式：开启】")
		if _is_waiting_for_wait_click:
			_wait_click_seq += 1
			_schedule_wait_click_auto(_wait_click_seq)
		elif not _is_typing:
			_schedule_auto_step()
	else:
		_show_notify("【自动播放模式：关闭】")

func _toggle_skip_mode() -> void:
	if _is_mail_mode:
		return
	_is_skip_active = not _is_skip_active
	if _is_skip_active:
		_is_auto_active = false
		_stop_mail_on_bubble()
		if _is_transitioning:
			_skip_active_transition()
		_flush_all_slots()
		_show_notify("【快进跳过模式：开启】")
		if _is_waiting_for_wait_click:
			_wait_click_seq += 1
			_schedule_wait_click_skip(_wait_click_seq)
		else:
			advance_dialogue()
	else:
		_show_notify("【快进跳过模式：关闭】")

func _on_config() -> void:
	_open_settings()

# Backlog 履历面板相关逻辑 (消灭双份状态，直接查询 Rust VmEngine)
func _open_backlog() -> void:
	if _is_mail_mode:
		return
	_reset_quick_menu_label_offsets()
	if backlog_panel == null or engine == null:
		return

	# 打开 Backlog 时强制暂停 AUTO / SKIP 计时器，避免后台悄悄跳句
	if _is_auto_active:
		_is_auto_active = false
		_show_notify("【进入履历：自动播放已暂停】")
	if _is_skip_active:
		_is_skip_active = false

	# 清空现有条目
	for child in history_list.get_children():
		child.queue_free()

	# 从 Rust VFS/VM 原生拉取历史记录
	var history_entries = engine.get_history(0, 100)

	if history_entries.is_empty():
		var empty_lbl = Label.new()
		empty_lbl.text = "暂无历史对白履历"
		if _sys_font:
			empty_lbl.add_theme_font_override("font", _sys_font)
		empty_lbl.add_theme_font_size_override("font_size", 18)
		empty_lbl.add_theme_color_override("font_color", Color(0.6, 0.6, 0.6, 1.0))
		history_list.add_child(empty_lbl)
	else:
		for item in history_entries:
			var entry_box = VBoxContainer.new()
			entry_box.add_theme_constant_override("separation", 4)

			var header_row = HBoxContainer.new()
			var spk = str(item.get("speaker", "")).strip_edges()
			if not spk.is_empty():
				var spk_lbl = Label.new()
				spk_lbl.text = "【" + spk + "】"
				spk_lbl.add_theme_color_override("font_color", Color(1.0, 0.85, 0.45, 1.0))
				if _sys_font:
					spk_lbl.add_theme_font_override("font", _sys_font)
				spk_lbl.add_theme_font_size_override("font_size", 18)
				header_row.add_child(spk_lbl)

			var voice_id = str(item.get("voice", "")).strip_edges()
			if not voice_id.is_empty():
				var voice_btn = Button.new()
				voice_btn.text = " 🔊 语音 "
				if _sys_font:
					voice_btn.add_theme_font_override("font", _sys_font)
				voice_btn.add_theme_font_size_override("font_size", 14)
				var v_capture = voice_id
				voice_btn.pressed.connect(func(): _replay_history_voice(v_capture))
				header_row.add_child(voice_btn)

			entry_box.add_child(header_row)

			var text_content = Label.new()
			text_content.text = str(item.get("text", ""))
			text_content.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
			text_content.add_theme_color_override("font_color", Color(0.9, 0.95, 1.0, 1.0))
			if _sys_font:
				text_content.add_theme_font_override("font", _sys_font)
			text_content.add_theme_font_size_override("font_size", 18)
			entry_box.add_child(text_content)

			# 分隔线
			var hs = HSeparator.new()
			hs.add_theme_constant_override("separation", 8)
			entry_box.add_child(hs)

			history_list.add_child(entry_box)

	backlog_panel.visible = true
	_wake_mouse_cursor()
	_update_pillarbox_visibility()
	_sync_mail_hud_visibility()

	# 延时滚至底部 (process_always = false，失焦暂停时不穿透触发)
	if is_inside_tree():
		var tree = get_tree()
		if tree:
			tree.create_timer(0.05, false).timeout.connect(func():
				var scroll: ScrollContainer = backlog_panel.get_node_or_null("ScrollContainer")
				if scroll:
					scroll.scroll_vertical = 999999
			)

func _close_backlog() -> void:
	if backlog_panel:
		backlog_panel.visible = false
	_update_pillarbox_visibility()
	_sync_mail_hud_visibility()

# 重听历史语音
func _replay_history_voice(voice_id: String) -> void:
	if engine == null or voice_player == null:
		return
	var stream = engine.play_history_voice(voice_id)
	if stream != null and stream is AudioStream:
		_play_voice_stream(stream, voice_id)
	else:
		# 无效语音或未加载到流时，确保不残留旧语音或非法流
		_stop_all_voices(true)
		_current_voice_asset = ""

# 浮动提示通知框
func _show_notify(msg: String) -> void:
	if quick_notify == null:
		return
	if _notify_tween and _notify_tween.is_valid():
		_notify_tween.kill()
	quick_notify.z_index = 100
	quick_notify.move_to_front()
	quick_notify.text = msg
	quick_notify.visible = true
	quick_notify.modulate.a = 1.0
	_notify_tween = create_tween()
	_notify_tween.tween_interval(1.2)
	_notify_tween.tween_property(quick_notify, "modulate:a", 0.0, 0.4)
	_notify_tween.finished.connect(func():
		quick_notify.visible = false
	)

# 女主角元数据字典（用于新角色登场特写与卡片排版）
const HEROINE_METADATA = {
	"sora": {
		"name_cn": "一瀬 空",
		"name_romaji": "ICHINOSE SORA",
		"motto": "青梅竹马什么的，最喜欢了！",
		"color": Color(0.35, 0.70, 0.95, 1.0),
		"sprite": "SO01_N001",
	},
	"moemi": {
		"name_cn": "道明寺 萌美",
		"name_romaji": "DOUMYOUJI MOEMI",
		"motto": "今天也要打起精神，制作最棒的甜点！",
		"color": Color(0.98, 0.60, 0.35, 1.0),
		"sprite": "MO01_M001",
	},
	"arisa": {
		"name_cn": "绀野 彩里沙",
		"name_romaji": "KONNO ARISA",
		"motto": "命运之人什么的，完全不存在！",
		"color": Color(0.95, 0.82, 0.32, 1.0),
		"sprite": "AR01_N001",
	},
	"ayumi": {
		"name_cn": "筱崎 步海",
		"name_romaji": "SHINOZAKI AYUMI",
		"motto": "如果能一直在一起的话，该有多好呢。",
		"color": Color(0.96, 0.48, 0.68, 1.0),
		"sprite": "AY01_N001",
	},
	"shiho": {
		"name_cn": "牧原 志帆",
		"name_romaji": "MAKIHARA SHIHO",
		"motto": "好，今天也加把劲，努力工作吧！",
		"color": Color(0.72, 0.60, 0.92, 1.0),
		"sprite": "SI01_N001",
	},
}

# ==============================================================================
# 新角色登场转场介绍特写 (1:1 复刻 PC 5 图层 ec_{code}_01..05 交错滑入与 Jingle)
# ==============================================================================
func _setup_character_intro_layer() -> void:
	if char_intro_layer != null:
		return

	char_intro_layer = CharacterIntroScript.new()
	add_child(char_intro_layer)
	char_intro_layer.setup_font(_custom_font)
	char_intro_layer.intro_finished.connect(_finish_character_intro)

	var fb: Dictionary = char_intro_layer.get_fallback_labels()
	intro_card_panel = fb.get("panel") as Control
	intro_card_rect = fb.get("card_rect") as TextureRect
	intro_name_lbl = fb.get("name") as Label
	intro_romaji_lbl = fb.get("romaji") as Label
	intro_motto_lbl = fb.get("motto") as Label
	intro_tag_lbl = fb.get("tag") as Label
	intro_jingle_player = char_intro_layer.jingle_player

func _run_character_intro(intro: Dictionary, step: Dictionary) -> void:
	_setup_character_intro_layer()
	_current_intro_snapshot = step
	var se_db := -80.0 if _setting_se_vol <= 0.001 else linear_to_db(_setting_se_vol)
	var v_db := -80.0 if _setting_voice_vol <= 0.001 else linear_to_db(_setting_voice_vol)
	char_intro_layer.set_audio_volumes(v_db, se_db)

	# 若只传单个 card_texture 且未带 layers，确保回退纹理兜底解码
	if (intro.get("card_texture") == null) and engine:
		var char_id := str(intro.get("character", "")).to_lower()
		var meta: Dictionary = HEROINE_METADATA.get(char_id, {})
		var fb_sprite := str(meta.get("sprite", ""))
		if not fb_sprite.is_empty():
			intro["card_texture"] = engine.load_ui_texture(fb_sprite)

	if _is_skip_active or DisplayServer.get_name() == "headless":
		char_intro_layer.play_intro(intro, true)
		char_intro_layer.visible = false
		_apply_dialogue_and_audio(step)
		return

	_is_in_character_intro = true
	_intro_start_frame = Engine.get_process_frames()
	dialogue_box.visible = false
	if face_avatar_layer:
		face_avatar_layer.visible = false
	_sync_mail_hud_visibility()
	char_intro_layer.play_intro(intro, false)

func _skip_character_intro() -> void:
	if not _is_in_character_intro:
		return
	if char_intro_layer and (char_intro_layer._is_playing or char_intro_layer.visible):
		char_intro_layer.skip()
	else:
		_finish_character_intro()

func _finish_character_intro() -> void:
	_is_in_character_intro = false
	dialogue_box.visible = true
	if face_avatar_layer:
		face_avatar_layer.visible = true
	if not _current_intro_snapshot.is_empty():
		var snap = _current_intro_snapshot
		_current_intro_snapshot = {}
		_apply_dialogue_and_audio(snap)

# ==============================================================================
# 主标题画面 TitleScreen (1:1 复刻 PC 原版 SGTitle990000 + 6 键与 OMAKE 鉴赏)
# ==============================================================================
func _setup_title_screen() -> void:
	if title_screen != null:
		title_screen.setup(engine, _custom_font)
		return

	title_screen = TitleScreenScript.new()
	add_child(title_screen)
	title_screen.setup(engine, _custom_font)
	title_screen.start_new_game_requested.connect(_on_title_start_new_game)
	title_screen.continue_load_requested.connect(_on_title_continue_game)
	title_screen.quick_load_requested.connect(_on_title_quick_load)
	title_screen.config_requested.connect(_on_title_settings)
	title_screen.quit_game_requested.connect(_on_title_quit_game)
	title_screen.play_se_requested.connect(_play_ui_se)
	title_screen.play_bgm_requested.connect(_play_named_bgm)

func _show_title_screen() -> void:
	_is_in_title_screen = true
	_is_mail_mode = false
	_stop_mail_on_bubble()
	_kill_mail_pulse_tween()
	_wake_mouse_cursor()
	dialogue_box.visible = false
	if face_avatar_layer:
		face_avatar_layer.visible = false
	if choice_box:
		choice_box.visible = false
	_setup_title_screen()
	title_screen.visible = true
	title_screen.modulate.a = 1.0
	title_screen.move_to_front()
	_update_pillarbox_visibility()
	_sync_mail_hud_visibility()

	_play_named_bgm("title_bgm")

func _return_to_title_screen() -> void:
	if _is_mail_mode:
		return
	_stop_all_voices(true)
	_stop_mail_on_bubble()
	_kill_mail_pulse_tween()
	if _scene_tween and _scene_tween.is_valid():
		_scene_tween.kill()
	if _bg_pan_tween and _bg_pan_tween.is_valid():
		_bg_pan_tween.kill()
	if _typing_tween and _typing_tween.is_valid():
		_typing_tween.kill()
	if _title_fade_tween and _title_fade_tween.is_valid():
		_title_fade_tween.kill()
	_is_transitioning = false
	_is_typing = false
	_is_in_character_intro = false
	_pending_snapshot = {}
	if transition_overlay:
		transition_overlay.visible = false
		transition_overlay.modulate.a = 0.0
	if engine:
		engine.reset_session_flags()
		engine.load_script(DEFAULT_SCRIPT)
		_sync_engine_config_regs()
		_update_mail_btn_state(int(engine.get_mail_btn_state()))
	_seen_character_intros.clear()
	_is_terminated = false
	_is_waiting_for_wait_click = false
	_wait_click_seq += 1
	_is_waiting_for_choice = false
	_is_auto_active = false
	_is_skip_active = false
	_current_bgm_name = ""
	background.texture = null
	_clear_spr_overlay_instant()
	_reset_all_slots()
	_update_face_avatar(null, null, true)
	_show_title_screen()

func _on_title_start_new_game() -> void:
	_stop_all_voices(true)
	_is_mail_mode = false
	_stop_mail_on_bubble()
	_kill_mail_pulse_tween()
	if engine:
		engine.reset_session_flags()
		engine.load_script(DEFAULT_SCRIPT)
		_sync_engine_config_regs()
		_update_mail_btn_state(int(engine.get_mail_btn_state()))
	_seen_character_intros.clear()
	_is_terminated = false
	_is_waiting_for_wait_click = false
	_wait_click_seq += 1
	_is_waiting_for_choice = false
	_clear_spr_overlay_instant()
	_reset_all_slots()
	_update_face_avatar(null, null, true)
	_hide_title_screen_and_start()

func _on_title_continue_game() -> void:
	if SettingsPanelScript.has_any_save():
		_setup_settings_panel()
		settings_panel.open_panel(6, true)
	else:
		_show_notify("【暂无存档数据】")

func _on_title_quick_load() -> void:
	if SettingsPanelScript.has_quicksave():
		_on_quick_load()
	else:
		_show_notify("【暂无可用快捷存档】")

func _on_title_settings() -> void:
	_open_settings()

func _on_title_quit_game() -> void:
	get_tree().quit()

func _hide_title_screen_and_start() -> void:
	if not _is_in_title_screen:
		return
	_is_in_title_screen = false
	_update_pillarbox_visibility()
	if DisplayServer.get_name() == "headless":
		if title_screen:
			title_screen.visible = false
			title_screen.modulate.a = 0.0
		dialogue_box.visible = true
		advance_dialogue()
		return

	if _title_fade_tween and _title_fade_tween.is_valid():
		_title_fade_tween.kill()
	_title_fade_tween = create_tween()
	_title_fade_tween.tween_property(title_screen, "modulate:a", 0.0, 0.4)
	_title_fade_tween.finished.connect(func():
		if title_screen:
			title_screen.visible = false
		dialogue_box.visible = true
		advance_dialogue()
	)

func _cleanup_audio() -> void:
	_stop_all_voices(true)
	_focus_paused_players.clear()
	_focus_paused_tweens.clear()
	for p in [bgm_player, se_player, amb_player]:
		if p:
			if p.is_playing():
				p.stop()
			p.stream = null
	if char_intro_layer:
		for cp in [char_intro_layer.jingle_player, char_intro_layer.se_player]:
			if cp:
				if cp.is_playing():
					cp.stop()
				cp.stream = null

func _setup_focus_guard() -> void:
	if _focus_guard != null and is_instance_valid(_focus_guard):
		return
	var fg := FocusGuardNode.new()
	fg.name = "FocusGuard"
	fg.owner_main = self
	fg.process_mode = Node.PROCESS_MODE_ALWAYS
	add_child(fg)
	_focus_guard = fg

func _collect_active_tweens() -> Array[Tween]:
	var list: Array[Tween] = []
	var candidates: Array = [
		_scene_tween, _bg_pan_tween, _typing_tween, _duck_tween, _bgm_osd_tween,
		_face_tween, _notify_tween, _title_fade_tween, _spr_tween, _mail_pulse_tween,
		_voice_fade_tweens[0], _voice_fade_tweens[1],
	]
	for tw in _slot_tweens.values():
		candidates.append(tw)
	for tw in _slot_move_tweens.values():
		candidates.append(tw)
	if char_intro_layer and "_tween" in char_intro_layer:
		candidates.append(char_intro_layer._tween)
	for c in candidates:
		if c is Tween and c.is_valid() and c.is_running() and not (c in list):
			list.append(c)
	return list

func _on_app_focus_out() -> void:
	_reset_quick_menu_label_offsets()
	_reset_mail_btn_hover_state()
	_is_window_focused = false
	_wake_mouse_cursor()
	if _setting_bg_run or _is_focus_paused:
		return
	_is_focus_paused = true

	# 仅记录失焦瞬间真正在播的音轨，严禁回焦时误拉起已结束的音效/语音
	_focus_paused_players.clear()
	var audio_candidates: Array = [bgm_player, se_player, amb_player]
	for vp in _voice_players:
		audio_candidates.append(vp)
	if char_intro_layer:
		audio_candidates.append(char_intro_layer.jingle_player)
		audio_candidates.append(char_intro_layer.se_player)
	for p in audio_candidates:
		if p is AudioStreamPlayer and p.is_inside_tree() and p.is_playing() and not p.stream_paused:
			p.stream_paused = true
			_focus_paused_players.append(p)

	# 同步记录并暂停所有活跃 Tween
	_focus_paused_tweens.clear()
	for tw in _collect_active_tweens():
		tw.pause()
		_focus_paused_tweens.append(tw)

	if is_inside_tree() and get_tree():
		get_tree().paused = true

func _on_app_focus_in() -> void:
	_is_window_focused = true
	_mouse_idle_timer = 0.0
	if not _is_vfs_initialized and _byoa_guide_overlay != null and is_instance_valid(_byoa_guide_overlay):
		_try_init_vfs_and_boot()
	if not _is_focus_paused:
		return
	_is_focus_paused = false

	if is_inside_tree() and get_tree():
		get_tree().paused = false

	for tw in _focus_paused_tweens:
		if tw != null and tw.is_valid():
			tw.play()
	_focus_paused_tweens.clear()

	for p in _focus_paused_players:
		if p != null and is_instance_valid(p) and p.is_inside_tree() and p.stream != null and p.stream_paused:
			p.stream_paused = false
	_focus_paused_players.clear()

func _load_global_system_flags() -> void:
	if engine == null:
		return
	var abs_path := ProjectSettings.globalize_path(SYSTEM_FLAGS_PATH)
	if FileAccess.file_exists(abs_path):
		var txt := FileAccess.get_file_as_string(abs_path)
		if not txt.is_empty():
			engine.load_system_flags_json(txt)

func _flush_system_flags_if_dirty() -> void:
	if engine == null:
		return
	if engine.consume_system_flags_dirty():
		var json_str: String = engine.get_system_flags_json()
		var abs_path := ProjectSettings.globalize_path(SYSTEM_FLAGS_PATH)
		var f := FileAccess.open(abs_path, FileAccess.WRITE)
		if f:
			f.store_string(json_str)
			f.close()

func _exit_tree() -> void:
	_flush_system_flags_if_dirty()
	_stop_mail_on_bubble()
	_kill_mail_pulse_tween()
	if _mail_on_bubble != null and is_instance_valid(_mail_on_bubble):
		_mail_on_bubble.texture = null
	if _mail_btn != null and is_instance_valid(_mail_btn):
		_mail_btn.texture_normal = null
		_mail_btn.texture_hover = null
		_mail_btn.texture_pressed = null
		_mail_btn.texture_disabled = null
	_mail_on_frames.clear()
	_mail_btn_textures.clear()
	_cleanup_audio()

func _notification(what: int) -> void:
	if what == NOTIFICATION_WM_CLOSE_REQUEST:
		_cleanup_audio()
	elif what == NOTIFICATION_APPLICATION_FOCUS_OUT:
		_on_app_focus_out()
	elif what == NOTIFICATION_APPLICATION_FOCUS_IN:
		_on_app_focus_in()

# 播放原生 UI / 系统音效 (syssnd.arc: Cursor / Select_01 / Cancel / sound_test_*)
func _play_ui_se(sfx_name: String) -> void:
	if engine == null or se_player == null or not se_player.is_inside_tree():
		return
	var s = engine.load_audio(sfx_name)
	if s != null and s is AudioStream:
		se_player.volume_db = -80.0 if _setting_se_vol <= 0.001 else linear_to_db(_setting_se_vol)
		se_player.stream = s
		se_player.stream_paused = false
		se_player.play()

func _play_named_bgm(bgm_name: String) -> void:
	if engine == null or bgm_player == null or not bgm_player.is_inside_tree():
		return
	var bgm = engine.load_audio(bgm_name)
	if bgm != null and bgm is AudioStream:
		_current_bgm_name = bgm_name
		bgm_player.volume_db = _normal_bgm_volume_db
		bgm_player.stream = bgm
		bgm_player.play()

func _show_bgm_osd(bgm_name: String) -> void:
	if bgm_osd_label == null:
		return
	if _setting_bgm_osd_mode == 0 or bgm_name.is_empty():
		bgm_osd_label.visible = false
		return
	if DisplayServer.get_name() == "headless":
		return
	if _bgm_osd_tween and _bgm_osd_tween.is_valid():
		_bgm_osd_tween.kill()
	bgm_osd_label.text = "♪  %s" % bgm_name
	bgm_osd_label.position = Vector2(860.0, 18.0)
	bgm_osd_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	bgm_osd_label.visible = true
	bgm_osd_label.modulate.a = 1.0
	if _setting_bgm_osd_mode == 3:
		return
	var hold_sec := 2.0 if _setting_bgm_osd_mode == 1 else 4.0
	_bgm_osd_tween = create_tween()
	_bgm_osd_tween.tween_interval(hold_sec)
	_bgm_osd_tween.tween_property(bgm_osd_label, "modulate:a", 0.0, 0.5)
	_bgm_osd_tween.finished.connect(func():
		bgm_osd_label.visible = false
	)

func _on_test_sound(kind: String, asset: String) -> void:
	if engine == null:
		return
	var stream = engine.load_audio(asset)
	if stream == null or not (stream is AudioStream):
		return
	if kind == "se" and se_player and se_player.is_inside_tree():
		se_player.volume_db = -80.0 if _setting_se_vol <= 0.001 else linear_to_db(_setting_se_vol)
		se_player.stream = stream
		se_player.stream_paused = false
		se_player.play()
	elif kind == "amb" and amb_player and amb_player.is_inside_tree():
		amb_player.volume_db = -80.0 if _setting_amb_vol <= 0.001 else linear_to_db(_setting_amb_vol)
		amb_player.stream = stream
		amb_player.play()
	elif kind.begins_with("voice") and voice_player and voice_player.is_inside_tree():
		_play_voice_stream(stream, kind)

# 用户输入响应（鼠标点击、滚轮、触控与按键）
func _gui_input(event: InputEvent) -> void:
	_handle_input_event(event)

func _unhandled_input(event: InputEvent) -> void:
	_handle_input_event(event)

func _handle_input_event(event: InputEvent) -> void:
	# 过滤 Godot emulate_touch_from_mouse / emulate_mouse_from_touch 派生的伪事件，
	# 防止单次点击连发 MouseButton + ScreenTouch 导致角色登场特写或打字机被瞬间误跳！
	if event.device == InputEvent.DEVICE_ID_EMULATION:
		return

	if _is_waiting_for_choice:
		return

	var vp = get_viewport()

	# 新角色登场介绍演出期间：仅左键点击、非长按重复键或真实触控触发跳过介绍，其余输入一律拦截防止穿透到 advance_dialogue
	if _is_in_character_intro:
		var is_intentional_skip: bool = (
			(event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_LEFT)
			or (event is InputEventKey and event.pressed and not event.echo)
			or (event is InputEventScreenTouch and event.pressed)
		)
		if is_intentional_skip and Engine.get_process_frames() > _intro_start_frame:
			_skip_character_intro()
		if vp:
			vp.set_input_as_handled()
		return

	# 若系统设置面板已打开，优先处理 ESC / 右键关闭并拦截底层交互
	if settings_panel and settings_panel.visible:
		if event is InputEventMouseButton and event.pressed:
			if event.button_index == MOUSE_BUTTON_RIGHT:
				_close_settings()
				if vp:
					vp.set_input_as_handled()
		elif event is InputEventKey and event.pressed:
			if event.keycode == KEY_ESCAPE:
				_close_settings()
				if vp:
					vp.set_input_as_handled()
		return

	# 标题画面激活中，拦截背景空白点击，不触发正片剧情推进
	if _is_in_title_screen:
		return

	# 共同线彩蛋信封独立子引擎 (mail_vm) 活跃期间：放行推进翻页点击/确认键及手机翻页期滚轮向下，屏蔽主线所有系统快捷键与右键
	if _is_mail_mode:
		if event is InputEventMouseButton and event.pressed:
			if event.button_index == MOUSE_BUTTON_LEFT:
				advance_dialogue()
			elif event.button_index == MOUSE_BUTTON_WHEEL_DOWN and _is_waiting_for_wait_click and _setting_wheel_down_action == 0:
				advance_dialogue()
			if vp:
				vp.set_input_as_handled()
		elif event is InputEventKey and event.pressed and not event.echo:
			if event.keycode == KEY_SPACE or event.keycode == KEY_ENTER or event.keycode == KEY_KP_ENTER:
				advance_dialogue()
			if vp:
				vp.set_input_as_handled()
		elif event is InputEventScreenTouch and event.pressed:
			advance_dialogue()
			if vp:
				vp.set_input_as_handled()
		return

	# 转场期间点击或按键：触发安全熔断快进，立刻对齐至新场景
	if _is_transitioning:
		if (event is InputEventMouseButton and event.pressed) or (event is InputEventKey and event.pressed) or (event is InputEventScreenTouch and event.pressed):
			_skip_active_transition()
			if vp:
				vp.set_input_as_handled()
			return
	if backlog_panel and backlog_panel.visible:
		if event is InputEventMouseButton and event.pressed:
			if event.button_index == MOUSE_BUTTON_RIGHT:
				_close_backlog()
				if vp:
					vp.set_input_as_handled()
		elif event is InputEventKey and event.pressed:
			if event.keycode == KEY_ESCAPE:
				_close_backlog()
				if vp:
					vp.set_input_as_handled()
		return

	if event is InputEventMouseButton and event.pressed:
		if event.button_index == MOUSE_BUTTON_LEFT:
			if _is_auto_active and _setting_auto_stop_on_click:
				_is_auto_active = false
				_show_notify("【自动播放模式：停止】")
			if _is_skip_active:
				_is_skip_active = false
				_show_notify("【快进跳过模式：停止】")
			advance_dialogue()
			if vp:
				vp.set_input_as_handled()
		elif event.button_index == MOUSE_BUTTON_RIGHT:
			if _is_waiting_for_wait_click:
				# 手机纯翻页期忽略右键，避免污染主线“右键隐藏对话框”状态机
				if vp:
					vp.set_input_as_handled()
				return
			if _setting_right_click_action == 0:
				if not _is_ui_hidden:
					_hide_ui()
				else:
					_show_ui()
			elif _setting_right_click_action == 1:
				_open_settings()
			if vp:
				vp.set_input_as_handled()
		elif event.button_index == MOUSE_BUTTON_WHEEL_UP:
			if not _is_waiting_for_wait_click and _setting_wheel_up_action == 0 and not _is_ui_hidden:
				_open_backlog()
				if vp:
					vp.set_input_as_handled()
		elif event.button_index == MOUSE_BUTTON_WHEEL_DOWN:
			if _setting_wheel_down_action == 0 and not _is_ui_hidden:
				advance_dialogue()
				if vp:
					vp.set_input_as_handled()

	elif event is InputEventKey and event.pressed and not event.echo:
		var matched_sc := -1
		for si in range(_setting_shortcuts.size()):
			if _event_matches_shortcut(event, str(_setting_shortcuts[si])):
				matched_sc = si
				break
		if matched_sc >= 0:
			match matched_sc:
				0: _toggle_skip_mode()
				1:
					if not _is_waiting_for_wait_click:
						if _is_ui_hidden:
							_show_ui()
						else:
							_hide_ui()
				2:
					if not _is_waiting_for_wait_click:
						_open_backlog()
				3, 4: _toggle_auto_mode()
				5: _open_settings(0)
				6: _open_settings(4)
				7: _show_notify("【当前章节暂不支持场景跳过】")
				8:
					if _is_waiting_for_wait_click:
						_show_notify("【短信翻页期间暂不可保存】")
					else:
						_open_settings(5)
				9:
					if _is_waiting_for_wait_click:
						_show_notify("【短信翻页期间暂不可读档】")
					else:
						_open_settings(6)
				10:
					_setup_settings_panel()
					settings_panel.open_standalone_confirm("要返回主界面吗？", _return_to_title_screen, "SGDialog990001")
				11:
					_setup_settings_panel()
					settings_panel.open_standalone_confirm("要结束游戏吗？", func(): get_tree().quit(), "SGDialog990000")
			if vp:
				vp.set_input_as_handled()
		elif event.keycode == KEY_SPACE or event.keycode == KEY_ENTER or event.keycode == KEY_KP_ENTER:
			if _is_skip_active:
				_is_skip_active = false
			advance_dialogue()
			if vp:
				vp.set_input_as_handled()
		elif event.keycode == KEY_ESCAPE:
			if _is_waiting_for_wait_click:
				if vp:
					vp.set_input_as_handled()
				return
			if _is_ui_hidden:
				_show_ui()
			else:
				_hide_ui()
			if vp:
				vp.set_input_as_handled()
		elif event.keycode == KEY_C:
			_open_settings()

	elif event is InputEventScreenTouch and event.pressed:
		if _is_auto_active and _setting_auto_stop_on_click:
			_is_auto_active = false
		advance_dialogue()
		if vp:
			vp.set_input_as_handled()

# ==============================================================================
# PC 原版系统设置面板 (SettingsPanelController) 与 user://settings.cfg 持久化
# ==============================================================================
func _load_settings() -> void:
	_setup_settings_panel()
	if settings_panel:
		var cfg: Dictionary = settings_panel.load_from_disk()
		_apply_settings_from_dict(cfg)
	else:
		_apply_settings()

func _save_settings() -> void:
	if settings_panel:
		settings_panel.save_to_disk()

func _event_matches_shortcut(event: InputEventKey, bind_name: String) -> bool:
	var norm := bind_name.strip_edges().to_lower()
	if norm.is_empty():
		return false
	match norm:
		"ctrl", "control":
			return event.keycode == KEY_CTRL
		"space":
			return event.keycode == KEY_SPACE
		"tab":
			return event.keycode == KEY_TAB
		"enter", "return":
			return event.keycode == KEY_ENTER or event.keycode == KEY_KP_ENTER
		_:
			var kc := OS.find_keycode_from_string(bind_name.strip_edges())
			if kc != KEY_NONE:
				return event.keycode == kc
			return OS.get_keycode_string(event.keycode).to_lower() == norm

func _apply_settings_from_dict(cfg: Dictionary) -> void:
	_setting_bgm_vol = clampf(float(cfg.get("bgm_vol", 0.85)), 0.0, 1.0)
	_setting_se_vol = clampf(float(cfg.get("se_vol", 0.90)), 0.0, 1.0)
	_setting_amb_vol = clampf(float(cfg.get("amb_vol", 0.80)), 0.0, 1.0)
	_setting_voice_vol = clampf(float(cfg.get("voice_vol", 1.0)), 0.0, 1.0)
	var cv: Variant = cfg.get("char_vols", {})
	_setting_char_vols = cv.duplicate() if cv is Dictionary else {}
	_setting_text_speed = clampf(float(cfg.get("text_speed", 1.0)), 0.4, 3.0)
	_setting_auto_speed = clampf(float(cfg.get("auto_speed", 1.0)), 0.5, 2.5)
	_setting_resolution_idx = clampi(int(cfg.get("resolution_idx", 1)), 0, 1)
	_setting_screen_mode = clampi(int(cfg.get("screen_mode", 0)), 0, 2)
	_setting_full_ratio = clampi(int(cfg.get("full_ratio", 0)), 0, 1)
	_setting_fullscreen = (_setting_resolution_idx == 0)
	_setting_window_alpha = clampf(float(cfg.get("window_alpha", 0.88)), 0.25, 1.0)
	_setting_bgm_osd_mode = int(cfg.get("bgm_osd_mode", 1))
	_setting_effect_mode = int(cfg.get("effect_mode", 0))
	_setting_font_idx = int(cfg.get("font_idx", 0))
	_setting_auto_stop_on_click = bool(cfg.get("auto_stop_on_click", true))
	_setting_skip_continue_mode = int(cfg.get("skip_continue_mode", 0))
	_setting_skip_cond = int(cfg.get("skip_cond", 0))
	_setting_skip_stop_on_choice = bool(cfg.get("skip_stop_on_choice", true))
	_setting_hscene_mode = clampi(int(cfg.get("skip_hscene", 0)), 0, 2)
	_setting_voice_stop_on_next = bool(cfg.get("voice_stop_on_next", false))
	_setting_mouse_hide = clampi(int(cfg.get("mouse_hide", 2)), 0, 2)
	_setting_right_click_action = int(cfg.get("right_click_action", 0))
	_setting_wheel_up_action = int(cfg.get("wheel_up_action", 0))
	_setting_wheel_down_action = int(cfg.get("wheel_down_action", 0))
	_setting_bg_run = bool(cfg.get("bg_run", true))
	_setting_qsave_confirm = bool(cfg.get("qsave_confirm", false))
	_setting_qload_confirm = bool(cfg.get("qload_confirm", true))
	_setting_text_color = Color(str(cfg.get("text_color", "#ffffff")))
	var sc: Variant = cfg.get("shortcuts", [])
	if sc is Array and sc.size() == 12:
		_setting_shortcuts = sc.duplicate()
	_apply_settings()

func _sync_engine_config_regs() -> void:
	if engine != null:
		engine.set_hscene_mode(clampi(_setting_hscene_mode, 0, 2))

func _setup_pillarbox_overlay() -> void:
	if _pillarbox_layer != null and is_instance_valid(_pillarbox_layer):
		return
	_pillarbox_layer = Control.new()
	_pillarbox_layer.name = "Pillarbox43Overlay"
	_pillarbox_layer.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	# 中央 960x720 透传点击，仅左右 160px 黑边 ColorRect 拦截误触
	_pillarbox_layer.mouse_filter = Control.MOUSE_FILTER_IGNORE
	# 严格位于游戏舞台/对话框(0)之上，位于 Backlog(20)/登场特写(25)/标题(30)/设置与存读档弹窗(45)之下
	_pillarbox_layer.z_index = 15
	_pillarbox_layer.visible = false

	_pillarbox_left = ColorRect.new()
	_pillarbox_left.name = "LeftBar"
	_pillarbox_left.position = Vector2(0.0, 0.0)
	_pillarbox_left.size = Vector2(160.0, 720.0)
	_pillarbox_left.color = Color(0.0, 0.0, 0.0, 1.0)
	_pillarbox_left.mouse_filter = Control.MOUSE_FILTER_STOP
	_pillarbox_layer.add_child(_pillarbox_left)

	_pillarbox_right = ColorRect.new()
	_pillarbox_right.name = "RightBar"
	_pillarbox_right.position = Vector2(1120.0, 0.0)
	_pillarbox_right.size = Vector2(160.0, 720.0)
	_pillarbox_right.color = Color(0.0, 0.0, 0.0, 1.0)
	_pillarbox_right.mouse_filter = Control.MOUSE_FILTER_STOP
	_pillarbox_layer.add_child(_pillarbox_right)

	add_child(_pillarbox_layer)

func _update_pillarbox_visibility() -> void:
	if _pillarbox_layer == null:
		return
	var in_system_menu: bool = (
		_is_in_title_screen
		or (settings_panel != null and settings_panel.visible)
		or (backlog_panel != null and backlog_panel.visible)
	)
	_pillarbox_layer.visible = (_setting_screen_mode == 1) and not in_system_menu

func _apply_display_settings() -> void:
	# 1. 恒定维持 1280x720 基准逻辑分辨率与 CANVAS_ITEMS 模式
	var win := get_window()
	if win:
		win.content_scale_size = Vector2i(1280, 720)
		win.content_scale_mode = Window.CONTENT_SCALE_MODE_CANVAS_ITEMS
		# 官方 SGCnfgDsp000000 中 screen_mode: 0=全屏幕, 1=4:3, 2=无补正；
		# full_ratio (0=分割/1=正常) 依据 SGCnfgDsp500400 为全屏下方分割文本框选项，仅 screen_mode==2(无补正) 触发 ASPECT_IGNORE
		if _setting_screen_mode == 2:
			win.content_scale_aspect = Window.CONTENT_SCALE_ASPECT_IGNORE
		else:
			win.content_scale_aspect = Window.CONTENT_SCALE_ASPECT_KEEP

	# 2. 更新 4:3 左右黑边裁剪遮罩与右下角信封安全区联动
	_setup_pillarbox_overlay()
	_update_pillarbox_visibility()
	_layout_mail_hud()
	if _is_waiting_for_choice and _choice_box_right_shifted:
		_apply_choice_box_layout(true)

	# 3. 应用桌面全屏/窗口化切换
	if OS.get_name() != "iOS" and DisplayServer.get_name() != "headless":
		var target_mode = DisplayServer.WINDOW_MODE_FULLSCREEN if _setting_fullscreen else DisplayServer.WINDOW_MODE_WINDOWED
		if DisplayServer.window_get_mode() != target_mode:
			DisplayServer.window_set_mode(target_mode)
		if not _setting_fullscreen and DisplayServer.window_get_size() != Vector2i(1280, 720):
			DisplayServer.window_set_size(Vector2i(1280, 720))

func _apply_settings() -> void:
	_normal_bgm_volume_db = -80.0 if _setting_bgm_vol <= 0.001 else linear_to_db(_setting_bgm_vol)
	_ducked_bgm_volume_db = maxf(-80.0, _normal_bgm_volume_db - 6.0)
	if bgm_player:
		var is_voice_playing = _is_any_voice_playing()
		bgm_player.volume_db = _ducked_bgm_volume_db if is_voice_playing else _normal_bgm_volume_db

	var voice_db = -80.0 if _setting_voice_vol <= 0.001 else linear_to_db(_setting_voice_vol)
	var se_db = -80.0 if _setting_se_vol <= 0.001 else linear_to_db(_setting_se_vol)
	if voice_player and not (_voice_fade_tweens[_active_voice_idx] is Tween and _voice_fade_tweens[_active_voice_idx].is_valid()):
		voice_player.volume_db = _calc_voice_target_db(_current_voice_asset)
	if se_player:
		se_player.volume_db = se_db
	if amb_player:
		amb_player.volume_db = -80.0 if _setting_amb_vol <= 0.001 else linear_to_db(_setting_amb_vol)
	if intro_jingle_player:
		intro_jingle_player.volume_db = voice_db
	if char_intro_layer:
		char_intro_layer.set_audio_volumes(voice_db, se_db)

	if dialogue_box:
		dialogue_box.self_modulate.a = _setting_window_alpha
	if text_label:
		text_label.add_theme_color_override("default_color", _setting_text_color)
	_apply_dialogue_font()
	_sync_engine_config_regs()

	if _setting_mouse_hide == 2:
		_wake_mouse_cursor()

	if bgm_osd_label:
		if _setting_bgm_osd_mode == 0:
			bgm_osd_label.visible = false
		elif _setting_bgm_osd_mode == 3 and not _current_bgm_name.is_empty():
			_show_bgm_osd(_current_bgm_name)

	_apply_display_settings()

func _setup_settings_panel() -> void:
	if settings_panel != null:
		settings_panel.setup(engine, _custom_font)
		return

	settings_panel = SettingsPanelScript.new()
	add_child(settings_panel)
	settings_panel.setup(engine, _custom_font)
	settings_panel.settings_changed.connect(func(cfg: Dictionary):
		_apply_settings_from_dict(cfg)
	)
	settings_panel.save_to_slot_requested.connect(_perform_save_to_slot)
	settings_panel.load_from_slot_requested.connect(_perform_load_save_data)
	settings_panel.return_to_title_requested.connect(_return_to_title_screen)
	settings_panel.quit_game_requested.connect(func(): get_tree().quit())
	settings_panel.closed_requested.connect(_on_settings_panel_closed)
	settings_panel.visibility_changed.connect(_on_settings_visibility_changed)
	settings_panel.play_se_requested.connect(_play_ui_se)
	settings_panel.test_sound_requested.connect(_on_test_sound)

func _on_settings_visibility_changed() -> void:
	_update_pillarbox_visibility()
	_sync_mail_hud_visibility()
	if settings_panel and settings_panel.visible:
		_wake_mouse_cursor()

func _on_settings_panel_closed() -> void:
	_cached_scene_thumb = null
	_save_settings()
	_update_pillarbox_visibility()
	_sync_mail_hud_visibility()
	_wake_mouse_cursor()

func _sync_settings_ui() -> void:
	if settings_panel:
		settings_panel._sync_all_ui()

func _reset_default_settings() -> void:
	if settings_panel:
		settings_panel.reset_defaults()
		_apply_settings_from_dict(settings_panel.config)
	_show_notify("【已恢复默认设置】")

func _open_settings(tab_idx: int = 0) -> void:
	if _is_mail_mode:
		return
	if _is_waiting_for_wait_click and (tab_idx == 5 or tab_idx == 6):
		_show_notify("【短信翻页期间暂不可存读档】")
		return
	_reset_quick_menu_label_offsets()
	if _is_auto_active:
		_is_auto_active = false
	if _is_skip_active:
		_is_skip_active = false
	_wake_mouse_cursor()
	if DisplayServer.get_name() != "headless" and not _is_in_title_screen and get_viewport() and get_viewport().get_texture():
		_cached_scene_thumb = get_viewport().get_texture().get_image()
	_setup_settings_panel()
	settings_panel.open_panel(tab_idx, _is_in_title_screen)
	_update_pillarbox_visibility()
	_sync_mail_hud_visibility()

func _close_settings() -> void:
	_cached_scene_thumb = null
	if settings_panel:
		settings_panel.close_panel()
	else:
		_save_settings()
	_update_pillarbox_visibility()
	_sync_mail_hud_visibility()
	_wake_mouse_cursor()
