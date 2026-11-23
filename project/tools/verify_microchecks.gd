# verify_microchecks.gd
# 针对师父指出的阶段五 2 项微检查与交互边界测试

extends SceneTree

const Guard = preload("verify_guard.gd")

func _check(cond: bool, msg: String = "校验失败") -> bool:
	return Guard.check(self, cond, msg)

func _init():
	Guard.install_watchdog(self, 45.0, "verify_microchecks")
	print("--- 开始微检查与边界测试 ---")
	
	# 1. 验证 ProjectSettings
	var w = ProjectSettings.get_setting("display/window/size/viewport_width")
	var h = ProjectSettings.get_setting("display/window/size/viewport_height")
	var stretch_mode = ProjectSettings.get_setting("display/window/stretch/mode")
	var stretch_aspect = ProjectSettings.get_setting("display/window/stretch/aspect")
	
	if not _check(w == 1280 and h == 720, "分辨率必须为 1280x720"): return
	if not _check(stretch_mode == "canvas_items", "stretch/mode 必须为 canvas_items"): return
	if not _check(stretch_aspect == "keep", "stretch/aspect 必须为 keep"): return
	print("[Check 1] 1280x720 canvas_items keep 校验通过")

	# 2. 实例化 Main.tscn
	var main_packed = load("res://Main.tscn")
	var main_node = main_packed.instantiate()
	root.add_child(main_node)
	main_node._ready()

	var txt_label: RichTextLabel = main_node.find_child("TextLabel", true, false)
	var initial_text = txt_label.text

	# 3. 验证 Auto / Skip 在打开 Backlog 时被暂停
	main_node._is_auto_active = true
	main_node._is_skip_active = true
	main_node._open_backlog()
	if not _check(main_node._is_auto_active == false, "打开 Backlog 必须强制暂停 AUTO"): return
	if not _check(main_node._is_skip_active == false, "打开 Backlog 必须强制暂停 SKIP"): return
	if not _check(main_node.backlog_panel.visible == true, "Backlog 必须可见"): return
	print("[Check 2] Backlog 强制暂停 AUTO/SKIP 校验通过")

	# 4. 验证 Backlog 打开时 advance_dialogue 被严格阻断
	main_node.advance_dialogue()
	if not _check(txt_label.text == initial_text, "Backlog 打开时 advance_dialogue 绝不能推进台词"): return
	print("[Check 3] Backlog 打开时推进阻断校验通过")

	# 5. 关闭 Backlog
	main_node._close_backlog()
	if not _check(main_node.backlog_panel.visible == false, "Backlog 已关闭"): return

	# 6. 验证纯净赏析隐藏 UI
	main_node._hide_ui()
	if not _check(main_node._is_ui_hidden == true, "纯净模式 _is_ui_hidden 必须为 true"): return
	if not _check(main_node.dialogue_box.visible == false, "对话框必须隐藏"): return
	
	# 在隐藏模式下调用 advance_dialogue 只唤醒 UI 不跳台词
	main_node.advance_dialogue()
	if not _check(main_node._is_ui_hidden == false, "首次调用必须仅唤醒 UI"): return
	if not _check(main_node.dialogue_box.visible == true, "对话框已恢复显示"): return
	if not _check(txt_label.text == initial_text, "首次唤醒 UI 绝不能跳过当前台词"): return
	print("[Check 4] 纯净赏析隐藏与恢复 UI 防漏看校验通过")

	# 7. 验证通道互斥：_replay_history_voice 重听逻辑
	main_node._replay_history_voice("non_existent_voice")
	if not _check(main_node.voice_player.stream == null, "重听不存在语音时不挂载流"): return
	print("[Check 5] 语音通道互斥校验通过")

	print("--- 全部微检查 100% 通过 ---")
	main_node.free()
	quit()
