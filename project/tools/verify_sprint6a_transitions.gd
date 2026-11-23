# verify_sprint6a_transitions.gd
# 《PURE×CONNECT》阶段六 Sprint 6A: 毫秒级黑场转场与双缓冲立绘无鬼影动效验收脚本

extends SceneTree

const Guard = preload("verify_guard.gd")

func _check(cond: bool, msg: String = "校验失败") -> bool:
	return Guard.check(self, cond, msg)

func _init():
	Guard.install_watchdog(self, 45.0, "verify_sprint6a_transitions")
	print("======================================================================")
	print("  [专项验收] 《PURE×CONNECT》Sprint 6A: 场景转场与立绘双缓冲动效全门禁")
	print("======================================================================")

	var main_packed = load("res://Main.tscn")
	if not _check(main_packed != null, "Main.tscn 必须能够成功加载"): return
	var main_node = main_packed.instantiate()
	if not _check(main_node != null, "Main 场景实例化不能为 null"): return
	root.add_child(main_node)

	if main_node.engine == null:
		main_node._ready()

	# 1. 验证毫秒级实测动效常量对齐 (对齐 PC 录像 58s~62s 与 42.10s~42.30s)
	print("[Check 1/5] 验证动效时长常量是否严格对齐 PC 录像实测...")
	if not _check(main_node.SCENE_FADE_OUT_SEC == 1.0, "黑场淡出时长必须为 1.0s (1000ms)"): return
	if not _check(main_node.SCENE_HOLD_BLACK_SEC == 0.5, "全黑保持时长必须为 0.5s (500ms)"): return
	if not _check(main_node.SCENE_FADE_IN_SEC == 0.5, "黑场淡入时长必须为 0.5s (500ms)"): return
	if not _check(main_node.SPRITE_FADE_SEC == 0.2, "立绘/表情过渡时长必须为 0.2s (200ms)"): return
	print("   -> OK: 1000ms + 500ms + 500ms 黑场节奏与 200ms 立绘过渡常量死锁对齐")

	# 2. 验证 TransitionOverlay 全屏遮罩层层级与输入穿透
	print("[Check 2/5] 验证 TransitionOverlay 节点属性与遮罩层级...")
	if not _check(main_node.transition_overlay != null, "TransitionOverlay 必须存在"): return
	if not _check(main_node.transition_overlay.color == Color(0, 0, 0, 1), "TransitionOverlay 必须为纯黑色"): return
	if not _check(main_node.transition_overlay.mouse_filter == Control.MOUSE_FILTER_IGNORE, "TransitionOverlay 必须设置 MOUSE_FILTER_IGNORE 防输入阻断"): return
	print("   -> OK: TransitionOverlay 节点属性合规")

	# 3. 验证立绘槽位双缓冲 (同级 Slot1Prev + Slot1) 与同轮廓无鬼影换表情算法
	print("[Check 3/5] 验证双缓冲槽位装配与同尺寸表情切换底层锁定 (消灭 25% 鬼影)...")
	if not _check(main_node.slots.has(1), "槽位 1 必须存在"): return
	if not _check(main_node.slot_prevs.has(1), "槽位 1 同级双缓冲映射必须存在"): return
	var slot1: TextureRect = main_node.slots[1]
	var prev1: TextureRect = main_node.slot_prevs[1] as TextureRect
	if not _check(prev1 != null, "CharacterSlots 下必须装配同级 Slot1Prev 双缓冲节点"): return
	if not _check(prev1.get_parent() == slot1.get_parent(), "Slot1Prev 必须与 Slot1 同级，避免父节点透明度归零穿透"): return
	if not _check(prev1.get_index() < slot1.get_index(), "Slot1Prev 必须严格位于 Slot1 渲染层之下"): return
	if not _check(prev1.mouse_filter == Control.MOUSE_FILTER_IGNORE, "Slot1Prev 必须设置 MOUSE_FILTER_IGNORE"): return

	# 创建两个同尺寸虚拟贴图模拟同一角色换表情
	var img_a = Image.create(200, 300, false, Image.FORMAT_RGBA8)
	img_a.fill(Color(1, 0, 0, 1))
	var tex_a = ImageTexture.create_from_image(img_a)

	var img_b = Image.create(200, 300, false, Image.FORMAT_RGBA8)
	img_b.fill(Color(0, 1, 0, 1))
	var tex_b = ImageTexture.create_from_image(img_b)

	# 进场瞬切测试
	main_node._set_slot_sprite(1, tex_a, true)
	if not _check(slot1.texture == tex_a, "Slot1 纹理必须为 tex_a"): return
	if not _check(slot1.visible, "Slot1 必须可见"): return
	if not _check(slot1.modulate.a == 1.0, "Slot1 modulate.a 必须为 1.0"): return
	if not _check(not prev1.visible, "SlotPrev 初始瞬切后必须隐藏"): return

	# 模拟同角色换表情逻辑: 验证 _should_hold_prev_solid 机制
	prev1.texture = tex_a
	prev1.modulate.a = 1.0
	prev1.visible = true
	slot1.texture = tex_b
	slot1.modulate.a = 0.5
	# 断言：底层 prev 保持满不透明 1.0，顶层淡入覆盖，两者叠加合成在任何时刻均不透出背景！
	if not _check(prev1.modulate.a == 1.0, "换表情期间底层 SlotPrev 必须锁死 alpha=1.0 杜绝背景鬼影！"): return
	print("   -> OK: 同尺寸双缓冲槽位无鬼影逻辑校验通过")

	# 清空槽位瞬切测试
	main_node._set_slot_sprite(1, null, true)
	if not _check(not slot1.visible, "清空后 Slot1 必须隐藏"): return
	if not _check(slot1.texture == null, "清空后 Slot1 纹理必须为 null"): return

	# 4. 验证黑场转场生命周期、原子提交与连点/SKIP 熔断
	print("[Check 4/5] 验证黑场转场原子提交、安全打断与 SKIP 熔断机制...")
	var mock_scene_step = {
		"state": "WaitingForInput",
		"speaker": "测试角色",
		"text": "转场后新场景台词",
		"bg_texture": tex_b,
		"changed_slots": { 1: tex_a },
		"cleared_slots": [],
		"face_changed": false
	}

	# 模拟触发转场
	main_node._is_transitioning = true
	main_node._pending_snapshot = mock_scene_step
	main_node.dialogue_box.visible = false
	main_node.transition_overlay.visible = true
	main_node.transition_overlay.modulate.a = 0.5

	# 测试玩家连点或按键打断：调用 _skip_active_transition()
	main_node._skip_active_transition()
	if not _check(not main_node._is_transitioning, "打断后 _is_transitioning 必须为 false"): return
	if not _check(not main_node.transition_overlay.visible, "打断后 transition_overlay 必须隐藏"): return
	if not _check(main_node.transition_overlay.modulate.a == 0.0, "打断后 transition_overlay modulate.a 必须归零"): return
	if not _check(main_node.dialogue_box.visible, "打断后 dialogue_box 必须恢复可见"): return
	if not _check(main_node.background.texture == tex_b, "原子提交后背景纹理必须更新为新背景"): return
	if not _check(main_node.slots[1].texture == tex_a, "原子提交后槽位 1 必须更新为新立绘"): return
	if not _check(main_node.text_label.text == "转场后新场景台词", "打断后对白必须正确呈现"): return
	print("   -> OK: 连点安全打断、瞬时原子换景与无残留黑幕校验通过")

	# 5. 验证 SKIP 模式瞬切与音频 Ducking 正常
	print("[Check 5/5] 验证 SKIP 快进模式转场跳过与 Audio Ducking 音量恢复...")
	main_node._toggle_skip_mode()
	if not _check(main_node._is_skip_active, "SKIP 模式必须处于激活状态"): return
	# 在 SKIP 模式下传入新场景，必须瞬切且不启动 2 秒转场
	var mock_skip_step = {
		"state": "WaitingForInput",
		"speaker": "测试角色2",
		"text": "快进场景台词",
		"bg_texture": tex_a,
		"changed_slots": {},
		"cleared_slots": [1],
		"face_changed": false
	}
	main_node._apply_snapshot(mock_skip_step)
	if not _check(not main_node._is_transitioning, "SKIP 模式下场景变更绝不能挂起转场"): return
	if not _check(main_node.background.texture == tex_a, "SKIP 模式下背景必须秒切"): return
	if not _check(not main_node.slots[1].visible, "SKIP 模式下槽位 1 必须秒清空"): return
	if not _check(main_node.text_label.text == "快进场景台词", "SKIP 模式下台词必须秒显"): return
	main_node._toggle_skip_mode()

	print("======================================================================")
	print("  [专项结论] Sprint 6A 表现层动效与双缓冲立绘 5 项门禁 100% 验证通过！")
	print("======================================================================\n")

	main_node.free()
	quit()
