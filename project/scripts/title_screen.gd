extends Control
class_name TitleScreenController

## 1:1 复刻 PC 原版主标题界面 (SGTitle990000 + SGTitle000000/000001 6 按钮 + 附赠 CG/音乐室)

signal start_new_game_requested
signal continue_load_requested
signal quick_load_requested
signal config_requested
signal quit_game_requested
signal play_se_requested(sfx_name: String)
signal play_bgm_requested(bgm_name: String)

var _engine: Object = null
var _sys_font: Font = null

var _bg_rect: TextureRect
var _menu_container: Control
var _buttons: Array[TextureButton] = []

var _omake_panel: Control = null
var _omake_cg_view: TextureRect = null
var _omake_cg_list: GridContainer = null
var _omake_bgm_list: VBoxContainer = null
var _omake_tab_cg_btn: Button = null
var _omake_tab_bgm_btn: Button = null
var _omake_cg_scroll: ScrollContainer = null
var _omake_bgm_scroll: ScrollContainer = null

var _quit_dialog: Control = null

const BUTTON_SPECS: Array[Dictionary] = [
	{"name": "BtnNewGame",   "label": "从头开始"},
	{"name": "BtnContinue",  "label": "继续游戏"},
	{"name": "BtnQuickLoad", "label": "快速读档"},
	{"name": "BtnConfig",    "label": "系统设置"},
	{"name": "BtnOmake",     "label": "附赠鉴赏"},
	{"name": "BtnExit",      "label": "结束游戏"},
]

const GALLERY_BG_CANDIDATES: PackedStringArray = [
	"pc_bg_01A", "pc_bg_01B", "pc_bg_04B", "pc_bg_04B_9", "pc_bg_13A",
	"pc_bg_28B", "pc_bg_42B", "pc_bg_44A", "pc_bg_46B", "pc_bg_60B",
	"ec_sh_04", "ec_ar_04", "ec_so_04", "ec_mo_04", "ec_ay_04",
]

func _init() -> void:
	visible = false

func _ready() -> void:
	name = "TitleScreen"
	set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	mouse_filter = Control.MOUSE_FILTER_STOP
	z_index = 30
	visible = false
	_build_ui()

func setup(engine_ref: Object, font: Font) -> void:
	_engine = engine_ref
	_sys_font = font
	_build_ui()
	_refresh_textures()

func _build_ui() -> void:
	if _bg_rect != null:
		return

	_bg_rect = TextureRect.new()
	_bg_rect.name = "TitleBg"
	_bg_rect.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_bg_rect.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	_bg_rect.stretch_mode = TextureRect.STRETCH_SCALE
	_bg_rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(_bg_rect)

	_menu_container = Control.new()
	_menu_container.name = "MenuHContainer"
	_menu_container.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_menu_container.mouse_filter = Control.MOUSE_FILTER_PASS
	add_child(_menu_container)

	_buttons.clear()
	var is_ios: bool = (OS.get_name() == "iOS")
	var x_shift: float = 95.0 if is_ios else 0.0

	for i in range(BUTTON_SPECS.size()):
		var spec: Dictionary = BUTTON_SPECS[i]
		var btn := TextureButton.new()
		btn.name = spec["name"]
		btn.ignore_texture_size = true
		btn.stretch_mode = TextureButton.STRETCH_SCALE
		btn.position = Vector2(81.0 + 190.0 * float(i) + x_shift, 655.0)
		btn.size = Vector2(172.0, 52.0)

		if is_ios and i == 5:
			btn.visible = false

		btn.mouse_entered.connect(func():
			if btn.visible:
				play_se_requested.emit("Cursor")
		)
		btn.button_down.connect(func():
			btn.modulate = Color(0.84, 0.92, 1.0, 1.0)
		)
		btn.button_up.connect(func():
			btn.modulate = Color.WHITE
		)
		var idx := i
		btn.pressed.connect(func():
			btn.modulate = Color.WHITE
			_on_button_pressed(idx)
		)

		_menu_container.add_child(btn)
		_buttons.append(btn)

func _refresh_textures() -> void:
	if _engine == null:
		return
	var tex_bg: Variant = _engine.load_ui_texture("SGTitle990000")
	if tex_bg is Texture2D:
		_bg_rect.texture = tex_bg
	else:
		# 无 sysgrp 时回退
		var fb: Variant = _engine.load_ui_texture("title_bg")
		if fb is Texture2D:
			_bg_rect.texture = fb

	var tex_normal: Variant = _engine.load_ui_texture("SGTitle000000")
	var tex_hover: Variant = _engine.load_ui_texture("SGTitle000001")
	if tex_normal is Texture2D:
		for i in range(_buttons.size()):
			var region := Rect2(81.0 + 190.0 * float(i), 655.0, 172.0, 52.0)
			var at_n := AtlasTexture.new()
			at_n.atlas = tex_normal
			at_n.region = region
			_buttons[i].texture_normal = at_n

			if tex_hover is Texture2D:
				var at_h := AtlasTexture.new()
				at_h.atlas = tex_hover
				at_h.region = region
				_buttons[i].texture_hover = at_h
				_buttons[i].texture_pressed = at_h
				_buttons[i].texture_focused = at_h

func _on_button_pressed(idx: int) -> void:
	play_se_requested.emit("Select_01")
	match idx:
		0:
			start_new_game_requested.emit()
		1:
			continue_load_requested.emit()
		2:
			quick_load_requested.emit()
		3:
			config_requested.emit()
		4:
			_open_omake_panel()
		5:
			_open_quit_confirm()

func _open_omake_panel() -> void:
	if _omake_panel == null:
		_build_omake_panel()
	_omake_panel.visible = true
	_omake_panel.move_to_front()
	_populate_omake_gallery()
	_populate_omake_music()

func _close_omake_panel() -> void:
	play_se_requested.emit("Cancel")
	if _omake_panel:
		_omake_panel.visible = false
	if _omake_cg_view:
		_omake_cg_view.visible = false
	# 离开音乐室时恢复标题曲
	play_bgm_requested.emit("title_bgm")

func _build_omake_panel() -> void:
	_omake_panel = Control.new()
	_omake_panel.name = "OmakePanel"
	_omake_panel.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_omake_panel.mouse_filter = Control.MOUSE_FILTER_STOP
	_omake_panel.visible = false
	add_child(_omake_panel)

	var bg_tex: Variant = _engine.load_ui_texture("SGGallery990000") if _engine else null
	if bg_tex is Texture2D:
		var tr := TextureRect.new()
		tr.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		tr.texture = bg_tex
		tr.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
		tr.stretch_mode = TextureRect.STRETCH_SCALE
		tr.mouse_filter = Control.MOUSE_FILTER_IGNORE
		_omake_panel.add_child(tr)
	else:
		var cr := ColorRect.new()
		cr.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		cr.color = Color(0.92, 0.96, 1.0, 0.96)
		cr.mouse_filter = Control.MOUSE_FILTER_IGNORE
		_omake_panel.add_child(cr)

	var header := HBoxContainer.new()
	header.position = Vector2(48, 24)
	header.size = Vector2(1184, 52)
	header.add_theme_constant_override("separation", 16)
	_omake_panel.add_child(header)

	var title_lbl := Label.new()
	title_lbl.text = "附赠鉴赏 / EXTRA GALLERY"
	if _sys_font:
		title_lbl.add_theme_font_override("font", _sys_font)
	title_lbl.add_theme_font_size_override("font_size", 24)
	title_lbl.add_theme_color_override("font_color", Color(0.12, 0.25, 0.45, 1.0))
	header.add_child(title_lbl)

	var spacer := Control.new()
	spacer.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	header.add_child(spacer)

	_omake_tab_cg_btn = Button.new()
	_omake_tab_cg_btn.text = " CG 赏析 "
	_omake_tab_cg_btn.custom_minimum_size = Vector2(130, 42)
	if _sys_font:
		_omake_tab_cg_btn.add_theme_font_override("font", _sys_font)
	_omake_tab_cg_btn.pressed.connect(func():
		play_se_requested.emit("Select_01")
		_omake_cg_scroll.visible = true
		_omake_bgm_scroll.visible = false
	)
	header.add_child(_omake_tab_cg_btn)

	_omake_tab_bgm_btn = Button.new()
	_omake_tab_bgm_btn.text = " 音乐鉴赏 "
	_omake_tab_bgm_btn.custom_minimum_size = Vector2(130, 42)
	if _sys_font:
		_omake_tab_bgm_btn.add_theme_font_override("font", _sys_font)
	_omake_tab_bgm_btn.pressed.connect(func():
		play_se_requested.emit("Select_01")
		_omake_cg_scroll.visible = false
		_omake_bgm_scroll.visible = true
	)
	header.add_child(_omake_tab_bgm_btn)

	var back_btn := Button.new()
	back_btn.text = " 返回标题 (BACK) "
	back_btn.custom_minimum_size = Vector2(160, 42)
	if _sys_font:
		back_btn.add_theme_font_override("font", _sys_font)
	back_btn.pressed.connect(_close_omake_panel)
	header.add_child(back_btn)

	_omake_cg_scroll = ScrollContainer.new()
	_omake_cg_scroll.position = Vector2(64, 108)
	_omake_cg_scroll.size = Vector2(1152, 560)
	_omake_panel.add_child(_omake_cg_scroll)

	_omake_cg_list = GridContainer.new()
	_omake_cg_list.columns = 4
	_omake_cg_list.add_theme_constant_override("h_separation", 20)
	_omake_cg_list.add_theme_constant_override("v_separation", 20)
	_omake_cg_scroll.add_child(_omake_cg_list)

	_omake_bgm_scroll = ScrollContainer.new()
	_omake_bgm_scroll.position = Vector2(140, 108)
	_omake_bgm_scroll.size = Vector2(1000, 560)
	_omake_bgm_scroll.visible = false
	_omake_panel.add_child(_omake_bgm_scroll)

	_omake_bgm_list = VBoxContainer.new()
	_omake_bgm_list.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_omake_bgm_list.add_theme_constant_override("separation", 10)
	_omake_bgm_scroll.add_child(_omake_bgm_list)

	# CG 全屏预览浮层
	_omake_cg_view = TextureRect.new()
	_omake_cg_view.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_omake_cg_view.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	_omake_cg_view.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
	_omake_cg_view.mouse_filter = Control.MOUSE_FILTER_STOP
	_omake_cg_view.visible = false
	_omake_cg_view.gui_input.connect(func(ev):
		if (ev is InputEventMouseButton and ev.pressed) or (ev is InputEventScreenTouch and ev.pressed):
			_omake_cg_view.visible = false
			play_se_requested.emit("Cancel")
	)
	_omake_panel.add_child(_omake_cg_view)

func _populate_omake_gallery() -> void:
	if _omake_cg_list == null or _engine == null:
		return
	if _omake_cg_list.get_child_count() > 0:
		return
	for cg_name in GALLERY_BG_CANDIDATES:
		var tex: Variant = _engine.load_ui_texture(cg_name)
		if not (tex is Texture2D):
			continue
		var card := TextureButton.new()
		card.custom_minimum_size = Vector2(265, 150)
		card.ignore_texture_size = true
		card.stretch_mode = TextureButton.STRETCH_KEEP_ASPECT_CENTERED
		card.texture_normal = tex
		card.pressed.connect(func():
			play_se_requested.emit("Select_01")
			_omake_cg_view.texture = tex
			_omake_cg_view.visible = true
			_omake_cg_view.move_to_front()
		)
		_omake_cg_list.add_child(card)

func _populate_omake_music() -> void:
	if _omake_bgm_list == null or _engine == null:
		return
	if _omake_bgm_list.get_child_count() > 0:
		return
	var tracks: Array[String] = ["title_bgm"]
	for n in range(1, 29):
		tracks.append("PC_BGM_%02d" % n)
	for tr_name in tracks:
		var btn := Button.new()
		btn.text = "♪  播放原声曲目：%s" % tr_name
		btn.custom_minimum_size = Vector2(0, 40)
		btn.alignment = HORIZONTAL_ALIGNMENT_LEFT
		if _sys_font:
			btn.add_theme_font_override("font", _sys_font)
		var captured := tr_name
		btn.pressed.connect(func():
			play_se_requested.emit("Select_01")
			play_bgm_requested.emit(captured)
		)
		_omake_bgm_list.add_child(btn)

func _open_quit_confirm() -> void:
	if _quit_dialog == null:
		_build_quit_dialog()
	_quit_dialog.visible = true
	_quit_dialog.move_to_front()

func _build_quit_dialog() -> void:
	_quit_dialog = Control.new()
	_quit_dialog.name = "TitleQuitConfirmDialog"
	_quit_dialog.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_quit_dialog.mouse_filter = Control.MOUSE_FILTER_STOP
	_quit_dialog.visible = false
	add_child(_quit_dialog)

	var bg_tex: Variant = _engine.load_ui_texture("SGDialog990000") if _engine else null
	if not (bg_tex is Texture2D) and _engine:
		bg_tex = _engine.load_ui_texture("SGDialog990001")

	if bg_tex is Texture2D:
		var tr := TextureRect.new()
		tr.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		tr.texture = bg_tex
		tr.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
		tr.stretch_mode = TextureRect.STRETCH_SCALE
		tr.mouse_filter = Control.MOUSE_FILTER_IGNORE
		_quit_dialog.add_child(tr)
	else:
		var dim := ColorRect.new()
		dim.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
		dim.color = Color(0, 0, 0, 0.55)
		_quit_dialog.add_child(dim)
		var lbl := Label.new()
		lbl.text = "确定要结束游戏吗？"
		lbl.position = Vector2(460, 310)
		if _sys_font:
			lbl.add_theme_font_override("font", _sys_font)
		lbl.add_theme_font_size_override("font_size", 24)
		_quit_dialog.add_child(lbl)

	var yes_btn := TextureButton.new()
	yes_btn.name = "QuitYesBtn"
	yes_btn.position = Vector2(476.0, 382.0)
	yes_btn.size = Vector2(146.0, 38.0)
	yes_btn.ignore_texture_size = true
	yes_btn.stretch_mode = TextureButton.STRETCH_SCALE

	var no_btn := TextureButton.new()
	no_btn.name = "QuitNoBtn"
	no_btn.position = Vector2(658.0, 382.0)
	no_btn.size = Vector2(146.0, 38.0)
	no_btn.ignore_texture_size = true
	no_btn.stretch_mode = TextureButton.STRETCH_SCALE

	if _engine:
		var yn = _engine.load_ui_texture("SGDialog000000")
		var yh = _engine.load_ui_texture("SGDialog000001")
		var nn = _engine.load_ui_texture("SGDialog000100")
		var nh = _engine.load_ui_texture("SGDialog000101")
		if yn is Texture2D:
			yes_btn.texture_normal = yn
		if yh is Texture2D:
			yes_btn.texture_hover = yh
			yes_btn.texture_pressed = yh
		if nn is Texture2D:
			no_btn.texture_normal = nn
		if nh is Texture2D:
			no_btn.texture_hover = nh
			no_btn.texture_pressed = nh

	yes_btn.pressed.connect(func():
		play_se_requested.emit("Select_01")
		_quit_dialog.visible = false
		quit_game_requested.emit()
	)
	no_btn.pressed.connect(func():
		play_se_requested.emit("Cancel")
		_quit_dialog.visible = false
	)
	_quit_dialog.add_child(yes_btn)
	_quit_dialog.add_child(no_btn)
