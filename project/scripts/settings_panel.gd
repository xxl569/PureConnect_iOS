extends Control
class_name SettingsPanelController

## 1:1 复刻 PC 原版全功能系统设置与 10×10 存读档窗口
## (SCREEN / TEXT / SOUND / SYSTEM / SHORTCUT / SAVE / LOAD + DEFAULT / BACK / TITLE / EXIT)

signal settings_changed(cfg: Dictionary)
signal save_to_slot_requested(slot_id: int)
signal load_from_slot_requested(save_data: Dictionary)
signal return_to_title_requested
signal quit_game_requested
signal closed_requested
signal play_se_requested(sfx_name: String)
signal test_sound_requested(sound_kind: String, asset_name: String)

const SETTINGS_PATH: String = "user://settings.cfg"
const SAVES_DIR: String = "user://saves"
const CHAR_CODES: PackedStringArray = ["so", "mo", "si", "ar", "ay", "sy", "ka", "se", "fi", "gi", "ki", "ke", "w", "m"]
const CHAR_LABELS: PackedStringArray = [
	"空", "萌美", "志帆", "彩里沙", "步海", "翔太", "奏",
	"老师", "菲", "吉尔", "奇", "庆太", "女配角", "男配角"
]

const TABS_COUNT: int = 7 # 0:Screen 1:Text 2:Sound 3:System 4:Shortcut 5:Save 6:Load

var _engine: Object = null
var _sys_font: Font = null
var _is_in_title: bool = false
var _textures_bound: bool = false

# 配置数据模型 (持久化至 user://settings.cfg)
var config: Dictionary = {}

# UI 节点容器
var _dim_bg: ColorRect
var _sys_bg_rect: TextureRect
var _header_hint_rect: TextureRect
var _tab_buttons: Array[TextureButton] = []
var _action_buttons: Dictionary = {} # "default", "back", "title", "exit"
var _tab_pages: Array[Control] = []
var _hint_bg_tex: TextureRect
var _footer_hint_lbl: Label

var _current_tab: int = 0
var _current_save_page: int = 0 # 0..9 -> 100 槽位
var _selected_char_idx: int = 1 # 默认萌美
var _active_color_target: int = 0 # 0=普通正文色, 1=已读正文色
var _selected_save_slot: int = -1

# 控件回调刷新表
var _sync_callbacks: Array[Callable] = []

# 动态子页关键元件引用
var _dsp_preview_box: Panel
var _dsp_preview_lbl: Label
var _txt_preview_box: Panel
var _txt_preview_lbl: Label
var _txt_read_preview_lbl: Label
var _txt_font_lbl: Label
var _txt_target_btns: Array[Button] = []
var _txt_target_lbls: Array[Label] = []
var _txt_target_swatches: Array[ColorRect] = []
var _txt_now_swatch: ColorRect
var _txt_rainbow_img: Image = null
var _snd_card_portrait: TextureRect
var _snd_char_slider_ctrl: Control
var _sys_rclick_lbl: Label
var _sys_wheelup_lbl: Label
var _sys_wheeldn_lbl: Label
var _key_pill_lbls: Array[Label] = []
var _waiting_key_bind_idx: int = -1

var _saveload_overlay_rect: TextureRect
var _save_slot_buttons: Array[Button] = []
var _save_slot_bgs: Array[TextureRect] = []
var _save_slot_thumbs: Array[TextureRect] = []
var _save_slot_headers: Array[Label] = []
var _save_slot_labels: Array[Label] = []
var _save_page_buttons: Array[TextureButton] = []
var _save_preview_tex: TextureRect
var _save_preview_header: Label
var _save_preview_info: Label

# 通用确认弹窗 (SGDialog9900xx + 000000/000100)
var _confirm_modal: Control
var _confirm_bg_rect: TextureRect
var _confirm_mask_rect: TextureRect
var _confirm_msg_lbl: Label
var _confirm_ok_cb: Callable
var _is_standalone_confirm: bool = false

# 保存截图与元数据缓存
var _saves_cache: Dictionary = {}

static func default_config() -> Dictionary:
	return {
		"cfg_version": 3,
		"resolution_idx": 1,        # 0:全屏 1:1280×720
		"screen_mode": 0,           # 0:全屏幕 1:4:3 2:无补正
		"full_ratio": 0,            # 0:分割 1:正常 (SGCnfgDsp000000)
		"window_alpha": 0.88,       # 0.25..1.0 对话框透明度
		"bgm_osd_mode": 1,          # 0:OFF 1:2秒 2:4秒 3:常時
		"effect_mode": 0,           # 0:高质量 1:简易 2:OFF
		"font_idx": 0,              # 0:思源黑体 1:微软雅黑 2:宋体
		"text_color": "#ffffff",
		"read_color": "#99d4ff",
		"text_speed": 1.0,          # 0.4..3.0 (3.0=瞬间)
		"auto_speed": 1.0,          # 0.5..2.5
		"auto_stop_on_click": true,
		"skip_continue_mode": 0,    # 0:一直持续 1:到选项为止 2:直到一天结束
		"skip_cond": 0,             # 0:仅已读 1:全部
		"skip_stop_on_choice": true,
		"skip_hscene": 0,           # 0:由选项决定(_SelectEx) 1:中出(LABEL_IN) 2:外射(LABEL_OUT)
		"voice_stop_on_next": false, # false:下一句语音时停止 true:单击鼠标时停止
		"bgm_vol": 0.85,
		"se_vol": 0.90,
		"amb_vol": 0.80,
		"voice_vol": 1.0,
		"char_vols": {
			"so": 1.0, "mo": 1.0, "si": 1.0, "ar": 1.0, "ay": 1.0,
			"sy": 1.0, "ka": 1.0, "se": 1.0, "fi": 1.0, "gi": 1.0,
			"ki": 1.0, "ke": 1.0, "w": 1.0, "m": 1.0,
		},
		"mouse_hide": 2,            # 0:5秒 1:10秒 2:永不消失
		"auto_move_cursor": false,
		"right_click_action": 0,    # 0:隐藏窗口 1:系统菜单 2:无动作
		"wheel_up_action": 0,       # 0:打开履历 1:无动作
		"wheel_down_action": 0,     # 0:下一句 1:无动作
		"bg_run": true,
		"qsave_confirm": false,
		"qload_confirm": true,
		"shortcuts": [
			"Ctrl", "H", "R", "A", "F", "O",
			"K", "Tab", "S", "L", "T", "E"
		],
	}

func _init() -> void:
	visible = false

func _ready() -> void:
	name = "SettingsPanel"
	set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	mouse_filter = Control.MOUSE_FILTER_STOP
	z_index = 45
	visible = false
	config = default_config()
	load_from_disk()
	_build_root_shell()

func setup(engine_ref: Object, font: Font) -> void:
	_engine = engine_ref
	_sys_font = font
	if config.is_empty():
		config = default_config()
		load_from_disk()
	_build_root_shell()
	_sync_all_ui()

func open_panel(tab_idx: int = 0, in_title: bool = false) -> void:
	_is_in_title = in_title
	_is_standalone_confirm = false
	_build_root_shell()
	_set_shell_chrome_visible(true)
	if _confirm_modal:
		_confirm_modal.visible = false
	# 若在标题页打开，禁用“存档”页签并避免回到 SAVE 页
	if _is_in_title and tab_idx == 5:
		tab_idx = 6
	switch_tab(tab_idx)
	visible = true
	move_to_front()
	_sync_all_ui()
	if _current_tab == 5 or _current_tab == 6:
		_refresh_save_grid()

func close_panel() -> void:
	if _is_standalone_confirm:
		dismiss_standalone_confirm()
		return
	save_to_disk()
	_waiting_key_bind_idx = -1
	visible = false
	closed_requested.emit()

func load_from_disk() -> Dictionary:
	config = default_config()
	var cfg_file := ConfigFile.new()
	if cfg_file.load(SETTINGS_PATH) == OK:
		var ver := int(cfg_file.get_value("main", "cfg_version", 1))
		for k in config.keys():
			if k == "cfg_version":
				continue
			if k == "char_vols":
				var cv: Dictionary = config["char_vols"].duplicate()
				for cc in CHAR_CODES:
					cv[cc] = clampf(float(cfg_file.get_value("char_voice", cc, cv[cc])), 0.0, 1.0)
				config["char_vols"] = cv
			elif k == "shortcuts":
				var loaded_sc = cfg_file.get_value("shortcuts", "keys", config["shortcuts"])
				var legacy_sc1 := [
					"Space/Enter", "Ctrl", "R", "H", "A", "Q.Save",
					"Q.Load", "S", "L", "C", "Esc", "F11"
				]
				var legacy_sc2 := [
					"Ctrl", "Space", "H", "A", "R", "O",
					"K", "Tab", "S", "L", "T", "E"
				]
				if loaded_sc is Array and loaded_sc.size() == 12 and loaded_sc != legacy_sc1 and (ver >= 2 or loaded_sc != legacy_sc2):
					config["shortcuts"] = loaded_sc
			else:
				config[k] = cfg_file.get_value("main", k, config[k])
		if ver < 3:
			if int(config.get("effect_mode", 0)) == 2:
				config["effect_mode"] = 0
			if int(config.get("mouse_hide", 2)) == 1:
				config["mouse_hide"] = 2
			if bool(config.get("voice_stop_on_next", false)) == true:
				config["voice_stop_on_next"] = false
			config["cfg_version"] = 3
			save_to_disk()
	var h_val := int(config.get("skip_hscene", 0))
	if h_val < 0 or h_val > 2:
		config["skip_hscene"] = 0
	return config

func save_to_disk() -> void:
	config["cfg_version"] = 3
	var cfg_file := ConfigFile.new()
	for k in config.keys():
		if k == "char_vols":
			var cv: Dictionary = config["char_vols"]
			for cc in CHAR_CODES:
				cfg_file.set_value("char_voice", cc, cv.get(cc, 1.0))
		elif k == "shortcuts":
			cfg_file.set_value("shortcuts", "keys", config["shortcuts"])
		else:
			cfg_file.set_value("main", k, config[k])
	cfg_file.save(SETTINGS_PATH)

func reset_defaults() -> void:
	config = default_config()
	save_to_disk()
	settings_changed.emit(config)
	_sync_all_ui()

func switch_tab(tab_idx: int) -> void:
	if _is_in_title and tab_idx == 5:
		return
	_current_tab = clampi(tab_idx, 0, TABS_COUNT - 1)
	_waiting_key_bind_idx = -1
	for i in range(_tab_pages.size()):
		if i < 5:
			_tab_pages[i].visible = (i == _current_tab)
		else:
			# Save(5) 与 Load(6) 共享 SGUsDtWnd 页容器
			_tab_pages[i].visible = (_current_tab == 5 or _current_tab == 6)

	for i in range(_tab_buttons.size()):
		if i == 5:
			_tab_buttons[i].disabled = _is_in_title
			_tab_buttons[i].modulate.a = 0.4 if _is_in_title else 1.0
		_tab_buttons[i].button_pressed = (i == _current_tab)

	if _header_hint_rect:
		var htex := _tex("SGSysWnd50%02d00" % _current_tab)
		_header_hint_rect.texture = htex
		_header_hint_rect.visible = (htex != null)
	if _action_buttons.has("default"):
		_action_buttons["default"].visible = (_current_tab < 5)

	var hints := [
		"画面设定：可调节画面模式、对话框透明度、背景音乐名提示及画面演出特效。",
		"文本设定：可调节字体、正文/已读配色、显示速度、自动模式及跳过方式。",
		"声音设定：可单独调节 BGM / 效果音 / 环境音 / 全部及各角色独立语音音量。",
		"系统设定：可配置鼠标消去时间、右键/滚轮行为、快存快读确认及后台处理。",
		"快捷键设定：点击任意胶囊按钮，按新键位即可重新绑定 (ESC 取消)。",
		"存档界面：共 10 页 100 槽位，点击槽位即可写入当前剧情进度与实时缩略图。",
		"读档界面：共 10 页 100 槽位，选择任意已有存档即可精确还原画面与进度。",
	]
	if _hint_bg_tex:
		_hint_bg_tex.visible = (_current_tab < 5)
	if _footer_hint_lbl:
		_footer_hint_lbl.visible = (_current_tab < 5)
		_footer_hint_lbl.text = hints[_current_tab]
	if _current_tab == 5 or _current_tab == 6:
		_refresh_save_grid()
	_sync_all_ui()

func _tex(asset_name: String) -> Texture2D:
	if _engine == null:
		return null
	var v: Variant = _engine.load_ui_texture(asset_name)
	return v if v is Texture2D else null

func _atlas(tex: Texture2D, rect: Rect2) -> AtlasTexture:
	if tex == null:
		return null
	var at := AtlasTexture.new()
	at.atlas = tex
	at.region = rect
	return at

func _add_bg_image(parent: Control, asset_name: String) -> TextureRect:
	var tr := TextureRect.new()
	tr.position = Vector2.ZERO
	tr.size = Vector2(1280, 720)
	tr.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	tr.stretch_mode = TextureRect.STRETCH_SCALE
	tr.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var t := _tex(asset_name)
	if t:
		tr.texture = t
	parent.add_child(tr)
	return tr

func _make_atlas_btn(
	parent: Control,
	rect: Rect2,
	n_tex: Texture2D,
	h_tex: Texture2D,
	p_tex: Texture2D,
	toggle: bool = false,
	fallback_txt: String = ""
) -> TextureButton:
	var btn := TextureButton.new()
	btn.position = rect.position
	btn.size = rect.size
	btn.ignore_texture_size = true
	btn.stretch_mode = TextureButton.STRETCH_SCALE
	btn.toggle_mode = toggle
	if n_tex:
		btn.texture_normal = _atlas(n_tex, rect)
	if h_tex:
		btn.texture_hover = _atlas(h_tex, rect)
	if p_tex:
		btn.texture_pressed = _atlas(p_tex, rect)
	elif h_tex:
		btn.texture_pressed = _atlas(h_tex, rect)

	if n_tex == null and fallback_txt != "":
		var lbl := Label.new()
		lbl.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		lbl.text = fallback_txt
		lbl.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
		lbl.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
		if _sys_font:
			lbl.add_theme_font_override("font", _sys_font)
		lbl.add_theme_font_size_override("font_size", 14)
		btn.add_child(lbl)

	btn.mouse_entered.connect(func():
		if not btn.disabled:
			play_se_requested.emit("Cursor")
	)
	parent.add_child(btn)
	return btn

func _make_slider(
	parent: Control,
	rail_x0: float,
	rail_x1: float,
	rail_y: float,
	getter: Callable,
	setter: Callable,
	knob_norm: Texture2D,
	knob_hov: Texture2D
) -> Control:
	var holder := Control.new()
	holder.position = Vector2(rail_x0 - 16.0, rail_y - 18.0)
	holder.size = Vector2((rail_x1 - rail_x0) + 32.0, 36.0)
	holder.mouse_filter = Control.MOUSE_FILTER_STOP
	parent.add_child(holder)

	var track_w := rail_x1 - rail_x0
	var fill := ColorRect.new()
	fill.position = Vector2(16.0, 15.0)
	fill.size = Vector2(0.0, 6.0)
	fill.color = Color(0.26, 0.68, 0.96, 0.72)
	fill.mouse_filter = Control.MOUSE_FILTER_IGNORE
	holder.add_child(fill)

	var knob := TextureRect.new()
	knob.size = Vector2(31, 31)
	knob.position = Vector2(0.0, 2.0)
	knob.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	knob.stretch_mode = TextureRect.STRETCH_SCALE
	knob.mouse_filter = Control.MOUSE_FILTER_IGNORE
	if knob_norm:
		knob.texture = knob_norm
	else:
		var kc := ColorRect.new()
		kc.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		kc.color = Color(0.35, 0.75, 1.0, 1.0)
		knob.add_child(kc)
	holder.add_child(knob)

	var update_visual := func(ratio: float):
		var cl := clampf(ratio, 0.0, 1.0)
		fill.size.x = cl * track_w
		knob.position.x = cl * track_w

	var dragging := {"active": false}
	var apply_x := func(local_x: float):
		var ratio := clampf((local_x - 16.0) / maxf(1.0, track_w), 0.0, 1.0)
		update_visual.call(ratio)
		setter.call(ratio)
		_sync_previews_only()

	holder.gui_input.connect(func(ev):
		if ev is InputEventMouseButton and ev.button_index == MOUSE_BUTTON_LEFT:
			dragging["active"] = ev.pressed
			if knob_hov and knob_norm:
				knob.texture = knob_hov if ev.pressed else knob_norm
			if ev.pressed:
				apply_x.call(ev.position.x)
		elif ev is InputEventMouseMotion and dragging["active"]:
			apply_x.call(ev.position.x)
		elif ev is InputEventScreenTouch:
			dragging["active"] = ev.pressed
			if ev.pressed:
				apply_x.call(ev.position.x)
		elif ev is InputEventScreenDrag:
			apply_x.call(ev.position.x)
	)

	_sync_callbacks.append(func():
		update_visual.call(float(getter.call()))
	)
	return holder

func _build_root_shell() -> void:
	if _dim_bg != null:
		if _engine != null and not _textures_bound:
			for ch in get_children():
				remove_child(ch)
				ch.free()
			_dim_bg = null
			_sys_bg_rect = null
			_header_hint_rect = null
			_hint_bg_tex = null
			_tab_buttons.clear()
			_action_buttons.clear()
			_tab_pages.clear()
			_sync_callbacks.clear()
			_txt_target_btns.clear()
			_txt_target_lbls.clear()
			_txt_target_swatches.clear()
			_txt_rainbow_img = null
			_key_pill_lbls.clear()
			_save_slot_buttons.clear()
			_save_slot_bgs.clear()
			_save_slot_thumbs.clear()
			_save_slot_headers.clear()
			_save_slot_labels.clear()
			_save_page_buttons.clear()
			_confirm_modal = null
		else:
			return
	_textures_bound = (_engine != null)

	_dim_bg = ColorRect.new()
	_dim_bg.name = "Backdrop"
	_dim_bg.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_dim_bg.color = Color(0.05, 0.12, 0.24, 0.65)
	_dim_bg.mouse_filter = Control.MOUSE_FILTER_STOP
	add_child(_dim_bg)

	# 1. SGSysWnd 底图皮肤 (全局背景壳子) + 右上角官方页头说明条 SGSysWnd500x00
	_sys_bg_rect = _add_bg_image(self, "SGSysWnd990000")
	_sys_bg_rect.name = "SysWndBase"

	_header_hint_rect = TextureRect.new()
	_header_hint_rect.name = "HeaderHintRect"
	_header_hint_rect.position = Vector2(488.0, 26.0)
	_header_hint_rect.size = Vector2(568.0, 34.0)
	_header_hint_rect.z_index = 2
	_header_hint_rect.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	_header_hint_rect.stretch_mode = TextureRect.STRETCH_SCALE
	_header_hint_rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var h0 := _tex("SGSysWnd500000")
	if h0:
		_header_hint_rect.texture = h0
	add_child(_header_hint_rect)

	# 2. 各分页容器 (0..4 独立页, 5=Save/Load 共享页容器)
	_tab_pages.clear()
	for i in range(6):
		var pg := Control.new()
		pg.name = "TabPage%d" % i
		pg.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		pg.mouse_filter = Control.MOUSE_FILTER_PASS
		pg.visible = false
		add_child(pg)
		_tab_pages.append(pg)
	# 第 7 个索引 (6: LOAD) 指向同一个 SaveLoad 页面容器
	_tab_pages.append(_tab_pages[5])

	# 3. 7 个右侧切换标签 + 4 个右下角功能按钮 (SGSysWnd000000/000001/000002)
	var sys_n := _tex("SGSysWnd000000")
	var sys_h := _tex("SGSysWnd000001")
	var sys_p := _tex("SGSysWnd000002")

	var tab_names := ["SCREEN", "TEXT", "SOUND", "SYSTEM", "SHORTCUT", "SAVE", "LOAD"]
	_tab_buttons.clear()
	for i in range(7):
		var rect := Rect2(1103.0, 25.0 + 60.0 * float(i), 177.0, 52.0)
		var btn := _make_atlas_btn(self, rect, sys_n, sys_h, sys_p, true, tab_names[i])
		btn.name = "TabBtn_%s" % tab_names[i]
		var t_idx := i
		btn.pressed.connect(func():
			play_se_requested.emit("Select_01")
			switch_tab(t_idx)
		)
		_tab_buttons.append(btn)

	var is_ios: bool = (OS.get_name() == "iOS")
	var act_specs := [
		{"key": "default", "rect": Rect2(1107.0, 495.0, 162.0, 42.0), "txt": "DEFAULT"},
		{"key": "back",    "rect": Rect2(1107.0, 565.0, 162.0, 42.0), "txt": "BACK"},
		{"key": "title",   "rect": Rect2(1107.0, 615.0, 162.0, 42.0), "txt": "TITLE"},
		{"key": "exit",    "rect": Rect2(1107.0, 665.0, 162.0, 42.0), "txt": "EXIT"},
	]
	for spec in act_specs:
		var ab := _make_atlas_btn(self, spec["rect"], sys_n, sys_h, sys_p, false, spec["txt"])
		ab.name = "ActionBtn_%s" % spec["key"]
		if is_ios and spec["key"] == "exit":
			ab.visible = false
		_action_buttons[spec["key"]] = ab

	_action_buttons["default"].pressed.connect(func():
		play_se_requested.emit("Select_01")
		_open_confirm("是否恢复所有设置项为默认值？", reset_defaults, "")
	)
	_action_buttons["back"].pressed.connect(func():
		play_se_requested.emit("Cancel")
		close_panel()
	)
	_action_buttons["title"].pressed.connect(func():
		play_se_requested.emit("Select_01")
		_open_confirm("要返回主界面吗？", func():
			close_panel()
			return_to_title_requested.emit()
		, "SGDialog990001")
	)
	_action_buttons["exit"].pressed.connect(func():
		play_se_requested.emit("Select_01")
		_open_confirm("要结束游戏吗？", func():
			quit_game_requested.emit()
		, "SGDialog990000")
	)

	# 4. 底部官方冰蓝 (i) 说明条 (SGSysWnd990200 @ 168, 636, 732x46，仅在 0..4 设置页显示)
	_hint_bg_tex = TextureRect.new()
	_hint_bg_tex.name = "FooterHintBar"
	_hint_bg_tex.position = Vector2(168.0, 636.0)
	_hint_bg_tex.size = Vector2(732.0, 46.0)
	_hint_bg_tex.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	_hint_bg_tex.stretch_mode = TextureRect.STRETCH_SCALE
	_hint_bg_tex.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var hb_tex := _tex("SGSysWnd990200")
	if hb_tex:
		_hint_bg_tex.texture = hb_tex
	else:
		var fb := Panel.new()
		fb.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		var hs := StyleBoxFlat.new()
		hs.bg_color = Color(0.55, 0.76, 1.0, 0.92)
		hs.set_corner_radius_all(12)
		fb.add_theme_stylebox_override("panel", hs)
		fb.mouse_filter = Control.MOUSE_FILTER_IGNORE
		_hint_bg_tex.add_child(fb)
	add_child(_hint_bg_tex)

	_footer_hint_lbl = Label.new()
	_footer_hint_lbl.position = Vector2(224.0, 636.0)
	_footer_hint_lbl.size = Vector2(662.0, 46.0)
	_footer_hint_lbl.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	if _sys_font:
		_footer_hint_lbl.add_theme_font_override("font", _sys_font)
	_footer_hint_lbl.add_theme_font_size_override("font_size", 15)
	_footer_hint_lbl.add_theme_color_override("font_color", Color.WHITE)
	_footer_hint_lbl.add_theme_color_override("font_outline_color", Color(0.14, 0.34, 0.68, 0.85))
	_footer_hint_lbl.add_theme_constant_override("outline_size", 2)
	add_child(_footer_hint_lbl)

	var knob_n := _tex("SGCnfgDsp020000")
	var knob_h := _tex("SGCnfgDsp020001")

	_build_tab_screen(_tab_pages[0], knob_n, knob_h)
	_build_tab_text(_tab_pages[1], knob_n, knob_h)
	_build_tab_sound(_tab_pages[2], knob_n, knob_h)
	_build_tab_system(_tab_pages[3])
	_build_tab_shortcut(_tab_pages[4])
	_build_tab_saveload(_tab_pages[5])
	_build_confirm_modal()

# ==============================================================================
# Tab 0: SCREEN (画面设定 SGCnfgDsp*)
# ==============================================================================
func _build_tab_screen(pg: Control, knob_n: Texture2D, knob_h: Texture2D) -> void:
	_add_bg_image(pg, "SGCnfgDsp990000")
	# 叠加原版左侧项目名称标签层 (画面尺寸 / 画面模式 / 屏幕比例 / 文本框透明度 / 背景音乐名称显示 / 画面效果)
	_add_bg_image(pg, "SGCnfgDsp000000")
	var tn := _tex("SGCnfgDsp000000")
	var th := _tex("SGCnfgDsp000001")
	var tp := _tex("SGCnfgDsp000002")

	# 预览框内置街景已在 SGCnfgDsp990000 [696, 128, 346, 194] 中，半透明对话框叠放在下半区 [696, 250, 346, 71]
	_dsp_preview_box = Panel.new()
	_dsp_preview_box.position = Vector2(696.0, 250.0)
	_dsp_preview_box.size = Vector2(346.0, 71.0)
	var ds := StyleBoxFlat.new()
	ds.bg_color = Color(0.10, 0.28, 0.56, float(config["window_alpha"]))
	ds.border_color = Color(0.75, 0.90, 1.0, 0.75)
	ds.set_border_width_all(1)
	ds.corner_radius_top_left = 4
	ds.corner_radius_top_right = 4
	_dsp_preview_box.add_theme_stylebox_override("panel", ds)
	pg.add_child(_dsp_preview_box)

	_dsp_preview_lbl = Label.new()
	_dsp_preview_lbl.position = Vector2(12.0, 10.0)
	_dsp_preview_lbl.size = Vector2(322.0, 52.0)
	_dsp_preview_lbl.text = "这里是对话框透明度的实时预览效果。"
	if _sys_font:
		_dsp_preview_lbl.add_theme_font_override("font", _sys_font)
	_dsp_preview_lbl.add_theme_font_size_override("font_size", 14)
	_dsp_preview_lbl.add_theme_color_override("font_color", Color.WHITE)
	_dsp_preview_lbl.add_theme_color_override("font_outline_color", Color(0.05, 0.14, 0.30, 0.9))
	_dsp_preview_lbl.add_theme_constant_override("outline_size", 2)
	_dsp_preview_lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_dsp_preview_box.add_child(_dsp_preview_lbl)

	# 选项组辅助注册
	_bind_option_group(pg, tn, th, tp, [
		Rect2(202, 126, 132, 34), Rect2(342, 126, 132, 34)
	], "resolution_idx", ["全屏", "1280×720"])

	_bind_option_group(pg, tn, th, tp, [
		Rect2(202, 206, 132, 34), Rect2(342, 206, 132, 34), Rect2(482, 206, 132, 34)
	], "screen_mode", ["全屏幕", "4:3", "无补正"])

	_bind_option_group(pg, tn, th, tp, [
		Rect2(202, 286, 132, 34), Rect2(342, 286, 132, 34)
	], "full_ratio", ["分割", "正常"])

	_make_slider(pg, 261.0, 418.0, 383.5,
		func(): return (float(config["window_alpha"]) - 0.25) / 0.75,
		func(r: float):
			config["window_alpha"] = 0.25 + r * 0.75
			settings_changed.emit(config),
		knob_n, knob_h
	)

	_bind_option_group(pg, tn, th, tp, [
		Rect2(202, 446, 92, 34), Rect2(306, 446, 92, 34),
		Rect2(410, 446, 92, 34), Rect2(514, 446, 92, 34)
	], "bgm_osd_mode", ["OFF", "2秒", "4秒", "常時"])

	_bind_option_group(pg, tn, th, tp, [
		Rect2(202, 526, 132, 34), Rect2(342, 526, 132, 34), Rect2(482, 526, 132, 34)
	], "effect_mode", ["高质量", "简易", "OFF"])

# ==============================================================================
# Tab 1: TEXT (文本设定 SGCnfgTxt*)
# ==============================================================================
func _build_tab_text(pg: Control, knob_n: Texture2D, knob_h: Texture2D) -> void:
	_add_bg_image(pg, "SGCnfgTxt990000")
	var txt_bg_tex := _tex("SGCnfgTxt990000")
	if txt_bg_tex:
		_txt_rainbow_img = txt_bg_tex.get_image()
	# 叠加原版左侧项目标签 (字体 / 文字颜色 / 已读文字颜色 / 显示速度 / 自动速度 / 点击左键 / 跳过持续 / H场景) 与右侧标签 + ▼ 按钮
	_add_bg_image(pg, "SGCnfgTxt000000")
	var tn := _tex("SGCnfgTxt000000")
	var th := _tex("SGCnfgTxt000001")
	var tp := _tex("SGCnfgTxt000002")

	# 字体切换胶囊 [203, 127, 260, 32] (文字避开右侧 x=432 的 ▼ 图标)
	var font_names := ["思源黑体 (NotoSansCJK)", "微软雅黑 (系统)", "宋体衬线 (Serif)"]
	var font_btn := Button.new()
	font_btn.position = Vector2(203.0, 127.0)
	font_btn.size = Vector2(260.0, 32.0)
	font_btn.flat = true
	pg.add_child(font_btn)

	_txt_font_lbl = Label.new()
	_txt_font_lbl.position = Vector2(214.0, 127.0)
	_txt_font_lbl.size = Vector2(212.0, 32.0)
	_txt_font_lbl.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	if _sys_font:
		_txt_font_lbl.add_theme_font_override("font", _sys_font)
	_txt_font_lbl.add_theme_font_size_override("font_size", 14)
	_txt_font_lbl.add_theme_color_override("font_color", Color(0.12, 0.22, 0.38, 1.0))
	pg.add_child(_txt_font_lbl)

	font_btn.pressed.connect(func():
		play_se_requested.emit("Select_01")
		config["font_idx"] = (int(config["font_idx"]) + 1) % font_names.size()
		_txt_font_lbl.text = font_names[int(config["font_idx"])]
		settings_changed.emit(config)
	)
	_sync_callbacks.append(func():
		_txt_font_lbl.text = font_names[clampi(int(config["font_idx"]), 0, font_names.size() - 1)]
	)

	# 颜色切换目标胶囊 (普通正文色 [202, 186, 262, 34] / 已读文本色 [202, 246, 262, 34])
	# 采用官方 SGCnfgTxt900400 冰蓝胶囊 + 清晰白字深蓝描边 + 右侧实色预览小方块
	var pill_tex := _tex("SGCnfgTxt900400")
	_txt_target_btns.clear()
	_txt_target_lbls.clear()
	_txt_target_swatches.clear()
	for ti in range(2):
		var tb := Button.new()
		tb.position = Vector2(202.0, 186.0 + 60.0 * float(ti))
		tb.size = Vector2(262.0, 34.0)
		tb.flat = (pill_tex != null)
		var pill_style := StyleBoxFlat.new()
		pill_style.bg_color = Color(0.61, 0.78, 1.0, 0.0 if pill_tex != null else 1.0)
		pill_style.border_color = Color(0.15, 0.42, 0.88, 1.0) if ti == _active_color_target else Color(0.45, 0.68, 0.96, 0.6)
		pill_style.set_border_width_all(2 if ti == _active_color_target else 0)
		pill_style.set_corner_radius_all(17)
		if pill_tex:
			var bg_p := TextureRect.new()
			bg_p.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
			bg_p.texture = pill_tex
			bg_p.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
			bg_p.stretch_mode = TextureRect.STRETCH_SCALE
			bg_p.mouse_filter = Control.MOUSE_FILTER_IGNORE
			tb.add_child(bg_p)
		tb.add_theme_stylebox_override("normal", pill_style)
		tb.add_theme_stylebox_override("hover", pill_style)
		tb.add_theme_stylebox_override("pressed", pill_style)
		tb.add_theme_stylebox_override("focus", pill_style)

		var ring := Panel.new()
		ring.name = "ActiveRing"
		ring.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		ring.mouse_filter = Control.MOUSE_FILTER_IGNORE
		var ring_sb := StyleBoxFlat.new()
		ring_sb.draw_center = false
		ring_sb.border_color = Color(0.12, 0.36, 0.86, 1.0)
		ring_sb.set_border_width_all(2)
		ring_sb.set_corner_radius_all(17)
		ring.add_theme_stylebox_override("panel", ring_sb)
		ring.visible = (ti == _active_color_target)
		tb.add_child(ring)

		var tlbl := Label.new()
		tlbl.position = Vector2(14.0, 0.0)
		tlbl.size = Vector2(204.0, 34.0)
		tlbl.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
		tlbl.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
		tlbl.text = "普通文本颜色（调色盘设色）" if ti == 0 else "已读文本颜色（点击切换）"
		tlbl.mouse_filter = Control.MOUSE_FILTER_IGNORE
		if _sys_font:
			tlbl.add_theme_font_override("font", _sys_font)
		tlbl.add_theme_font_size_override("font_size", 13)
		tlbl.add_theme_color_override("font_color", Color.WHITE)
		tlbl.add_theme_color_override("font_outline_color", Color(0.10, 0.25, 0.52, 0.95))
		tlbl.add_theme_constant_override("outline_size", 3)
		tb.add_child(tlbl)

		var sw_frame := ColorRect.new()
		sw_frame.position = Vector2(223.0, 7.0)
		sw_frame.size = Vector2(26.0, 20.0)
		sw_frame.color = Color(0.18, 0.34, 0.62, 0.9)
		sw_frame.mouse_filter = Control.MOUSE_FILTER_IGNORE
		tb.add_child(sw_frame)

		var sw := ColorRect.new()
		sw.position = Vector2(225.0, 9.0)
		sw.size = Vector2(22.0, 16.0)
		sw.color = Color.WHITE
		sw.mouse_filter = Control.MOUSE_FILTER_IGNORE
		tb.add_child(sw)

		var captured := ti
		tb.pressed.connect(func():
			play_se_requested.emit("Select_01")
			_active_color_target = captured
			_sync_previews_only()
		)
		pg.add_child(tb)
		_txt_target_btns.append(tb)
		_txt_target_lbls.append(tlbl)
		_txt_target_swatches.append(sw)

	# 右侧 NOW 当前色块严格落在 SGCnfgTxt990000 白色色框 [794..819, 438..457] 内，不遮挡 "NOW" 文字
	_txt_now_swatch = ColorRect.new()
	_txt_now_swatch.position = Vector2(796.0, 440.0)
	_txt_now_swatch.size = Vector2(22.0, 16.0)
	_txt_now_swatch.color = Color.WHITE
	pg.add_child(_txt_now_swatch)

	# 6 枚右侧预设颜色按钮 strictly 对齐底图 6 个方框 [840, 874, 908, 942, 980, 1014]
	var preset_hex := ["#88d8ff", "#ffdf80", "#ff9ec4", "#9effb8", "#ffffff", "#111111"]
	var swatch_x := [840.0, 874.0, 908.0, 942.0, 980.0, 1014.0]
	for si in range(preset_hex.size()):
		var sb := Button.new()
		sb.position = Vector2(swatch_x[si], 438.0)
		sb.size = Vector2(26.0, 18.0)
		sb.flat = true
		var col := Color(preset_hex[si])
		var sw_fill := ColorRect.new()
		sw_fill.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		sw_fill.color = col
		sw_fill.mouse_filter = Control.MOUSE_FILTER_IGNORE
		sb.add_child(sw_fill)
		var hex_val: String = str(preset_hex[si])
		sb.pressed.connect(func():
			play_se_requested.emit("Select_01")
			if _active_color_target == 0:
				config["text_color"] = hex_val
			else:
				config["read_color"] = hex_val
			settings_changed.emit(config)
			_sync_previews_only()
		)
		pg.add_child(sb)

	# 彩虹渐变拾色区域精确对齐底图 [696, 332, 346, 95]，直接从原图采样并支持按下拖拽实时取色
	var rainbow := Control.new()
	rainbow.name = "RainbowPalette"
	rainbow.position = Vector2(696.0, 332.0)
	rainbow.size = Vector2(346.0, 95.0)
	rainbow.mouse_filter = Control.MOUSE_FILTER_STOP
	var rb_drag := {"active": false}
	var pick_rainbow := func(local_pos: Vector2):
		var px_f := clampf(local_pos.x, 0.0, 345.0)
		var py_f := clampf(local_pos.y, 0.0, 94.0)
		var picked: Color
		if _txt_rainbow_img and not _txt_rainbow_img.is_empty():
			var ix := clampi(int(696.0 + px_f), 696, mini(1041, _txt_rainbow_img.get_width() - 1))
			var iy := clampi(int(332.0 + py_f), 332, mini(426, _txt_rainbow_img.get_height() - 1))
			picked = _txt_rainbow_img.get_pixel(ix, iy)
		else:
			picked = Color.from_hsv(px_f / 345.0, 0.85 - (py_f / 94.0) * 0.65, 1.0 - (py_f / 94.0) * 0.35)
		var hex_s := "#" + picked.to_html(false)
		if _active_color_target == 0:
			config["text_color"] = hex_s
		else:
			config["read_color"] = hex_s
		settings_changed.emit(config)
		_sync_previews_only()

	rainbow.gui_input.connect(func(ev):
		if ev is InputEventMouseButton and ev.button_index == MOUSE_BUTTON_LEFT:
			rb_drag["active"] = ev.pressed
			if ev.pressed:
				pick_rainbow.call(ev.position)
		elif ev is InputEventMouseMotion and (rb_drag["active"] or Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT)):
			pick_rainbow.call(ev.position)
		elif ev is InputEventScreenTouch:
			rb_drag["active"] = ev.pressed
			if ev.pressed:
				pick_rainbow.call(ev.position)
		elif ev is InputEventScreenDrag:
			pick_rainbow.call(ev.position)
	)
	pg.add_child(rainbow)

	# 文本预览窗叠放在内置街景下方半区 [696, 206, 346, 89]，上方天空街景完整保留
	_txt_preview_box = Panel.new()
	_txt_preview_box.position = Vector2(696.0, 206.0)
	_txt_preview_box.size = Vector2(346.0, 89.0)
	var ts := StyleBoxFlat.new()
	ts.bg_color = Color(0.08, 0.22, 0.44, 0.78)
	ts.border_color = Color(0.75, 0.90, 1.0, 0.75)
	ts.set_border_width_all(1)
	ts.corner_radius_top_left = 4
	ts.corner_radius_top_right = 4
	_txt_preview_box.add_theme_stylebox_override("panel", ts)
	pg.add_child(_txt_preview_box)

	_txt_preview_lbl = Label.new()
	_txt_preview_lbl.position = Vector2(12.0, 8.0)
	_txt_preview_lbl.size = Vector2(322.0, 36.0)
	_txt_preview_lbl.text = "普通文本示例：欢迎来到 PURE×CONNECT！"
	_txt_preview_lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	if _sys_font:
		_txt_preview_lbl.add_theme_font_override("font", _sys_font)
	_txt_preview_lbl.add_theme_font_size_override("font_size", 14)
	_txt_preview_lbl.add_theme_color_override("font_outline_color", Color(0.04, 0.10, 0.22, 0.9))
	_txt_preview_lbl.add_theme_constant_override("outline_size", 2)
	_txt_preview_box.add_child(_txt_preview_lbl)

	_txt_read_preview_lbl = Label.new()
	_txt_read_preview_lbl.position = Vector2(12.0, 46.0)
	_txt_read_preview_lbl.size = Vector2(322.0, 36.0)
	_txt_read_preview_lbl.text = "已读文本示例：这段对白已经阅读过。"
	_txt_read_preview_lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	if _sys_font:
		_txt_read_preview_lbl.add_theme_font_override("font", _sys_font)
	_txt_read_preview_lbl.add_theme_font_size_override("font_size", 14)
	_txt_read_preview_lbl.add_theme_color_override("font_outline_color", Color(0.04, 0.10, 0.22, 0.9))
	_txt_read_preview_lbl.add_theme_constant_override("outline_size", 2)
	_txt_preview_box.add_child(_txt_read_preview_lbl)

	# 文本速度 & 自动速度滑块 (对齐白色轨道 x=261..418, y=323/383)
	_make_slider(pg, 261.0, 418.0, 323.5,
		func(): return (float(config["text_speed"]) - 0.4) / 2.6,
		func(r: float):
			config["text_speed"] = 0.4 + r * 2.6
			settings_changed.emit(config),
		knob_n, knob_h
	)
	_make_slider(pg, 261.0, 418.0, 383.5,
		func(): return (float(config["auto_speed"]) - 0.5) / 2.0,
		func(r: float):
			config["auto_speed"] = 0.5 + r * 2.0
			settings_changed.emit(config),
		knob_n, knob_h
	)

	_bind_bool_pair(pg, tn, th, tp, Rect2(202, 426, 132, 34), Rect2(342, 426, 132, 34), "auto_stop_on_click")
	_bind_option_group(pg, tn, th, tp, [
		Rect2(202, 486, 132, 34), Rect2(342, 486, 132, 34), Rect2(482, 486, 132, 34)
	], "skip_continue_mode", ["一直持续", "到选项", "到天末"])
	_bind_option_group(pg, tn, th, tp, [
		Rect2(788, 486, 132, 34), Rect2(928, 486, 132, 34)
	], "skip_cond", ["仅已读", "全部"])
	# 官方 SGCnfgTxt000000 按钮顺序为 [中出, 外射, 由选项决定]，对应 BGI 0xE3(20) 值 [1, 2, 0]
	_bind_option_group(pg, tn, th, tp, [
		Rect2(202, 546, 132, 34), Rect2(342, 546, 132, 34), Rect2(482, 546, 132, 34)
	], "skip_hscene", ["中出", "外射", "由选项决定"], [1, 2, 0])
	_bind_bool_pair(pg, tn, th, tp, Rect2(788, 546, 132, 34), Rect2(928, 546, 132, 34), "voice_stop_on_next")

# ==============================================================================
# Tab 2: SOUND (声音设定 SGCnfgSnd*)
# ==============================================================================
func _build_tab_sound(pg: Control, knob_n: Texture2D, knob_h: Texture2D) -> void:
	_add_bg_image(pg, "SGCnfgSnd990000")
	# 叠加原版 BGM / 效果音 / 环境音 / 全部语音 / 个别语音 左侧标题层
	_add_bg_image(pg, "SGCnfgSnd000000")
	var sn := _tex("SGCnfgSnd000000")
	var sh := _tex("SGCnfgSnd000001")
	var sp := _tex("SGCnfgSnd000002")
	var test_n := _tex("SGCnfgSnd000100")
	var test_h := _tex("SGCnfgSnd000101")

	# 4 路全局音量滑块 (严格对齐白色轨道边界，滑块旋钮绝不遮挡 MIN / MAX 文字)
	_make_slider(pg, 260.0, 417.0, 143.5,
		func(): return float(config["bgm_vol"]),
		func(r: float):
			config["bgm_vol"] = r
			settings_changed.emit(config),
		knob_n, knob_h
	)
	_make_slider(pg, 772.0, 929.0, 143.5,
		func(): return float(config["se_vol"]),
		func(r: float):
			config["se_vol"] = r
			settings_changed.emit(config),
		knob_n, knob_h
	)
	_make_slider(pg, 260.0, 417.0, 223.5,
		func(): return float(config["amb_vol"]),
		func(r: float):
			config["amb_vol"] = r
			settings_changed.emit(config),
		knob_n, knob_h
	)
	_make_slider(pg, 770.0, 927.0, 223.5,
		func(): return float(config["voice_vol"]),
		func(r: float):
			config["voice_vol"] = r
			settings_changed.emit(config),
		knob_n, knob_h
	)

	# 3 个全局 TEST 试听按钮
	var make_test_btn := func(pos: Vector2, kind: String, asset: String):
		var tb := TextureButton.new()
		tb.position = pos
		tb.size = Vector2(62, 34)
		tb.ignore_texture_size = true
		tb.stretch_mode = TextureButton.STRETCH_SCALE
		if test_n:
			tb.texture_normal = test_n
		if test_h:
			tb.texture_hover = test_h
			tb.texture_pressed = test_h
		tb.pressed.connect(func():
			test_sound_requested.emit(kind, asset)
		)
		pg.add_child(tb)
		return tb

	make_test_btn.call(Vector2(992.0, 126.0), "se", "sound_test_se")
	make_test_btn.call(Vector2(482.0, 206.0), "amb", "sound_test_sound")
	make_test_btn.call(Vector2(992.0, 206.0), "voice_all", "sound_test_all")

	# 左下角独立角色语音卡片 (50, 334, 474, 252)
	var card_frame := TextureRect.new()
	card_frame.position = Vector2(50.0, 334.0)
	card_frame.size = Vector2(474.0, 252.0)
	card_frame.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	card_frame.stretch_mode = TextureRect.STRETCH_SCALE
	card_frame.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var cf_tex := _tex("SGCnfgSnd990100")
	if cf_tex:
		card_frame.texture = cf_tex
	pg.add_child(card_frame)

	_snd_card_portrait = TextureRect.new()
	_snd_card_portrait.position = Vector2(50.0, 334.0)
	_snd_card_portrait.size = Vector2(474.0, 252.0)
	_snd_card_portrait.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	_snd_card_portrait.stretch_mode = TextureRect.STRETCH_SCALE
	_snd_card_portrait.mouse_filter = Control.MOUSE_FILTER_IGNORE
	pg.add_child(_snd_card_portrait)

	var rail_tr := TextureRect.new()
	rail_tr.position = Vector2(212.0, 516.0)
	rail_tr.size = Vector2(272.0, 34.0)
	rail_tr.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	rail_tr.stretch_mode = TextureRect.STRETCH_SCALE
	rail_tr.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var rail_tex := _tex("SGCnfgSnd900200")
	if rail_tex:
		rail_tr.texture = rail_tex
	pg.add_child(rail_tr)

	_snd_char_slider_ctrl = _make_slider(pg, 268.0, 425.0, 533.0,
		func():
			var code: String = CHAR_CODES[_selected_char_idx]
			return float(config["char_vols"].get(code, 1.0)),
		func(r: float):
			var code: String = CHAR_CODES[_selected_char_idx]
			config["char_vols"][code] = r
			settings_changed.emit(config),
		knob_n, knob_h
	)

	var card_test_btn: TextureButton = make_test_btn.call(Vector2(422.0, 476.0), "voice_char", "sound_test_mo")
	# 动态绑定当前选中的角色语音试听资产
	for c in card_test_btn.pressed.get_connections():
		card_test_btn.pressed.disconnect(c["callable"])
	card_test_btn.pressed.connect(func():
		var code: String = CHAR_CODES[_selected_char_idx]
		test_sound_requested.emit("voice_" + code, "sound_test_" + code)
	)

	# 右下角 14 位角色选择按钮矩阵 [560/720/880, 330..530, 152, 34]
	var char_btns: Array[TextureButton] = []
	for ci in range(CHAR_CODES.size()):
		var col := ci % 3
		var row := ci / 3
		var rect := Rect2(560.0 + 160.0 * float(col), 330.0 + 50.0 * float(row), 152.0, 34.0)
		var cb := _make_atlas_btn(pg, rect, sn, sh, sp, true, CHAR_LABELS[ci])
		var idx := ci
		cb.pressed.connect(func():
			play_se_requested.emit("Select_01")
			_selected_char_idx = idx
			_refresh_sound_card()
			for b_i in range(char_btns.size()):
				char_btns[b_i].set_pressed_no_signal(b_i == _selected_char_idx)
		)
		char_btns.append(cb)

	_sync_callbacks.append(func():
		_refresh_sound_card()
		for b_i in range(char_btns.size()):
			char_btns[b_i].set_pressed_no_signal(b_i == _selected_char_idx)
	)

func _refresh_sound_card() -> void:
	if _snd_card_portrait:
		var t := _tex("SGCnfgSnd01%02d02" % _selected_char_idx)
		_snd_card_portrait.texture = t

# ==============================================================================
# Tab 3: SYSTEM (系统设定 SGCnfgSys*)
# ==============================================================================
func _build_tab_system(pg: Control) -> void:
	_add_bg_image(pg, "SGCnfgSys990000")
	# 叠加原版左侧项目标签 (鼠标消去时间 / 鼠标自动移动 / 鼠标右键 / 滚轮↑ / 滚轮↓ / 后台处理 / 快存确认 / 快读确认)
	_add_bg_image(pg, "SGCnfgSys000000")
	var sn := _tex("SGCnfgSys000000")
	var sh := _tex("SGCnfgSys000001")
	var sp := _tex("SGCnfgSys000002")

	_bind_option_group(pg, sn, sh, sp, [
		Rect2(202, 126, 132, 34), Rect2(342, 126, 132, 34), Rect2(482, 126, 132, 34)
	], "mouse_hide", ["5秒", "10秒", "永不消失"])

	_bind_bool_pair(pg, sn, sh, sp, Rect2(202, 186, 132, 34), Rect2(342, 186, 132, 34), "auto_move_cursor")

	# 右键 / 滚轮上 / 滚轮下 动作循环胶囊：复用官方 132x34 冰蓝胶囊 SGCnfgKey020100/020200，精准覆盖底图占位胶囊
	var pill_n := _tex("SGCnfgKey020100")
	var pill_h := _tex("SGCnfgKey020200")
	var pill_fb := _tex("SGCnfgKey020000")
	var make_cycle_pill := func(y: float, cfg_key: String, choices: PackedStringArray) -> Label:
		var btn := TextureButton.new()
		btn.position = Vector2(202.0, y)
		btn.size = Vector2(132.0, 34.0)
		btn.ignore_texture_size = true
		btn.stretch_mode = TextureButton.STRETCH_SCALE
		if pill_n:
			btn.texture_normal = pill_n
		elif pill_fb:
			btn.texture_normal = pill_fb
		if pill_h:
			btn.texture_hover = pill_h
			btn.texture_pressed = pill_h
		pg.add_child(btn)
		var lbl := Label.new()
		lbl.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		lbl.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
		lbl.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
		lbl.mouse_filter = Control.MOUSE_FILTER_IGNORE
		if _sys_font:
			lbl.add_theme_font_override("font", _sys_font)
		lbl.add_theme_font_size_override("font_size", 14)
		lbl.add_theme_color_override("font_color", Color(0.10, 0.22, 0.42, 1.0))
		btn.add_child(lbl)
		btn.pressed.connect(func():
			play_se_requested.emit("Select_01")
			config[cfg_key] = (int(config[cfg_key]) + 1) % choices.size()
			lbl.text = choices[int(config[cfg_key])]
			settings_changed.emit(config)
		)
		_sync_callbacks.append(func():
			lbl.text = choices[clampi(int(config[cfg_key]), 0, choices.size() - 1)]
		)
		return lbl

	_sys_rclick_lbl = make_cycle_pill.call(246.0, "right_click_action", ["隐藏窗口", "系统菜单", "无动作"])
	_sys_wheelup_lbl = make_cycle_pill.call(306.0, "wheel_up_action", ["打开履历", "无动作"])
	_sys_wheeldn_lbl = make_cycle_pill.call(366.0, "wheel_down_action", ["下一句", "无动作"])

	_bind_bool_pair(pg, sn, sh, sp, Rect2(202, 426, 132, 34), Rect2(342, 426, 132, 34), "bg_run")
	_bind_bool_pair(pg, sn, sh, sp, Rect2(202, 486, 132, 34), Rect2(342, 486, 132, 34), "qsave_confirm")
	_bind_bool_pair(pg, sn, sh, sp, Rect2(202, 546, 132, 34), Rect2(342, 546, 132, 34), "qload_confirm")

# ==============================================================================
# Tab 4: SHORTCUT (快捷键设定 SGCnfgKey*)
# ==============================================================================
func _build_tab_shortcut(pg: Control) -> void:
	_add_bg_image(pg, "SGCnfgKey990000")
	# 叠加原版左右双栏 12 项快捷键名称标签层 (文本跳过 / 隐藏文本框 / 履历 / 自动模式 / 设置 / 快捷键 / 场景跳过 / 保存 / 读取 / 标题 / 关闭游戏)
	_add_bg_image(pg, "SGCnfgKey000000")
	var kn := _tex("SGCnfgKey020100")
	if kn == null:
		kn = _tex("SGCnfgKey020000")
	var kh := _tex("SGCnfgKey020200")
	if kh == null:
		kh = kn
	var kp := kh
	_key_pill_lbls.clear()

	for i in range(12):
		var col := 0 if i < 6 else 1
		var row := i % 6
		var pos := Vector2(202.0 + 514.0 * float(col), 126.0 + 80.0 * float(row))
		var btn := TextureButton.new()
		btn.position = pos
		btn.size = Vector2(132.0, 34.0)
		btn.ignore_texture_size = true
		btn.stretch_mode = TextureButton.STRETCH_SCALE
		if kn:
			btn.texture_normal = kn
		if kh:
			btn.texture_hover = kh
		if kp:
			btn.texture_pressed = kp
		pg.add_child(btn)

		var lbl := Label.new()
		lbl.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		lbl.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
		lbl.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
		if _sys_font:
			lbl.add_theme_font_override("font", _sys_font)
		lbl.add_theme_font_size_override("font_size", 14)
		lbl.add_theme_color_override("font_color", Color.WHITE)
		lbl.add_theme_color_override("font_outline_color", Color(0.14, 0.36, 0.72, 0.95))
		lbl.add_theme_constant_override("outline_size", 3)
		btn.add_child(lbl)
		_key_pill_lbls.append(lbl)

		var idx := i
		btn.pressed.connect(func():
			play_se_requested.emit("Select_01")
			_waiting_key_bind_idx = idx
			_refresh_shortcut_labels()
		)

	_sync_callbacks.append(func():
		_refresh_shortcut_labels()
	)

func _refresh_shortcut_labels() -> void:
	var sc: Array = config.get("shortcuts", [])
	for i in range(_key_pill_lbls.size()):
		if _waiting_key_bind_idx == i:
			_key_pill_lbls[i].text = "[请按按键...]"
		elif i < sc.size():
			_key_pill_lbls[i].text = str(sc[i])

func _input(event: InputEvent) -> void:
	if not visible or _current_tab != 4 or _waiting_key_bind_idx < 0:
		return
	if event is InputEventKey and event.pressed and not event.echo:
		var kname := "Ctrl" if event.keycode == KEY_CTRL else OS.get_keycode_string(event.keycode)
		if event.keycode == KEY_ESCAPE or kname == "":
			play_se_requested.emit("Cancel")
			_waiting_key_bind_idx = -1
			_refresh_shortcut_labels()
			get_viewport().set_input_as_handled()
			return
		var sc: Array = config.get("shortcuts", []).duplicate()
		var dup_idx := -1
		for j in range(sc.size()):
			if j != _waiting_key_bind_idx and str(sc[j]).to_lower() == kname.to_lower():
				dup_idx = j
				break
		if dup_idx >= 0:
			play_se_requested.emit("Cancel")
			if _footer_hint_lbl:
				_footer_hint_lbl.text = "快捷键 [%s] 已被第 %d 项占用，请改按其他按键。" % [kname, dup_idx + 1]
			_waiting_key_bind_idx = -1
			_refresh_shortcut_labels()
			get_viewport().set_input_as_handled()
			return
		if _waiting_key_bind_idx < sc.size():
			sc[_waiting_key_bind_idx] = kname
			config["shortcuts"] = sc
			settings_changed.emit(config)
		_waiting_key_bind_idx = -1
		_refresh_shortcut_labels()
		get_viewport().set_input_as_handled()

# ==============================================================================
# Tabs 5 & 6: SAVE / LOAD (SGUsDtWnd* 10页 × 10槽位 = 100 档)
# ==============================================================================
func _build_tab_saveload(pg: Control) -> void:
	_add_bg_image(pg, "SGUsDtWnd990000")

	# SGUsDtWnd990100 (存档蓝) / SGUsDtWnd990101 (读档紫) 为全幅 1280x720 叠加层，包含左上页头与右侧 PREVIEW/COMMENT 框
	_saveload_overlay_rect = TextureRect.new()
	_saveload_overlay_rect.position = Vector2.ZERO
	_saveload_overlay_rect.size = Vector2(1280.0, 720.0)
	_saveload_overlay_rect.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	_saveload_overlay_rect.stretch_mode = TextureRect.STRETCH_SCALE
	_saveload_overlay_rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var dc_tex := _tex("SGUsDtWnd990100")
	if dc_tex:
		_saveload_overlay_rect.texture = dc_tex
	pg.add_child(_saveload_overlay_rect)

	# 右侧预览截图精确对齐底图灰框 [622, 136, 410, 230]
	_save_preview_tex = TextureRect.new()
	_save_preview_tex.position = Vector2(622.0, 136.0)
	_save_preview_tex.size = Vector2(410.0, 230.0)
	_save_preview_tex.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	_save_preview_tex.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
	pg.add_child(_save_preview_tex)

	# 右侧信息区严格分两块：顶部槽号/时间行位于 ❖ 右侧虚线上方 [652..1024, 391..419]，摘要正文位于 COMMENT 磨砂框内 [642..1012, 466..564]
	_save_preview_header = Label.new()
	_save_preview_header.position = Vector2(652.0, 391.0)
	_save_preview_header.size = Vector2(372.0, 28.0)
	_save_preview_header.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	if _sys_font:
		_save_preview_header.add_theme_font_override("font", _sys_font)
	_save_preview_header.add_theme_font_size_override("font_size", 15)
	_save_preview_header.add_theme_color_override("font_color", Color(0.15, 0.25, 0.42, 1.0))
	pg.add_child(_save_preview_header)

	_save_preview_info = Label.new()
	_save_preview_info.position = Vector2(642.0, 466.0)
	_save_preview_info.size = Vector2(370.0, 98.0)
	_save_preview_info.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	if _sys_font:
		_save_preview_info.add_theme_font_override("font", _sys_font)
	_save_preview_info.add_theme_font_size_override("font_size", 15)
	_save_preview_info.add_theme_color_override("font_color", Color(0.15, 0.25, 0.42, 1.0))
	pg.add_child(_save_preview_info)

	# 10 个存档槽按钮 (46/306, 108/208/308/408/508, 252x92)
	var card_save_n := _tex("SGUsDtWnd800000")
	var card_save_h := _tex("SGUsDtWnd800001")
	var card_load_n := _tex("SGUsDtWnd800100")
	var card_load_h := _tex("SGUsDtWnd800101")
	_save_slot_buttons.clear()
	_save_slot_bgs.clear()
	_save_slot_thumbs.clear()
	_save_slot_headers.clear()
	_save_slot_labels.clear()
	for si in range(10):
		var col := si / 5
		var row := si % 5
		var sbtn := Button.new()
		sbtn.position = Vector2(46.0 + 260.0 * float(col), 108.0 + 100.0 * float(row))
		sbtn.size = Vector2(252.0, 92.0)
		sbtn.flat = (card_save_n != null)
		var bg_tr: TextureRect = null
		if card_save_n:
			bg_tr = TextureRect.new()
			bg_tr.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
			bg_tr.texture = card_save_n
			bg_tr.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
			bg_tr.stretch_mode = TextureRect.STRETCH_SCALE
			bg_tr.mouse_filter = Control.MOUSE_FILTER_IGNORE
			sbtn.add_child(bg_tr)
			sbtn.mouse_entered.connect(func():
				var h_tex := card_load_h if (_current_tab == 6 and card_load_h != null) else card_save_h
				if h_tex:
					bg_tr.texture = h_tex
				play_se_requested.emit("Cursor")
			)
			sbtn.mouse_exited.connect(func():
				var n_tex := card_load_n if (_current_tab == 6 and card_load_n != null) else card_save_n
				if n_tex:
					bg_tr.texture = n_tex
			)
		_save_slot_bgs.append(bg_tr)

		# 左侧缩略图框 [12, 10, 128, 72]（仅存在存档时显示，空槽保持底图 NO DATA 雪花）
		var thumb_tr := TextureRect.new()
		thumb_tr.position = Vector2(12.0, 10.0)
		thumb_tr.size = Vector2(128.0, 72.0)
		thumb_tr.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
		thumb_tr.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_COVERED
		thumb_tr.mouse_filter = Control.MOUSE_FILTER_IGNORE
		thumb_tr.visible = false
		sbtn.add_child(thumb_tr)
		_save_slot_thumbs.append(thumb_tr)

		# 右侧顶部槽号 NO.001 (位于 ❖ 右侧、虚线上方 [168, 12, 74, 24])
		var shdr := Label.new()
		shdr.position = Vector2(168.0, 12.0)
		shdr.size = Vector2(74.0, 24.0)
		shdr.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
		if _sys_font:
			shdr.add_theme_font_override("font", _sys_font)
		shdr.add_theme_font_size_override("font_size", 13)
		shdr.add_theme_color_override("font_color", Color(0.16, 0.24, 0.44, 1.0))
		shdr.mouse_filter = Control.MOUSE_FILTER_IGNORE
		sbtn.add_child(shdr)
		_save_slot_headers.append(shdr)

		# 右侧虚线下方时间日期 [150, 44, 94, 42]
		var slbl := Label.new()
		slbl.position = Vector2(150.0, 44.0)
		slbl.size = Vector2(94.0, 42.0)
		slbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		if _sys_font:
			slbl.add_theme_font_override("font", _sys_font)
		slbl.add_theme_font_size_override("font_size", 12)
		slbl.add_theme_color_override("font_color", Color(0.18, 0.26, 0.42, 1.0))
		slbl.mouse_filter = Control.MOUSE_FILTER_IGNORE
		sbtn.add_child(slbl)

		var local_i := si
		sbtn.mouse_entered.connect(func():
			_show_save_preview(_current_save_page * 10 + local_i)
		)
		sbtn.pressed.connect(func():
			play_se_requested.emit("Select_01")
			_on_save_slot_clicked(_current_save_page * 10 + local_i)
		)
		pg.add_child(sbtn)
		_save_slot_buttons.append(sbtn)
		_save_slot_labels.append(slbl)

	# 底部 1..10 分页按钮 [258 + 60*p, 626, 52, 52]
	var pg_n := _tex("SGUsDtWnd000000")
	var pg_h := _tex("SGUsDtWnd000001")
	var pg_p := _tex("SGUsDtWnd000002")
	_save_page_buttons.clear()
	for p in range(10):
		var rect := Rect2(258.0 + 60.0 * float(p), 626.0, 52.0, 52.0)
		var pb := _make_atlas_btn(pg, rect, pg_n, pg_h, pg_p, true, str(p + 1))
		var p_idx := p
		pb.pressed.connect(func():
			play_se_requested.emit("Select_01")
			_current_save_page = p_idx
			_refresh_save_grid()
		)
		_save_page_buttons.append(pb)

func _ensure_saves_dir() -> void:
	var abs_dir := ProjectSettings.globalize_path(SAVES_DIR)
	if not DirAccess.dir_exists_absolute(abs_dir):
		DirAccess.make_dir_recursive_absolute(abs_dir)

static func has_any_save() -> bool:
	var abs_dir := ProjectSettings.globalize_path(SAVES_DIR)
	if not DirAccess.dir_exists_absolute(abs_dir):
		return false
	var dir := DirAccess.open(abs_dir)
	if dir == null:
		return false
	dir.list_dir_begin()
	var fn := dir.get_next()
	while fn != "":
		if fn.ends_with(".json"):
			return true
		fn = dir.get_next()
	return false

static func has_quicksave() -> bool:
	return FileAccess.file_exists(ProjectSettings.globalize_path(SAVES_DIR + "/quicksave.json"))

static func load_save_metadata(slot_key: String) -> Dictionary:
	var path := ProjectSettings.globalize_path("%s/%s.json" % [SAVES_DIR, slot_key])
	if not FileAccess.file_exists(path):
		return {}
	var txt := FileAccess.get_file_as_string(path)
	var parsed: Variant = JSON.parse_string(txt)
	return parsed if parsed is Dictionary else {}

static func write_save_entry(slot_key: String, metadata: Dictionary, thumb_img: Image = null) -> void:
	var abs_dir := ProjectSettings.globalize_path(SAVES_DIR)
	if not DirAccess.dir_exists_absolute(abs_dir):
		DirAccess.make_dir_recursive_absolute(abs_dir)
	var json_path := "%s/%s.json" % [abs_dir, slot_key]
	var f := FileAccess.open(json_path, FileAccess.WRITE)
	if f:
		f.store_string(JSON.stringify(metadata, "\t"))
		f.close()
	if thumb_img != null and not thumb_img.is_empty():
		var scaled := thumb_img.duplicate()
		scaled.resize(410, 230, Image.INTERPOLATE_BILINEAR)
		scaled.save_png("%s/%s.png" % [abs_dir, slot_key])

func refresh_saves_view() -> void:
	if _current_tab == 5 or _current_tab == 6:
		_refresh_save_grid()

func _refresh_save_grid() -> void:
	var is_load: bool = (_current_tab == 6)
	if _saveload_overlay_rect:
		var ov_tex := _tex("SGUsDtWnd990101" if is_load else "SGUsDtWnd990100")
		if ov_tex:
			_saveload_overlay_rect.texture = ov_tex

	var card_tex := _tex("SGUsDtWnd800100" if is_load else "SGUsDtWnd800000")
	var pg_n := _tex("SGUsDtWnd100000" if is_load else "SGUsDtWnd000000")
	var pg_h := _tex("SGUsDtWnd100001" if is_load else "SGUsDtWnd000001")
	var pg_p := _tex("SGUsDtWnd100002" if is_load else "SGUsDtWnd000002")

	for p in range(_save_page_buttons.size()):
		var pb := _save_page_buttons[p]
		var rect := Rect2(258.0 + 60.0 * float(p), 626.0, 52.0, 52.0)
		if pg_n:
			pb.texture_normal = _atlas(pg_n, rect)
		if pg_h:
			pb.texture_hover = _atlas(pg_h, rect)
		if pg_p:
			pb.texture_pressed = _atlas(pg_p, rect)
		pb.set_pressed_no_signal(p == _current_save_page)

	for i in range(10):
		if i < _save_slot_bgs.size() and _save_slot_bgs[i] != null and card_tex != null:
			_save_slot_bgs[i].texture = card_tex
		var g_slot := _current_save_page * 10 + i
		var key := "slot_%03d" % g_slot
		var meta := load_save_metadata(key)
		_saves_cache[g_slot] = meta
		_save_slot_headers[i].text = "NO.%03d" % (g_slot + 1)
		if meta.is_empty():
			_save_slot_labels[i].text = ""
			if i < _save_slot_thumbs.size():
				_save_slot_thumbs[i].texture = null
				_save_slot_thumbs[i].visible = false
		else:
			var ts := str(meta.get("timestamp", "")).replace("-", "/")
			_save_slot_labels[i].text = ts.replace(" ", "\n")
			var png_path := ProjectSettings.globalize_path("%s/slot_%03d.png" % [SAVES_DIR, g_slot])
			if i < _save_slot_thumbs.size() and FileAccess.file_exists(png_path):
				var im := Image.load_from_file(png_path)
				if im and not im.is_empty():
					_save_slot_thumbs[i].texture = ImageTexture.create_from_image(im)
					_save_slot_thumbs[i].visible = true
				else:
					_save_slot_thumbs[i].visible = false
			elif i < _save_slot_thumbs.size():
				_save_slot_thumbs[i].visible = false

	var preview_id := _selected_save_slot if (_selected_save_slot >= _current_save_page * 10 and _selected_save_slot < (_current_save_page + 1) * 10) else (_current_save_page * 10)
	_show_save_preview(preview_id)

func _show_save_preview(global_slot: int) -> void:
	_selected_save_slot = global_slot
	var meta: Dictionary = _saves_cache.get(global_slot, {})
	if meta.is_empty():
		_save_preview_tex.texture = null
		if _save_preview_header:
			_save_preview_header.text = "NO.%03d   [ 空存档位 ]" % (global_slot + 1)
		_save_preview_info.text = "尚未保存任何游戏进度。"
		return

	var png_path := ProjectSettings.globalize_path("%s/slot_%03d.png" % [SAVES_DIR, global_slot])
	if FileAccess.file_exists(png_path):
		var im := Image.load_from_file(png_path)
		if im and not im.is_empty():
			_save_preview_tex.texture = ImageTexture.create_from_image(im)
	else:
		_save_preview_tex.texture = null

	var spk := _sanitize_protagonist_name(str(meta.get("speaker", "")))
	var txt := _sanitize_protagonist_name(str(meta.get("text", "")))
	var line_prefix := (spk + "：") if spk != "" else ""
	if _save_preview_header:
		_save_preview_header.text = "NO.%03d   %s" % [global_slot + 1, str(meta.get("timestamp", "")).replace("-", "/")]
	_save_preview_info.text = "剧本：%s (第 %d 句)\n%s%s" % [
		str(meta.get("script_name", "")),
		int(meta.get("dialogue_index", 0)),
		line_prefix,
		txt,
	]

func _sanitize_protagonist_name(s: String) -> String:
	var out := s
	if out.contains("主人公"):
		out = out.replace("主人公（姓）　主人公（名）", "李骏")
		out = out.replace("主人公（姓） 主人公（名）", "李骏")
		out = out.replace("主人公（姓）主人公（名）", "李骏")
		out = out.replace("主人公（名）", "李骏")
		out = out.replace("主人公（姓）", "李")
	out = out.replace("想要一直呆在隼人君的身边", "想要一直呆在前辈的身边")
	out = out.replace("今天一整天都能和最喜欢的宇佐喵一起约会", "今天一整天都能和最喜欢的你一起约会")
	out = out.replace("在家的时候一直都在在意隼人的事", "在家的时候一直都在在意你的事")
	out = out.replace("总觉得隼人像小孩子一样", "总觉得你像小孩子一样")
	return out

func _on_save_slot_clicked(global_slot: int) -> void:
	var existing: Dictionary = _saves_cache.get(global_slot, {})
	if _current_tab == 5:
		# SAVE 模式
		if existing.is_empty():
			save_to_slot_requested.emit(global_slot)
		else:
			_open_confirm("是否覆盖存档 No.%03d？" % (global_slot + 1), func():
				save_to_slot_requested.emit(global_slot)
			, "SGDialog990003")
	elif _current_tab == 6:
		# LOAD 模式
		if existing.is_empty():
			return
		_open_confirm("是否读取存档 No.%03d？" % (global_slot + 1), func():
			load_from_slot_requested.emit(existing)
			close_panel()
		, "SGDialog990005")

# ==============================================================================
# 通用 SGDialog 模态确认弹窗
# ==============================================================================
func _set_shell_chrome_visible(show_chrome: bool) -> void:
	if _dim_bg:
		_dim_bg.visible = show_chrome
	if _sys_bg_rect:
		_sys_bg_rect.visible = show_chrome
	if _header_hint_rect:
		_header_hint_rect.visible = show_chrome and (_header_hint_rect.texture != null)
	if _hint_bg_tex:
		_hint_bg_tex.visible = show_chrome and (_current_tab < 5)
	if _footer_hint_lbl:
		_footer_hint_lbl.visible = show_chrome and (_current_tab < 5)
	for b in _tab_buttons:
		b.visible = show_chrome
	for k in _action_buttons.keys():
		var ab: Control = _action_buttons[k]
		if k == "exit" and OS.get_name() == "iOS":
			ab.visible = false
		elif k == "default":
			ab.visible = show_chrome and (_current_tab < 5)
		else:
			ab.visible = show_chrome
	for i in range(_tab_pages.size()):
		if not show_chrome:
			_tab_pages[i].visible = false
		else:
			if i < 5:
				_tab_pages[i].visible = (i == _current_tab)
			else:
				_tab_pages[i].visible = (_current_tab == 5 or _current_tab == 6)

func _build_confirm_modal() -> void:
	_confirm_modal = Control.new()
	_confirm_modal.name = "ConfirmModal"
	_confirm_modal.z_index = 10
	_confirm_modal.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_confirm_modal.mouse_filter = Control.MOUSE_FILTER_STOP
	_confirm_modal.visible = false
	add_child(_confirm_modal)

	var bg_tex := _tex("SGDialog990000")
	if bg_tex:
		_confirm_bg_rect = TextureRect.new()
		_confirm_bg_rect.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		_confirm_bg_rect.texture = bg_tex
		_confirm_bg_rect.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
		_confirm_bg_rect.stretch_mode = TextureRect.STRETCH_SCALE
		_confirm_bg_rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
		_confirm_modal.add_child(_confirm_bg_rect)

		# 用同模板下部空白雪花纹带掩盖原图预置文字，避免自定义提示文案产生双字重影
		_confirm_mask_rect = TextureRect.new()
		_confirm_mask_rect.position = Vector2(360.0, 298.0)
		_confirm_mask_rect.size = Vector2(560.0, 50.0)
		_confirm_mask_rect.texture = _atlas(bg_tex, Rect2(360.0, 362.0, 560.0, 50.0))
		_confirm_mask_rect.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
		_confirm_mask_rect.stretch_mode = TextureRect.STRETCH_SCALE
		_confirm_mask_rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
		_confirm_mask_rect.visible = false
		_confirm_modal.add_child(_confirm_mask_rect)
	else:
		var dim := ColorRect.new()
		dim.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		dim.color = Color(0, 0, 0, 0.55)
		_confirm_modal.add_child(dim)

	_confirm_msg_lbl = Label.new()
	_confirm_msg_lbl.position = Vector2(360.0, 294.0)
	_confirm_msg_lbl.size = Vector2(560.0, 56.0)
	_confirm_msg_lbl.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_confirm_msg_lbl.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	_confirm_msg_lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	if _sys_font:
		_confirm_msg_lbl.add_theme_font_override("font", _sys_font)
	_confirm_msg_lbl.add_theme_font_size_override("font_size", 22)
	_confirm_msg_lbl.add_theme_color_override("font_color", Color(0.14, 0.28, 0.56, 1.0))
	_confirm_msg_lbl.add_theme_color_override("font_outline_color", Color.WHITE)
	_confirm_msg_lbl.add_theme_constant_override("outline_size", 4)
	_confirm_modal.add_child(_confirm_msg_lbl)

	var yes_btn := TextureButton.new()
	yes_btn.name = "ConfirmYesBtn"
	yes_btn.position = Vector2(476.0, 382.0)
	yes_btn.size = Vector2(146.0, 38.0)
	yes_btn.ignore_texture_size = true
	yes_btn.stretch_mode = TextureButton.STRETCH_SCALE
	var yn := _tex("SGDialog000000")
	var yh := _tex("SGDialog000001")
	if yn: yes_btn.texture_normal = yn
	if yh:
		yes_btn.texture_hover = yh
		yes_btn.texture_pressed = yh
	yes_btn.pressed.connect(func():
		play_se_requested.emit("Select_01")
		_confirm_modal.visible = false
		if _is_standalone_confirm:
			_is_standalone_confirm = false
			visible = false
			_set_shell_chrome_visible(true)
		if _confirm_ok_cb.is_valid():
			_confirm_ok_cb.call()
	)
	_confirm_modal.add_child(yes_btn)

	var no_btn := TextureButton.new()
	no_btn.name = "ConfirmNoBtn"
	no_btn.position = Vector2(658.0, 382.0)
	no_btn.size = Vector2(146.0, 38.0)
	no_btn.ignore_texture_size = true
	no_btn.stretch_mode = TextureButton.STRETCH_SCALE
	var nn := _tex("SGDialog000100")
	var nh := _tex("SGDialog000101")
	if nn: no_btn.texture_normal = nn
	if nh:
		no_btn.texture_hover = nh
		no_btn.texture_pressed = nh
	no_btn.pressed.connect(func():
		play_se_requested.emit("Cancel")
		dismiss_standalone_confirm()
	)
	_confirm_modal.add_child(no_btn)

func dismiss_standalone_confirm() -> void:
	if _confirm_modal:
		_confirm_modal.visible = false
	if _is_standalone_confirm:
		_is_standalone_confirm = false
		visible = false
		_set_shell_chrome_visible(true)

func open_standalone_confirm(msg: String, on_yes: Callable, dialog_asset: String = "") -> void:
	_build_root_shell()
	_is_standalone_confirm = true
	_set_shell_chrome_visible(false)
	visible = true
	move_to_front()
	_open_confirm(msg, on_yes, dialog_asset)

func _open_confirm(msg: String, on_yes: Callable, dialog_asset: String = "") -> void:
	if _confirm_modal == null:
		_build_confirm_modal()
	var dtex := _tex(dialog_asset) if dialog_asset != "" else null
	if _confirm_bg_rect:
		if dtex != null:
			_confirm_bg_rect.texture = dtex
			if _confirm_mask_rect:
				_confirm_mask_rect.visible = false
			_confirm_msg_lbl.visible = false
		else:
			var fb_tex := _tex("SGDialog990000")
			if fb_tex:
				_confirm_bg_rect.texture = fb_tex
			if _confirm_mask_rect:
				_confirm_mask_rect.visible = true
			_confirm_msg_lbl.text = msg
			_confirm_msg_lbl.visible = true
	else:
		_confirm_msg_lbl.text = msg
		_confirm_msg_lbl.visible = true
	_confirm_ok_cb = on_yes
	_confirm_modal.visible = true
	_confirm_modal.move_to_front()

# ==============================================================================
# 通用选项组绑定与同步刷新
# ==============================================================================
func _bind_option_group(
	pg: Control,
	n_tex: Texture2D,
	h_tex: Texture2D,
	p_tex: Texture2D,
	rects: Array,
	cfg_key: String,
	labels: Array,
	values: Array = []
) -> void:
	var btns: Array[TextureButton] = []
	for i in range(rects.size()):
		var fb: String = labels[i] if (i < labels.size() and n_tex == null) else ""
		var b := _make_atlas_btn(pg, rects[i], n_tex, h_tex, p_tex, true, fb)
		var val: int = int(values[i]) if (i < values.size()) else i
		b.pressed.connect(func():
			play_se_requested.emit("Select_01")
			config[cfg_key] = val
			settings_changed.emit(config)
			for j in range(btns.size()):
				var j_val: int = int(values[j]) if (j < values.size()) else j
				btns[j].set_pressed_no_signal(j_val == val)
			_sync_previews_only()
		)
		btns.append(b)

	_sync_callbacks.append(func():
		var cur := int(config.get(cfg_key, 0))
		for j in range(btns.size()):
			var j_val: int = int(values[j]) if (j < values.size()) else j
			btns[j].set_pressed_no_signal(j_val == cur)
	)

func _bind_bool_pair(
	pg: Control,
	n_tex: Texture2D,
	h_tex: Texture2D,
	p_tex: Texture2D,
	rect_true: Rect2,
	rect_false: Rect2,
	cfg_key: String
) -> void:
	var bt := _make_atlas_btn(pg, rect_true, n_tex, h_tex, p_tex, true, "ON")
	var bf := _make_atlas_btn(pg, rect_false, n_tex, h_tex, p_tex, true, "OFF")
	bt.pressed.connect(func():
		play_se_requested.emit("Select_01")
		config[cfg_key] = true
		settings_changed.emit(config)
		bt.set_pressed_no_signal(true)
		bf.set_pressed_no_signal(false)
	)
	bf.pressed.connect(func():
		play_se_requested.emit("Select_01")
		config[cfg_key] = false
		settings_changed.emit(config)
		bt.set_pressed_no_signal(false)
		bf.set_pressed_no_signal(true)
	)
	_sync_callbacks.append(func():
		var v := bool(config.get(cfg_key, false))
		bt.set_pressed_no_signal(v)
		bf.set_pressed_no_signal(not v)
	)

func _sync_all_ui() -> void:
	for cb in _sync_callbacks:
		if cb.is_valid():
			cb.call()
	_sync_previews_only()

func _sync_previews_only() -> void:
	if _dsp_preview_box:
		var st := _dsp_preview_box.get_theme_stylebox("panel")
		if st is StyleBoxFlat:
			st.bg_color.a = clampf(float(config.get("window_alpha", 0.88)), 0.2, 1.0)
	var tc := Color(str(config.get("text_color", "#ffffff")))
	var rc := Color(str(config.get("read_color", "#99d4ff")))
	if _txt_preview_lbl:
		_txt_preview_lbl.add_theme_color_override("font_color", tc)
	if _txt_read_preview_lbl:
		_txt_read_preview_lbl.add_theme_color_override("font_color", rc)
	if _txt_target_swatches.size() == 2:
		_txt_target_swatches[0].color = tc
		_txt_target_swatches[1].color = rc
	for bi in range(_txt_target_btns.size()):
		var active := (bi == _active_color_target)
		var ring := _txt_target_btns[bi].get_node_or_null("ActiveRing")
		if ring:
			ring.visible = active
		if bi < _txt_target_lbls.size():
			if bi == 0:
				_txt_target_lbls[bi].text = "普通文本颜色（设色中）" if active else "普通文本颜色（点击切换）"
			else:
				_txt_target_lbls[bi].text = "已读文本颜色（设色中）" if active else "已读文本颜色（点击切换）"
	if _txt_now_swatch:
		_txt_now_swatch.color = tc if _active_color_target == 0 else rc
