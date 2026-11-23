# sprite_slot.gd
# 《PURE×CONNECT》双层立绘槽位独立组件
# 通用于舞台 1~10 号立绘槽位与左下角 FaceAvatar 半身像槽位，
# 自动处理进场、退场、同轮廓无鬼影换表情、异轮廓延迟交叉淡化、连点打断以及 SKIP/黑场原子秒切。

class_name SpriteSlot
extends Control

## 默认立绘过渡时长（对齐 PC 录像实测的 200ms）
@export var default_fade_sec: float = 0.2
@export var texture_expand_mode: TextureRect.ExpandMode = TextureRect.EXPAND_IGNORE_SIZE
@export var texture_stretch_mode: TextureRect.StretchMode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED

var _prev_rect: TextureRect
var _curr_rect: TextureRect
var _tween: Tween = null

## 逻辑态跟踪（防止连点打断 Tween 时状态漂移）
var _target_texture: Texture2D = null
var _target_pose_key: String = ""


func _ready() -> void:
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	_ensure_layers()


## 核心入口：更新槽位贴图
## - new_tex: 新贴图，传 null 表示退场清空
## - duration: 过渡时长（秒），传负值则使用 default_fade_sec
## - instant: true 为无动画瞬切（用于黑场峰值原子换景、SKIP 快进）
## - pose_key: 可选，同角色同底图标识（如 "mo02pb"）。用于精准判定是否属于“仅换表情差分”
func set_sprite(
	new_tex: Texture2D,
	duration: float = -1.0,
	instant: bool = false,
	pose_key: String = ""
) -> void:
	_ensure_layers()

	var fade_sec := default_fade_sec if duration < 0.0 else duration
	var old_tex := _target_texture
	var old_pose := _target_pose_key

	# 1. 目标纹理未变化：若要求瞬切则强制完结残留动画，否则直接跳过
	if new_tex == old_tex:
		if instant:
			flush_transition()
		return

	_target_texture = new_tex
	_target_pose_key = pose_key if new_tex != null else ""

	# 2. 瞬切条件：显式 instant、时长 <= 0、或节点尚未进入场景树
	if instant or fade_sec <= 0.0 or not is_inside_tree():
		_apply_instant(new_tex)
		return

	# 3. 退场（有图 -> 空槽）：从当前实际透明度平滑淡出，防止半途打断时闪烁
	if new_tex == null:
		_kill_tween()
		_clear_prev()
		if old_tex == null or not _curr_rect.visible:
			_clear_curr()
			return

		_curr_rect.texture = old_tex
		_tween = create_tween()
		_tween.tween_property(_curr_rect, "modulate:a", 0.0, fade_sec)
		_tween.tween_callback(_clear_curr)
		return

	# 4. 进场（空槽 -> 有图）：单层 0.0 -> 1.0 平滑淡入
	if old_tex == null:
		_kill_tween()
		_clear_prev()
		_curr_rect.texture = new_tex
		_curr_rect.modulate.a = 0.0
		_curr_rect.visible = true

		_tween = create_tween()
		_tween.tween_property(_curr_rect, "modulate:a", 1.0, fade_sec)
		return

	# 5. 替换（有图 -> 新图）：区分“同轮廓无鬼影差分”与“异轮廓交叉淡化”
	_kill_tween()

	# 将旧图沉底至 SlotPrev 并锁定满不透明；新图置于顶层 SlotCurr 从 0 淡入
	_prev_rect.texture = old_tex
	_prev_rect.modulate.a = 1.0
	_prev_rect.visible = true

	_curr_rect.texture = new_tex
	_curr_rect.modulate.a = 0.0
	_curr_rect.visible = true

	var hold_prev_solid := _should_hold_prev_solid(old_tex, new_tex, old_pose, pose_key)

	_tween = create_tween()
	if hold_prev_solid:
		# 同角色同底图换表情：底层死锁 alpha=1.0，顶层覆盖淡入，全程 0% 背景透出（无鬼影）
		_tween.tween_property(_curr_rect, "modulate:a", 1.0, fade_sec)
		_tween.tween_callback(_clear_prev)
	else:
		# 换角色或大姿势切换：底层延迟 30%（60ms）后在剩余 70%（140ms）内淡出，顶层全程 200ms 淡入
		var delay_sec := fade_sec * 0.3
		var out_sec := fade_sec * 0.7
		_tween.set_parallel(true)
		_tween.tween_property(_curr_rect, "modulate:a", 1.0, fade_sec)
		_tween.tween_property(_prev_rect, "modulate:a", 0.0, out_sec).set_delay(delay_sec)
		_tween.chain().tween_callback(_clear_prev)


## 快捷退场接口
func clear_sprite(duration: float = -1.0, instant: bool = false) -> void:
	set_sprite(null, duration, instant, "")


## 立即完结进行中的过渡（在 SKIP 启动或黑场强制打断时调用）
func flush_transition() -> void:
	_ensure_layers()
	_apply_instant(_target_texture)


func get_current_texture() -> Texture2D:
	return _target_texture


# ==============================================================================
# 内部工具方法
# ==============================================================================

func _ensure_layers() -> void:
	if _prev_rect != null and _curr_rect != null:
		return

	_prev_rect = get_node_or_null("SlotPrev") as TextureRect
	if _prev_rect == null:
		_prev_rect = _create_layer_rect("SlotPrev")
		add_child(_prev_rect)

	_curr_rect = get_node_or_null("SlotCurr") as TextureRect
	if _curr_rect == null:
		_curr_rect = _create_layer_rect("SlotCurr")
		add_child(_curr_rect)

	# 严守渲染层级：SlotPrev 在下，SlotCurr 在上
	move_child(_prev_rect, 0)
	move_child(_curr_rect, 1)


func _create_layer_rect(layer_name: String) -> TextureRect:
	var rect := TextureRect.new()
	rect.name = layer_name
	rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	rect.expand_mode = texture_expand_mode
	rect.stretch_mode = texture_stretch_mode
	rect.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	rect.visible = false
	rect.modulate.a = 0.0
	return rect


func _should_hold_prev_solid(
	old_tex: Texture2D,
	new_tex: Texture2D,
	old_pose: String,
	new_pose: String
) -> bool:
	# 优先使用 Rust 下发的底图 pose_key（如 "mo02pb"），最精准
	if not old_pose.is_empty() and not new_pose.is_empty():
		return old_pose == new_pose and old_tex.get_size() == new_tex.get_size()
	# 若未传 pose_key，则以纹理像素尺寸一致作为启发式回退
	return old_tex.get_size() == new_tex.get_size()


func _apply_instant(tex: Texture2D) -> void:
	_kill_tween()
	_clear_prev()
	if tex == null:
		_clear_curr()
	else:
		_curr_rect.texture = tex
		_curr_rect.modulate.a = 1.0
		_curr_rect.visible = true


func _kill_tween() -> void:
	if _tween != null and _tween.is_valid():
		_tween.kill()
	_tween = null


func _clear_prev() -> void:
	if _prev_rect != null:
		_prev_rect.visible = false
		_prev_rect.texture = null
		_prev_rect.modulate.a = 0.0


func _clear_curr() -> void:
	if _curr_rect != null:
		_curr_rect.visible = false
		_curr_rect.texture = null
		_curr_rect.modulate.a = 0.0
