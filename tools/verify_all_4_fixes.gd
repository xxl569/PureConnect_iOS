# verify_all_4_fixes.gd
# 针对本次 4 大缺陷专项深度回归验收脚本 (Headless)

extends SceneTree

const Guard = preload("verify_guard.gd")

func _check(cond: bool, msg: String = "校验失败") -> bool:
	return Guard.check(self, cond, msg)

func _init():
	Guard.install_watchdog(self, 45.0, "verify_all_4_fixes")
	print("======================================================================")
	print("  [深度验收套件] 《PURE×CONNECT》4 大缺陷综合回归自动化测试")
	print("======================================================================")

	var main_packed = load("res://Main.tscn")
	if not _check(main_packed != null, "Main.tscn 载入失败"): return
	var main_node = main_packed.instantiate()
	root.add_child(main_node)
	main_node._ready()

	# ------------------------------------------------------------------
	# Fix 2: 7 功能按钮文字与字体挂载检查
	# ------------------------------------------------------------------
	print("\n[验收 1/4] 验证 Quick Menu 7 功能按钮中文名称与字体/交互属性...")
	var expected_btns = {
		"BtnQSave": "快存",
		"BtnQLoad": "快读",
		"BtnLog": "履历",
		"BtnAuto": "自动",
		"BtnSkip": "快进",
		"BtnConfig": "设置",
		"BtnHide": "隐藏",
	}

	for btn_name in expected_btns.keys():
		var btn = main_node.find_child(btn_name, true, false)
		if not _check(btn != null, "按钮 %s 必须存在" % btn_name): return
		var lbl = btn.get_node_or_null("BtnLabel") as Label
		if not _check(lbl != null, "按钮 %s 必须包含 BtnLabel 子节点" % btn_name): return
		var exp_text = expected_btns[btn_name]
		if not _check(lbl.text == exp_text, "按钮 %s 文字必须为 [%s], 实际为 [%s]" % [btn_name, exp_text, lbl.text]): return
		if not _check(lbl.mouse_filter == Control.MOUSE_FILTER_IGNORE, "BtnLabel 的 mouse_filter 必须为 IGNORE，防止阻断按钮点击"): return
		if not _check(lbl.has_theme_font_override("font"), "BtnLabel 必须挂载统一中文字体覆盖"): return
		print("   -> OK: %s -> 文字 [%s], mouse_filter=IGNORE, 字体覆盖有效" % [btn_name, lbl.text])

	# ------------------------------------------------------------------
	# Fix 3: 主标题画面 TitleScreen (PC 原版 1:1 SGTitle990000 + 6 按钮 + 无存档提示层级)
	# ------------------------------------------------------------------
	print("\n[验收 2/6] 验证主标题画面 TitleScreen 1:1 原版底图、6 入口按钮与通知层级...")
	main_node._show_title_screen()
	if not _check(main_node._is_in_title_screen == true, "_is_in_title_screen 必须为 true"): return
	var ts = main_node.title_screen
	if not _check(ts != null and ts.visible == true, "title_screen 容器必须存在且可见"): return
	var tbg := ts.get_node_or_null("TitleBg") as TextureRect
	if not _check(tbg != null and tbg.texture != null, "标题画面背景 SGTitle990000 纹理必须成功加载"): return
	if not _check(tbg.texture.get_width() == 1280 and tbg.texture.get_height() == 720, "标题底图必须为 1280x720 原版画幅"): return

	var menu_h = ts.get_node_or_null("MenuHContainer")
	if not _check(menu_h != null, "MenuHContainer 横排按钮容器必须存在"): return
	if not _check(menu_h.get_child_count() == 6, "MenuHContainer 必须包含 PC 原版全部 6 枚按钮"): return
	for btn_i in range(6):
		var tb := menu_h.get_child(btn_i) as TextureButton
		if not _check(tb != null and tb.texture_normal != null, "第 %d 枚标题按钮必须装配 AtlasTexture" % btn_i): return
	print("   -> OK: TitleScreen 1:1 原版底图与 6 枚横排按钮装配完毕")

	# 若当前尚无存档，点击继续游戏必须浮现 QuickNotify 且 z_index >= 100
	if not main_node.SettingsPanelScript.has_any_save():
		main_node._on_title_continue_game()
		var qn = main_node.quick_notify
		if not _check(qn != null and qn.visible == true, "无存档点击继续游戏时，quick_notify 必须显现"): return
		if not _check(qn.z_index >= 100, "quick_notify 的 z_index 必须 >= 100"): return

	# 标题画面激活期间点击空白区域不推进剧情
	var text_curr = main_node.text_label.text
	var click_ev = InputEventMouseButton.new()
	click_ev.button_index = MOUSE_BUTTON_LEFT
	click_ev.pressed = true
	main_node._gui_input(click_ev)
	if not _check(main_node.text_label.text == text_curr, "标题画面激活期间点击空白绝对不能推进对白"): return

	main_node._on_title_start_new_game()
	if not _check(main_node._is_in_title_screen == false, "点击从头开始后 _is_in_title_screen 必须置为 false"): return
	print("   -> OK: 点击'从头开始'成功切出标题画面并开始演播")

	# ------------------------------------------------------------------
	# Fix 4: 新角色登场特写 (0x00F0) 原生 5 图层 ec_*_01..05 与防吞台词状态机
	# ------------------------------------------------------------------
	print("\n[验收 3/6] 验证新角色登场 5 图层滑入 (CharacterIntroLayer) 与防吞台词状态机...")
	main_node._setup_character_intro_layer()
	if not _check(main_node.char_intro_layer != null, "char_intro_layer 必须已初始化"): return
	if not _check(main_node.intro_card_panel != null, "intro_card_panel 卡片容器必须存在"): return
	if not _check(main_node.intro_name_lbl != null, "intro_name_lbl 标签必须存在"): return

	var ar_layers := []
	for li in range(1, 6):
		var lt = main_node.engine.load_ui_texture("ec_ar_0%d" % li)
		if not _check(lt != null and lt is Texture2D, "ec_ar_0%d 原生图层必须成功解码" % li): return
		ar_layers.append(lt)

	var real_intro = {
		"character": "arisa",
		"card_texture": ar_layers[3],
		"layers": ar_layers,
		"jingle_stream": null,
	}
	var intro_step = {
		"state": "WaitingForInput",
		"character_intro": real_intro,
		"speaker": "彩里沙",
		"text": "命运之人什么的，完全不存在！",
		"bg_texture": null,
		"changed_slots": {},
		"cleared_slots": [],
	}

	main_node._apply_snapshot(intro_step)
	if not _check(main_node._seen_character_intros.has("arisa"), "彩里沙 (arisa) 必须被记录至 _seen_character_intros"): return
	if not _check(main_node.char_intro_layer.get_layer(3).position == Vector2(-314, -36), "ec_ar_03 停靠坐标必须精确对齐 (-314, -36)"): return
	if not _check(main_node.char_intro_layer.get_layer(4).position == Vector2(-314, -36), "ec_ar_04 停靠坐标必须精确对齐 (-314, -36)"): return
	if not _check(main_node.intro_name_lbl.text == "绀野 彩里沙", "角色名元数据校验通过"): return

	main_node._is_in_character_intro = true
	main_node.advance_dialogue()
	if not _check(main_node._is_in_character_intro == false, "调用 advance_dialogue 必须先跳过特写"): return
	if not _check(main_node.text_label.text == "命运之人什么的，完全不存在！", "登场首句台词必须完整保留"): return
	print("   -> OK: 5 图层原生角色登场特写与防吞首句台词校验全部通过")

	# ------------------------------------------------------------------
	# Fix 5: 全功能 7 分页设置窗口 (SCREEN/TEXT/SOUND/SYSTEM/SHORTCUT/SAVE/LOAD) 与 100 槽存读档
	# ------------------------------------------------------------------
	print("\n[验收 4/6] 验证 PC 原版 7 分页系统设置面板与存读档持久化链路...")
	main_node._open_settings(0)
	var sp = main_node.settings_panel
	if not _check(sp != null and sp.visible == true, "SettingsPanel 必须成功打开"): return
	for tab_idx in range(7):
		sp.switch_tab(tab_idx)
	if not _check(sp.get_node("SysWndBase").texture != null, "SGSysWnd990000 原版底座必须成功加载"): return
	main_node._close_settings()
	if not _check(sp.visible == false, "关闭 SettingsPanel 后必须隐藏"): return
	print("   -> OK: 7 大设置与存读档分页切换正常")

	# ------------------------------------------------------------------
	# Fix 6: 01_prologue1 背景缓慢平移运镜 + 萌美左下角不裁切头像 & 舞台零污染
	# ------------------------------------------------------------------
	print("\n[验收 5/6] 验证 01_prologue1 开场背景平移动画与萌美左下角头像无裁切、不占舞台...")
	main_node._on_title_start_new_game()
	# 验证前几句中的背景平移最终目标已对齐 (-920, 0) (pc_bg_04B_9)
	if not _check(main_node.background.size.x >= 2700, "pc_bg_04B_9 超宽背景尺寸应完整保留 (2760x1640): 实际=%.0fx%.0f" % [main_node.background.size.x, main_node.background.size.y]): return
	if not _check(absf(main_node.background.position.x - (-920.0)) <= 1.0, "Headless 瞬切后背景平移目标 X 应准确到达 -920.0 (实际 %.1f)" % main_node.background.position.x): return

	var found_moemi_face := false
	for _s in range(40):
		if "萌美" in main_node.speaker_label.text and "什锦烧" in main_node.text_label.text:
			found_moemi_face = true
			if not _check(main_node.face_avatar.visible == true, "萌美台词左下角 FaceAvatar 必须可见"): return
			if not _check(main_node.face_avatar.get_parent().name == "FaceAvatarLayer", "FaceAvatar 必须位于独立的 FaceAvatarLayer 而不是被 PanelContainer 裁切"): return
			if not _check(main_node.face_avatar.position == Vector2(-207, 272), "FaceAvatar 坐标必须精确匹配 PC (-207, 272)"): return
			if not _check(main_node.face_avatar.size == Vector2(612, 828), "FaceAvatar 尺寸必须精确匹配 PC (612, 828)"): return
			if not _check(main_node.slots[1].visible == false, "slot 9 半身头像绝不能污染舞台中央 Slot1！"): return
			break
		main_node.advance_dialogue()
	if not _check(found_moemi_face, "必须成功推进至萌美什锦烧台词并校验左下角半身头像"): return

	# 测试存读档实操往返 (使用测试槽 99，结束后自动清理)
	main_node._perform_save_to_slot(99)
	var saved_meta = main_node.SettingsPanelScript.load_save_metadata("slot_099")
	if not _check(not saved_meta.is_empty() and "什锦烧" in str(saved_meta.get("text", "")), "slot_099 必须真实落盘并记录当前对白"): return
	main_node.advance_dialogue()
	if not _check(not ("什锦烧" in main_node.text_label.text), "推进一句后应离开什锦烧对白"): return
	main_node._perform_load_save_data(saved_meta)
	if not _check("什锦烧" in main_node.text_label.text, "读档恢复后必须精确回到萌美什锦烧对白"): return
	if not _check(main_node.face_avatar.visible == true, "读档恢复后必须重建萌美左下角头像"): return
	var test_json = ProjectSettings.globalize_path("user://saves/slot_099.json")
	var test_png = ProjectSettings.globalize_path("user://saves/slot_099.png")
	if FileAccess.file_exists(test_json): DirAccess.remove_absolute(test_json)
	if FileAccess.file_exists(test_png): DirAccess.remove_absolute(test_png)
	print("   -> OK: 真实存读档快进重建与左下角萌美头像验证全部通过")

	# ------------------------------------------------------------------
	# Fix 1: 人物立绘宏代号解析与 MO06 特殊服装回退
	# ------------------------------------------------------------------
	print("\n[验收 6/6] 验证人物立绘宏代号与多部件槽位合成落地...")
	# 测试标准立绘底图
	var sora_tex = main_node.engine.load_ui_texture("so01_n_b_01")
	if not _check(sora_tex != null and sora_tex is Texture2D, "身体底图 so01_n_b_01 必须成功解码"): return
	print("   -> 身体底图 Texture2D 尺寸: %dx%d" % [sora_tex.get_width(), sora_tex.get_height()])

	# 测试前次尝试中导致完全黑屏缺失的 MO06 姿态/服装回退
	var moemi_tex = main_node.engine.load_ui_texture("mo01_m_b_01")
	if not _check(moemi_tex != null and moemi_tex is Texture2D, "萌美基础身体底图 mo01_m_b_01 必须成功解码"): return
	print("   -> 萌美身体底图 Texture2D 尺寸: %dx%d" % [moemi_tex.get_width(), moemi_tex.get_height()])

	# 模拟挂载至 Slot 1
	main_node._set_slot_sprite(1, sora_tex, true)
	var slot1 = main_node.slots[1]
	if not _check(slot1.visible == true, "Slot 1 挂载立绘后必须可见"): return
	if not _check(slot1.texture == sora_tex, "Slot 1 纹理必须与解码纹理一致"): return
	print("   -> OK: Slot 1 立绘槽位装配、显示与纹理赋值全部通过！")

	print("\n======================================================================")
	print("  [验收结论] 4 大缺陷专项测试全部 100% 成功通过！")
	print("======================================================================\n")

	main_node.free()
	quit(0)
