# verify_stage5.gd
# 阶段五表现层装配与 Windows 1:1 热调试自动化验收脚本 (Headless 验证)

extends SceneTree

const Guard = preload("verify_guard.gd")

func _check(cond: bool, msg: String = "校验失败") -> bool:
	return Guard.check(self, cond, msg)

func _init():
	Guard.install_watchdog(self, 45.0, "verify_stage5")
	print("======================================================================")
	print("  [验收套件] 《PURE×CONNECT》阶段五: 表现层装配与 GDExtension 集成测试")
	print("======================================================================")

	# 1. 验证 GDExtension 原生类注册
	print("[Check 1/9] 检查 PureConnectEngine ClassDB 注册...")
	if not _check(ClassDB.class_exists("PureConnectEngine"), "PureConnectEngine 必须存在于 ClassDB 中"): return
	print("   -> OK: PureConnectEngine 类已注册")

	# 2. 验证场景资源加载
	print("[Check 2/9] 检查 Main.tscn 资源加载...")
	var main_packed = load("res://Main.tscn")
	if not _check(main_packed != null, "Main.tscn 必须能够成功加载"): return
	if not _check(main_packed is PackedScene, "Main.tscn 必须为合法的 PackedScene"): return
	print("   -> OK: Main.tscn 载入成功")

	# 3. 实例化场景并挂载
	print("[Check 3/9] 实例化 Main.tscn 并检查节点树结构...")
	var main_node = main_packed.instantiate()
	if not _check(main_node != null, "Main 场景实例化不能为 null"): return
	root.add_child(main_node)

	# 4. 严格断言所有规范要求的舞台节点
	print("[Check 4/9] 验证表现层所有规范节点的存在性与类型...")
	var bg = main_node.get_node_or_null("Background")
	if not _check(bg is TextureRect, "Background 节点必须存在且为 TextureRect"): return

	var char_slots = main_node.get_node_or_null("CharacterSlots")
	if not _check(char_slots is Control, "CharacterSlots 节点必须存在且为 Control"): return

	for i in range(1, 11):
		var slot_node = char_slots.get_node_or_null("Slot" + str(i))
		if not _check(slot_node is TextureRect, "Slot%d 必须存在且为 TextureRect" % i): return

	var dlg_box = main_node.get_node_or_null("DialogueBox")
	if not _check(dlg_box is PanelContainer, "DialogueBox 节点必须存在且为 PanelContainer"): return

	var spk_label = main_node.find_child("SpeakerLabel", true, false)
	if not _check(spk_label is Label, "SpeakerLabel 节点必须存在且为 Label"): return

	var txt_label = main_node.find_child("TextLabel", true, false)
	if not _check(txt_label is RichTextLabel, "TextLabel 节点必须存在且为 RichTextLabel"): return

	var choice_box = main_node.get_node_or_null("ChoiceBox")
	if not _check(choice_box is PanelContainer, "ChoiceBox 节点必须存在且为 PanelContainer"): return

	var voice_player = main_node.get_node_or_null("VoicePlayer")
	if not _check(voice_player is AudioStreamPlayer, "VoicePlayer 节点必须存在且为 AudioStreamPlayer"): return

	var bgm_player = main_node.get_node_or_null("BgmPlayer")
	if not _check(bgm_player is AudioStreamPlayer, "BgmPlayer 节点必须存在且为 AudioStreamPlayer"): return

	print("   -> OK: Background, CharacterSlots(1~10), DialogueBox, SpeakerLabel, TextLabel, ChoiceBox, VoicePlayer, BgmPlayer 全部合规！")

	# 5. 验证驱动脚本初始化与 VFS 挂载
	print("[Check 5/9] 验证 main.gd 驱动脚本生命周期与 VFS 挂载...")
	if main_node.engine == null:
		main_node._ready()

	if not _check(main_node.engine != null, "main_node.engine 必须已成功实例化 PureConnectEngine"): return
	if not _check(main_node.slots.size() == 10, "main_node.slots 必须映射全部 10 个立绘槽位"): return
	print("   -> OK: 驱动脚本生命周期执行完毕，引擎状态: ", main_node.engine.get_state())

	# 6. 验证首句对白渲染与增量快照落地
	print("[Check 6/9] 验证首句台词渲染、背景图片与说话人...")
	if not _check(not txt_label.text.is_empty(), "首句台词正文不能为空"): return
	if not _check(bg.texture != null, "首句背景纹理必须成功赋值"): return
	print("   -> 初始说话人: [%s]" % spk_label.text)
	print("   -> 初始台词: %s" % txt_label.text)
	print("   -> 初始背景纹理: %s" % bg.texture)

	# 7. 连续推进 10 句台词验证稳定性
	print("[Check 7/9] 模拟连续推进 10 句台词并验证状态机连续性...")
	for step in range(1, 11):
		main_node.advance_dialogue()
		if not _check(not txt_label.text.is_empty(), "第 %d 步台词不能为空" % step): return
	print("   -> OK: 连续 10 步流式推进顺畅")

	# 8. 验证用户输入事件响应 (鼠标点击与空格键推进)
	print("[Check 8/9] 验证用户输入事件响应 (_gui_input 与 _unhandled_input)...")
	var text_before_click = txt_label.text
	var mouse_ev = InputEventMouseButton.new()
	mouse_ev.button_index = MOUSE_BUTTON_LEFT
	mouse_ev.pressed = true
	main_node._gui_input(mouse_ev)
	if not _check(txt_label.text != text_before_click, "鼠标左键点击必须成功推进台词"): return

	var text_before_key = txt_label.text
	var key_ev = InputEventKey.new()
	key_ev.keycode = KEY_SPACE
	key_ev.pressed = true
	main_node._unhandled_input(key_ev)
	if not _check(txt_label.text != text_before_key, "空格键按下必须成功推进台词"): return
	print("   -> OK: 鼠标与键盘按键输入响应推进全部正常")

	# 9. 验证分支选择弹窗 UI 状态机与选项渲染
	print("[Check 9/9] 验证 WaitingForChoice 分支选择弹窗与状态恢复...")
	var mock_choice_snapshot = {
		"state": "WaitingForChoice",
		"choice_prompt": "要去哪里？",
		"choice_options": ["去休息室", "去事务所"],
		"changed_slots": {},
		"cleared_slots": [],
	}
	main_node._apply_snapshot(mock_choice_snapshot)

	if not _check(choice_box.visible, "遇到 WaitingForChoice 时 ChoiceBox 必须弹出显示"): return
	if not _check(main_node._is_waiting_for_choice, "_is_waiting_for_choice 必须被置为 true"): return
	var options_box = choice_box.find_child("OptionsContainer", true, false)
	if not _check(options_box != null, "OptionsContainer 必须存在"): return
	if not _check(options_box.get_child_count() == 2, "分支选项按钮数量必须为 2"): return

	var btn0 = options_box.get_child(0)
	if not _check(btn0 is Button and btn0.text == "去休息室", "第 1 个选项必须为'去休息室'"): return
	var btn1 = options_box.get_child(1)
	if not _check(btn1 is Button and btn1.text == "去事务所", "第 2 个选项必须为'去事务所'"): return

	# 验证在 WaitingForChoice 期间普通点击不应推进台词
	var text_in_choice = txt_label.text
	main_node._gui_input(mouse_ev)
	if not _check(txt_label.text == text_in_choice, "分支选择期间普通点击绝不能推进台词"): return

	print("   -> OK: ChoiceBox 弹窗、选项按钮生成与交互拦截全部合规！")

	print("======================================================================")
	print("  [验收结论] 阶段五全部表现层节点装配与 1:1 热调试验证 100% 通过！")
	print("======================================================================\n")

	main_node.free()
	quit()
