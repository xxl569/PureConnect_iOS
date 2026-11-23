extends Control
class_name CharacterIntroLayer

## 1:1 复刻 PC 原版 5 图层角色登场平滑介绍演播层 (ec_{sh,ar,so,mo,ay}_01..05)

signal intro_finished

const HEROINE_METADATA: Dictionary = {
	"sora": {
		"name_cn": "一瀬 空",
		"name_romaji": "ICHINOSE SORA",
		"motto": "青梅竹马什么的，最喜欢了！",
		"color": Color(0.35, 0.70, 0.95, 1.0),
		"sprite": "SO01_N001",
		"dir": 1.0,
	},
	"moemi": {
		"name_cn": "道明寺 萌美",
		"name_romaji": "DOUMYOUJI MOEMI",
		"motto": "今天也要打起精神，制作最棒的甜点！",
		"color": Color(0.98, 0.60, 0.35, 1.0),
		"sprite": "MO01_M001",
		"dir": -1.0,
	},
	"arisa": {
		"name_cn": "绀野 彩里沙",
		"name_romaji": "KONNO ARISA",
		"motto": "命运之人什么的，完全不存在！",
		"color": Color(0.95, 0.82, 0.32, 1.0),
		"sprite": "AR01_N001",
		"dir": -1.0,
	},
	"ayumi": {
		"name_cn": "筱崎 步海",
		"name_romaji": "SHINOZAKI AYUMI",
		"motto": "如果能一直在一起的话，该有多好呢。",
		"color": Color(0.96, 0.48, 0.68, 1.0),
		"sprite": "AY01_N001",
		"dir": 1.0,
	},
	"shiho": {
		"name_cn": "牧原 志帆",
		"name_romaji": "MAKIHARA SHIHO",
		"motto": "好，今天也加把劲，努力工作吧！",
		"color": Color(0.72, 0.60, 0.92, 1.0),
		"sprite": "SI01_N001",
		"dir": 1.0,
	},
}

var _white_backdrop: ColorRect
var _layers: Array[TextureRect] = []
var _fallback_panel: Control
var _fallback_card_rect: TextureRect
var _fallback_name_lbl: Label
var _fallback_romaji_lbl: Label
var _fallback_motto_lbl: Label
var _fallback_tag_lbl: Label

var jingle_player: AudioStreamPlayer
var se_player: AudioStreamPlayer

var _tween: Tween = null
var _is_playing: bool = false
var _play_start_frame: int = -1
var _sys_font: Font = null

func _init() -> void:
	visible = false

func _ready() -> void:
	mouse_filter = Control.MOUSE_FILTER_STOP
	set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	clip_contents = true
	z_index = 25
	visible = false
	modulate.a = 0.0
	_build_nodes()

func setup_font(font: Font) -> void:
	_sys_font = font
	_build_nodes()
	for lbl in [_fallback_name_lbl, _fallback_romaji_lbl, _fallback_motto_lbl, _fallback_tag_lbl]:
		if lbl and _sys_font:
			lbl.add_theme_font_override("font", _sys_font)

func set_audio_volumes(voice_db: float, se_db: float) -> void:
	if jingle_player:
		jingle_player.volume_db = voice_db
	if se_player:
		se_player.volume_db = se_db

func _build_nodes() -> void:
	if _white_backdrop != null:
		return

	# 原版白底遮罩 (防止底图渐变边缘透出下层舞台)
	_white_backdrop = ColorRect.new()
	_white_backdrop.name = "WhiteBackdrop"
	_white_backdrop.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_white_backdrop.color = Color(0.96, 0.98, 1.0, 1.0)
	_white_backdrop.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(_white_backdrop)

	# 5 层原生 CG 图层 (ec_*_01..05)
	_layers.clear()
	for i in range(1, 6):
		var tr = TextureRect.new()
		tr.name = "Layer0%d" % i
		tr.mouse_filter = Control.MOUSE_FILTER_IGNORE
		tr.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
		tr.stretch_mode = TextureRect.STRETCH_SCALE
		if i == 3 or i == 4:
			tr.position = Vector2(-314.0, -36.0)
			tr.size = Vector2(1908.0, 792.0)
		else:
			tr.position = Vector2(-64.0, -36.0)
			tr.size = Vector2(1408.0, 792.0)
		tr.visible = false
		add_child(tr)
		_layers.append(tr)

	# 单卡回退面板 (仅当测试/缺失 5 图层时使用，保持验收接口兼容)
	_fallback_panel = Control.new()
	_fallback_panel.name = "IntroCardPanel"
	_fallback_panel.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_fallback_panel.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_fallback_panel.visible = false
	add_child(_fallback_panel)

	var fb_bg = ColorRect.new()
	fb_bg.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	fb_bg.color = Color(0.03, 0.06, 0.14, 0.90)
	fb_bg.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_fallback_panel.add_child(fb_bg)

	_fallback_card_rect = TextureRect.new()
	_fallback_card_rect.name = "IntroCardRect"
	_fallback_card_rect.position = Vector2(520, 10)
	_fallback_card_rect.size = Vector2(720, 700)
	_fallback_card_rect.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	_fallback_card_rect.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
	_fallback_card_rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_fallback_panel.add_child(_fallback_card_rect)

	var text_col = VBoxContainer.new()
	text_col.position = Vector2(96, 180)
	text_col.size = Vector2(560, 360)
	text_col.add_theme_constant_override("separation", 14)
	text_col.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_fallback_panel.add_child(text_col)

	_fallback_tag_lbl = Label.new()
	_fallback_tag_lbl.text = "CHARACTER INTRODUCTION"
	_fallback_tag_lbl.add_theme_font_size_override("font_size", 16)
	_fallback_tag_lbl.add_theme_color_override("font_color", Color(0.65, 0.85, 1.0, 0.9))
	text_col.add_child(_fallback_tag_lbl)

	_fallback_name_lbl = Label.new()
	_fallback_name_lbl.add_theme_font_size_override("font_size", 56)
	_fallback_name_lbl.add_theme_color_override("font_color", Color.WHITE)
	text_col.add_child(_fallback_name_lbl)

	_fallback_romaji_lbl = Label.new()
	_fallback_romaji_lbl.add_theme_font_size_override("font_size", 22)
	_fallback_romaji_lbl.add_theme_color_override("font_color", Color(0.85, 0.92, 1.0, 0.9))
	text_col.add_child(_fallback_romaji_lbl)

	_fallback_motto_lbl = Label.new()
	_fallback_motto_lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_fallback_motto_lbl.add_theme_font_size_override("font_size", 26)
	_fallback_motto_lbl.add_theme_color_override("font_color", Color(0.95, 0.98, 1.0, 1.0))
	text_col.add_child(_fallback_motto_lbl)

	jingle_player = AudioStreamPlayer.new()
	jingle_player.name = "IntroJinglePlayer"
	add_child(jingle_player)

	se_player = AudioStreamPlayer.new()
	se_player.name = "IntroSePlayer"
	add_child(se_player)

	if _sys_font:
		setup_font(_sys_font)

func play_intro(intro: Dictionary, headless_instant: bool = false) -> void:
	_build_nodes()
	if _tween and _tween.is_valid():
		_tween.kill()

	var char_id = str(intro.get("character", "sora")).to_lower()
	var meta: Dictionary = HEROINE_METADATA.get(char_id, HEROINE_METADATA["sora"])
	var dir_sign: float = float(meta.get("dir", 1.0))

	_fallback_name_lbl.text = str(meta.get("name_cn", char_id))
	_fallback_romaji_lbl.text = str(meta.get("name_romaji", ""))
	_fallback_motto_lbl.text = str(meta.get("motto", ""))

	var raw_layers = intro.get("layers", [])
	var has_all_5 := false
	if raw_layers is Array and raw_layers.size() >= 5:
		var ok_count := 0
		for idx in range(5):
			var t = raw_layers[idx]
			if t != null and t is Texture2D:
				_layers[idx].texture = t
				ok_count += 1
			else:
				_layers[idx].texture = null
		has_all_5 = (ok_count >= 4 and _layers[3].texture != null)

	_is_playing = true
	_play_start_frame = Engine.get_process_frames()
	visible = true
	move_to_front()

	if has_all_5:
		_fallback_panel.visible = false
		# 原版 ec_*_01 为半透明白雾层，隐藏实色白底以还原舞台背景微透视觉效果
		_white_backdrop.visible = false
		for i in range(5):
			_layers[i].visible = (_layers[i].texture != null)

		# 精确初始姿态与滑入偏移：03 彩色剪影与 04 主立绘从左右反方向交错飞入至 -314±60 再缓动归位 -314
		var x03_start := -314.0 + 1500.0 * dir_sign
		var x03_cross := -314.0 + 60.0 * dir_sign
		var x04_start := -314.0 - 1500.0 * dir_sign
		var x04_cross := -314.0 - 60.0 * dir_sign

		if headless_instant:
			modulate.a = 1.0
			for i in range(5):
				_layers[i].modulate.a = 1.0
			_layers[2].position = Vector2(-314.0, -36.0)
			_layers[3].position = Vector2(-314.0, -36.0)
			_is_playing = false
			return

		modulate.a = 0.0
		_layers[0].position = Vector2(-64.0, -36.0)
		_layers[1].position = Vector2(-64.0, -36.0)
		_layers[2].position = Vector2(x03_start, -36.0)
		_layers[2].modulate.a = 1.0
		_layers[3].position = Vector2(x04_start, -36.0)
		_layers[3].modulate.a = 1.0
		_layers[4].position = Vector2(-64.0, -36.0)
		_layers[4].modulate.a = 0.0

		var se_stream = intro.get("se_stream")
		if se_stream != null and se_stream is AudioStream and se_player.is_inside_tree():
			se_player.stream = se_stream
			se_player.play()

		var jingle = intro.get("jingle_stream")
		if jingle != null and jingle is AudioStream and jingle_player.is_inside_tree():
			jingle_player.stream = jingle
			jingle_player.play()

		_tween = create_tween()
		# Phase 1 (0..0.25s): 白雾与波点背景显现
		_tween.tween_property(self, "modulate:a", 1.0, 0.25)
		# Phase 2 (0.25..0.70s): 03 彩色剪影与 04 人物立绘反向飞入至交错缓冲点 (-314 ± 60)
		_tween.chain()
		_tween.tween_property(_layers[2], "position:x", x03_cross, 0.45).set_trans(Tween.TRANS_CUBIC).set_ease(Tween.EASE_OUT)
		_tween.parallel().tween_property(_layers[3], "position:x", x04_cross, 0.45).set_trans(Tween.TRANS_CUBIC).set_ease(Tween.EASE_OUT)
		# Phase 3 (0.70..3.10s): 05 人物名字标语渐显 + 03/04 持续 2.4s 缓慢对向平移停靠在精确 (-314, -36)
		_tween.chain()
		_tween.tween_property(_layers[4], "modulate:a", 1.0, 0.35)
		_tween.parallel().tween_property(_layers[2], "position:x", -314.0, 2.4).set_trans(Tween.TRANS_SINE).set_ease(Tween.EASE_OUT)
		_tween.parallel().tween_property(_layers[3], "position:x", -314.0, 2.4).set_trans(Tween.TRANS_SINE).set_ease(Tween.EASE_OUT)
		# Phase 4 (3.10..4.85s): 展示停靠完整画面后淡出进入该角色首句台词
		_tween.chain()
		_tween.tween_interval(1.3)
		_tween.tween_property(self, "modulate:a", 0.0, 0.45)
		_tween.tween_callback(_on_intro_completed)
	else:
		# 回退：使用单张 card_texture 展示
		for l in _layers:
			l.visible = false
		_white_backdrop.visible = false
		_fallback_panel.visible = true
		var card_tex = intro.get("card_texture")
		_fallback_card_rect.texture = card_tex if card_tex is Texture2D else null

		if headless_instant:
			modulate.a = 1.0
			_is_playing = false
			return

		modulate.a = 0.0
		var jingle = intro.get("jingle_stream")
		if jingle != null and jingle is AudioStream and jingle_player.is_inside_tree():
			jingle_player.stream = jingle
			jingle_player.play()
		_tween = create_tween()
		_tween.tween_property(self, "modulate:a", 1.0, 0.35)
		_tween.tween_interval(2.6)
		_tween.tween_property(self, "modulate:a", 0.0, 0.4)
		_tween.tween_callback(_on_intro_completed)

func get_layer(idx_1_based: int) -> TextureRect:
	_build_nodes()
	if idx_1_based >= 1 and idx_1_based <= _layers.size():
		return _layers[idx_1_based - 1]
	return null

func get_fallback_labels() -> Dictionary:
	_build_nodes()
	return {
		"panel": _fallback_panel,
		"card_rect": _fallback_card_rect,
		"name": _fallback_name_lbl,
		"romaji": _fallback_romaji_lbl,
		"motto": _fallback_motto_lbl,
		"tag": _fallback_tag_lbl,
	}

func skip() -> void:
	if not _is_playing and not visible:
		return
	_on_intro_completed()

func _on_intro_completed() -> void:
	if _tween and _tween.is_valid():
		_tween.kill()
	_tween = null
	if se_player and se_player.is_playing():
		se_player.stop()
	if jingle_player and jingle_player.is_playing():
		jingle_player.stop()
	_is_playing = false
	visible = false
	modulate.a = 0.0
	intro_finished.emit()

func _gui_input(event: InputEvent) -> void:
	if event.device == InputEvent.DEVICE_ID_EMULATION:
		return
	if Engine.get_process_frames() <= _play_start_frame:
		return
	if visible and (
		(event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_LEFT)
		or (event is InputEventScreenTouch and event.pressed)
	):
		skip()
		accept_event()
