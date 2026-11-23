# verify_guard.gd
# Godot 自动化验证脚本共享防挂死守卫（第二防线）
# 1. 禁止使用裸 assert()：校验失败时打印诊断信息并立刻 quit(1)
# 2. 启动时挂独立兜底超时计时器：防止异步循环卡死并自动 quit(124)
extends RefCounted

static func install_watchdog(tree: SceneTree, timeout_sec: float = 45.0, label: String = "") -> void:
	if tree == null:
		return
	var timer := tree.create_timer(timeout_sec, true, false, true)
	timer.timeout.connect(func():
		var tag := (" [%s]" % label) if not label.is_empty() else ""
		printerr("[TIMEOUT]%s 脚本运行超过 %.1fs 未主动退出，触发内部兜底 quit(124)" % [tag, timeout_sec])
		tree.quit(124)
	)

static func check(tree: SceneTree, cond: bool, msg: String) -> bool:
	if cond:
		return true
	printerr("[FAIL] %s" % msg)
	if tree != null:
		tree.quit(1)
	return false
