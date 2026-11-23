use pure_connect_core::{BgiScript, VirtualFileSystem, VmEngine, VmEvent, VmState};
use std::path::Path;

#[test]
fn test_vm_execution_on_real_translated_script() {
    let mut vfs = VirtualFileSystem::new();
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");

    if !patch_dir.is_dir() {
        eprintln!("汉化补丁解包目录不存在，跳过真实剧本端到端集成测试");
        return;
    }

    vfs.mount_directory(patch_dir).expect("挂载汉化目录失败");
    let script_bytes = vfs.read("05_chapter1_2").expect("无法读取 05_chapter1_2 剧本");

    // 1. 验证剧本头部解析
    let script = BgiScript::parse(&script_bytes).expect("BGI 剧本头解析失败");
    assert_eq!(script.base_offset(), 0x40, "代码基址偏移不匹配");
    assert_eq!(script.header_size(), 0x24, "头部大小不匹配");
    println!(
        "  [OK] 剧本 05_chapter1_2 头解析成功！依赖模块: {:?}",
        script.modules()
    );

    // 2. 初始化虚拟机引擎
    let mut vm = VmEngine::new(&script);
    assert_eq!(*vm.state(), VmState::Ready, "VM 初始状态非 Ready");

    // 3. 连续流式执行前 100 句台词并验证
    let mut dialogues = Vec::with_capacity(100);
    for _ in 0..100 {
        let dlg = vm
            .next_dialogue()
            .expect("VM 流式执行对白错误")
            .expect("未能提取到预期对白");
        dialogues.push(dlg);
    }

    assert_eq!(dialogues.len(), 100, "提取台词总数未达 100 句");
    println!("  [OK] VM 成功连续流式执行 100 句对白！");

    // 全局防假性断点自检：前 100 句台词绝不允许存在任何空白/虚假同步标记
    for (i, d) in dialogues.iter().enumerate() {
        assert!(
            !d.text.trim().is_empty(),
            "第 {} 句台词内容为空！存在将内部视听标记误当对白挂起的 Bug (PC: 0x{:x})",
            i + 1,
            d.pc
        );
    }
    println!("  [OK] 前 100 句台词非空校验全部通过（0 处虚假同步标记挂起）！");

    // 4. 精确断言关键台词元数据
    // 第 1 句：步海语音首发
    let d1 = &dialogues[0];
    assert_eq!(d1.index, 1);
    assert_eq!(d1.speaker.as_deref(), Some("步海"));
    assert_eq!(d1.text, "「哥哥，步海想玩过家家」");
    assert_eq!(d1.voice.as_deref(), Some("AY_0202"));
    assert_eq!(d1.bg.as_deref(), Some("white"));
    assert_eq!(d1.bgm.as_deref(), Some("PC_BGM_17"));

    // 第 2 句：男主角对白（无语音）
    let d2 = &dialogues[1];
    assert_eq!(d2.index, 2);
    assert_eq!(d2.speaker.as_deref(), Some("李骏"));
    assert!(d2.text.contains("好啊，那先分配角色吧"));
    assert!(d2.voice.is_none());

    // 第 3 句：名台词 萩本泰三
    let d3 = &dialogues[2];
    assert_eq!(d3.index, 3);
    assert_eq!(d3.speaker.as_deref(), Some("李骏"));
    assert!(
        d3.text.contains("我是涂装工人萩本泰三，54岁的酒鬼"),
        "第3句台词内容不匹配: {}",
        d3.text
    );
    assert!(d3.voice.is_none());

    // 第 5 句：步海第二句语音
    let d5 = &dialogues[4];
    assert_eq!(d5.index, 5);
    assert_eq!(d5.speaker.as_deref(), Some("步海"));
    assert!(d5.text.contains("那步海要当妈妈！"));
    assert_eq!(d5.voice.as_deref(), Some("AY_0203"));

    // 第 8 句：旁白（无 speaker）
    let d8 = &dialogues[7];
    assert_eq!(d8.index, 8);
    assert!(d8.speaker.is_none(), "旁白不应存在 speaker");
    assert_eq!(d8.text, "……");

    // 第 10 句：步海第三句语音
    let d10 = &dialogues[9];
    assert_eq!(d10.index, 10);
    assert_eq!(d10.speaker.as_deref(), Some("步海"));
    assert!(d10.text.contains("汉堡包"));
    assert_eq!(d10.voice.as_deref(), Some("AY_0204"));

    // 第 48 句：香里奈再来一碗
    let d48 = &dialogues[47];
    assert_eq!(d48.index, 48);
    assert_eq!(d48.speaker.as_deref(), Some("香里奈"));
    assert!(d48.text.contains("再来一碗"));

    // 第 49 句：步海切葱（原先在此处被空标记挂起 Bug 阻断）
    let d49 = &dialogues[48];
    assert_eq!(d49.index, 49);
    assert_eq!(d49.speaker.as_deref(), Some("步海"));
    assert!(d49.text.contains("我现在就切葱"));
    assert_eq!(d49.voice.as_deref(), Some("AY_0211"));

    // 第 50 句：旁白带头照顾
    let d50 = &dialogues[49];
    assert_eq!(d50.index, 50);
    assert!(d50.speaker.is_none(), "第50句为旁白");
    assert!(d50.text.contains("步海还带头照顾大家"));

    // 第 95 句：步海名场面（新生活）
    let d95 = &dialogues[94];
    assert_eq!(d95.index, 95);
    assert_eq!(d95.speaker.as_deref(), Some("步海"));
    assert!(d95.text.contains("习惯新生活"));
    assert_eq!(d95.voice.as_deref(), Some("AY_0222"));
    assert_eq!(d95.bg.as_deref(), Some("pc_bg_12A"));
    assert_eq!(d95.bgm.as_deref(), Some("PC_BGM_09"));

    // 第 96 句：步海名场面（哥哥就是哥哥）
    let d96 = &dialogues[95];
    assert_eq!(d96.index, 96);
    assert_eq!(d96.speaker.as_deref(), Some("步海"));
    assert!(d96.text.contains("对我来说，哥哥就是哥哥"));
    assert_eq!(d96.voice.as_deref(), Some("AY_0223"));

    // 第 97 句：步海
    let d97 = &dialogues[96];
    assert_eq!(d97.index, 97);
    assert_eq!(d97.speaker.as_deref(), Some("步海"));
    assert_eq!(d97.voice.as_deref(), Some("AY_0224"));

    // 第 98 句：男主角沉默
    let d98 = &dialogues[97];
    assert_eq!(d98.index, 98);
    assert_eq!(d98.speaker.as_deref(), Some("李骏"));
    assert!(d98.text.contains("………"));

    // 第 99 句：空踩雷
    let d99 = &dialogues[98];
    assert_eq!(d99.index, 99);
    assert_eq!(d99.speaker.as_deref(), Some("空"));
    assert!(d99.text.contains("踩了个大雷"));
    assert_eq!(d99.voice.as_deref(), Some("SO_0228"));

    // 第 100 句：香里奈调侃
    let d100 = &dialogues[99];
    assert_eq!(d100.index, 100);
    assert_eq!(d100.speaker.as_deref(), Some("香里奈"));
    assert!(d100.text.contains("这就已经是极限了"));
    assert_eq!(d100.voice.as_deref(), Some("KA_0101"));
    assert!(!d100.slots.is_empty(), "第100句立绘槽位映射不应为空");

    println!("  [OK] 前 100 句台词中文字符串、说话人、语音 ID、立绘槽位、背景与 BGM 校验全部无误！");

    // 5. 测试细粒度 step() 事件派发机制
    let mut step_vm = VmEngine::new(&script);
    let mut seen_scene = false;
    let mut seen_bgm = false;
    let mut seen_voice = false;
    let mut seen_dialogue = false;

    for _ in 0..200 {
        if let Some(event) = step_vm.step().expect("单步执行失败") {
            match event {
                VmEvent::SceneChange { bg } => {
                    assert_eq!(bg, "white");
                    seen_scene = true;
                }
                VmEvent::BgmPlay { bgm } => {
                    assert_eq!(bgm, "PC_BGM_17");
                    seen_bgm = true;
                }
                VmEvent::VoicePlay { voice } => {
                    assert_eq!(voice, "AY_0202");
                    seen_voice = true;
                }
                VmEvent::Dialogue(dlg) => {
                    assert_eq!(dlg.index, 1);
                    assert_eq!(dlg.text, "「哥哥，步海想玩过家家」");
                    seen_dialogue = true;
                    break;
                }
                _ => {}
            }
        }
    }

    assert!(seen_scene, "未捕获到场景切换事件");
    assert!(seen_bgm, "未捕获到 BGM 播放事件");
    assert!(seen_voice, "未捕获到语音播放事件");
    assert!(seen_dialogue, "未捕获到第一句台词对白事件");
    println!("  [OK] 细粒度 step() 事件流验证完全通过！");
}

#[test]
fn test_vm_multi_script_execution() {
    let mut vfs = VirtualFileSystem::new();
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");

    if !patch_dir.is_dir() {
        return;
    }

    vfs.mount_directory(patch_dir).expect("挂载汉化目录失败");

    // 测试女主有栖路线剧本 arisa_01
    if let Ok(script_bytes) = vfs.read("arisa_01") {
        let script = BgiScript::parse(&script_bytes).expect("arisa_01 剧本解析失败");
        let mut vm = VmEngine::new(&script);
        let mut dialogues = Vec::new();
        for _ in 0..50 {
            if let Some(dlg) = vm.next_dialogue().expect("流式执行错误") {
                dialogues.push(dlg);
            }
        }
        assert_eq!(dialogues.len(), 50, "arisa_01 前50句台词提取未达标");
        assert_eq!(dialogues[2].speaker.as_deref(), Some("李骏"));
        assert!(dialogues[3].text.contains("揉了揉惺松的睡眼"));
        println!("  [OK] arisa_01 前50句台词流式解析与校验成功！");
    }

    // 测试序章剧本 01_prologue1
    if let Ok(script_bytes) = vfs.read("01_prologue1") {
        let script = BgiScript::parse(&script_bytes).expect("01_prologue1 剧本解析失败");
        let mut vm = VmEngine::new(&script);
        let mut dialogues = Vec::new();
        for _ in 0..50 {
            if let Some(dlg) = vm.next_dialogue().expect("流式执行错误") {
                dialogues.push(dlg);
            }
        }
        assert_eq!(dialogues.len(), 50, "01_prologue1 前50句台词提取未达标");
        assert_eq!(dialogues[0].speaker.as_deref(), Some("美由纪"));
        assert!(dialogues[0].text.contains("听得见吗"));
        println!("  [OK] 01_prologue1 前50句台词流式解析与校验成功！");
    }
}

#[test]
fn test_mail_chapter111_loopback_and_real_arisa_mail_script() {
    let mut vfs = VirtualFileSystem::new();
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");
    if !patch_dir.is_dir() {
        return;
    }
    vfs.mount_directory(patch_dir).expect("挂载汉化目录失败");

    // 1. 黄金集成测试 mail_chapter111 回跳闭环：
    // 进邮箱选单 -> 选邮件1 (flag[300]=1) -> 0x0038(&&)判假 0x0019 向后回跳重弹选单 -> 选邮件2 (flag[301]=1) -> 0x0038(&&)判真退出
    let mail_bytes = vfs.read("mail_chapter111").expect("读取 mail_chapter111 失败");
    let mail_script = BgiScript::parse(&mail_bytes).expect("解析 mail_chapter111 失败");
    let mut vm = VmEngine::new(&mail_script);
    vm.set_script_name("mail_chapter111");

    // 第 1 次进入选单
    assert!(vm.next_dialogue().expect("执行失败").is_none());
    match vm.state() {
        VmState::WaitingForChoice {
            prompt, options, ..
        } => {
            assert!(prompt.is_none(), "mail_chapter111 无标题，prompt 必须为 None");
            assert_eq!(options, &vec!["给空发邮件", "给步海发邮件", "不发邮件"]);
        }
        other => panic!("预期第1次 WaitingForChoice，实际 {:?}", other),
    }
    let seq1 = vm.choice_seq();
    assert!(vm.submit_choice(seq1, 0), "选邮件1失败");
    assert!(!vm.submit_choice(seq1, 1), "重复提交 seq1 必须被拒绝");

    // flag[300]=1, flag[301]=0 -> 0x0038(&&) 为假 -> 回跳重弹选单
    assert!(vm.next_dialogue().expect("回跳执行失败").is_none());
    assert_eq!(vm.get_flag(300), 1, "选邮件1后 flag[300] 必须为 1");
    assert_eq!(vm.get_flag(301), 0, "选邮件1后 flag[301] 仍为 0");
    match vm.state() {
        VmState::WaitingForChoice { options, .. } => {
            assert_eq!(options, &vec!["给空发邮件", "给步海发邮件", "不发邮件"]);
        }
        other => panic!("0x0038(&&) 判假后必须回跳重弹选单，实际 {:?}", other),
    }

    // 第 2 次进入选单，选邮件2 (seq2 > seq1)
    let seq2 = vm.choice_seq();
    assert!(seq2 > seq1, "choice_seq 必须单调递增");
    assert!(!vm.submit_choice(seq1, 1), "旧 seq1 必须被拒绝");
    assert!(vm.submit_choice(seq2, 1), "选邮件2失败");

    // flag[300]=1 且 flag[301]=1 -> 0x0038(&&) 为真 -> 正常退出 Terminated
    assert!(vm.next_dialogue().expect("退出执行失败").is_none());
    assert_eq!(vm.get_flag(300), 1);
    assert_eq!(vm.get_flag(301), 1);
    assert_eq!(*vm.state(), VmState::Terminated, "两封邮件均发送后必须正常 Terminated");
    assert_eq!(vm.eval_stack_len(), 0);

    // 2. 真实脚本 mail_chapter1_arisa 跑通全流程（含嵌套 flag[53]=flag[53]+flag[10]）
    let arisa_mail_bytes = vfs.read("mail_chapter1_arisa").expect("读取 mail_chapter1_arisa 失败");
    let arisa_mail_script = BgiScript::parse(&arisa_mail_bytes).expect("解析失败");
    let mut am_vm = VmEngine::new(&arisa_mail_script);
    am_vm.set_script_name("mail_chapter1_arisa");
    let mut loops = 0usize;
    while *am_vm.state() != VmState::Terminated {
        loops += 1;
        assert!(loops < 200, "mail_chapter1_arisa 步数异常");
        let _ = am_vm.next_dialogue().expect("mail_chapter1_arisa 执行失败");
        if matches!(am_vm.state(), VmState::WaitingForChoice { .. }) {
            let sq = am_vm.choice_seq();
            assert!(am_vm.submit_choice(sq, 0));
        }
    }
    assert_eq!(am_vm.eval_stack_len(), 0);
}

#[test]
fn test_five_mainline_scripts_full_flow_and_dual_branch() {
    let mut vfs = VirtualFileSystem::new();
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");
    if !patch_dir.is_dir() {
        return;
    }
    vfs.mount_directory(patch_dir).expect("挂载汉化目录失败");

    let run_to_end = |name: &str, init_flags: &[(u32, i32)]| -> Vec<String> {
        let bytes = vfs.read(name).unwrap_or_else(|e| panic!("读取 {} 失败: {:?}", name, e));
        let script = BgiScript::parse(&bytes).unwrap_or_else(|e| panic!("解析 {} 失败: {:?}", name, e));
        let mut vm = VmEngine::new(&script);
        vm.set_script_name(name);
        for &(k, v) in init_flags {
            vm.set_flag(k, v);
        }
        let mut texts = Vec::new();
        let mut guard = 0usize;
        while *vm.state() != VmState::Terminated {
            guard += 1;
            assert!(guard < 10_000, "{} 超过 10_000 句未停机", name);
            if let Some(dlg) = vm.next_dialogue().unwrap_or_else(|e| panic!("{} 执行失败: {:?}", name, e)) {
                assert_eq!(vm.eval_stack_len(), 0, "{} 对白处求值栈应平衡", name);
                texts.push(dlg.text);
            } else if matches!(vm.state(), VmState::WaitingForChoice { .. }) {
                let sq = vm.choice_seq();
                assert!(vm.submit_choice(sq, 0), "{} 提交选项失败", name);
            }
        }
        assert_eq!(vm.eval_stack_len(), 0, "{} 停机后求值栈应为空", name);
        texts
    };

    // 1. 03_prologue3 (含带标题 _SelectEx)
    let p3_texts = run_to_end("03_prologue3", &[]);
    assert!(p3_texts.len() > 200, "03_prologue3 对白数应 > 200 (实际 {})", p3_texts.len());

    // 2. 04_chapter1_1 注入不同 flag[300/301] 走双路径，验证绝无双线同时播报
    let ch1_sora = run_to_end("04_chapter1_1", &[(300, 1), (301, 0)]);
    let ch1_ayumi = run_to_end("04_chapter1_1", &[(300, 0), (301, 1)]);
    assert!(ch1_sora.len() > 100);
    assert!(ch1_ayumi.len() > 100);
    assert_ne!(
        ch1_sora, ch1_ayumi,
        "04_chapter1_1 在不同 flag[300/301] 下必须走不同剧情分支"
    );
    // 验证排他性：两条分支不应将另一条分支的专属对白全数混播
    let sora_only = ch1_sora.iter().filter(|t| !ch1_ayumi.contains(t)).count();
    let ayumi_only = ch1_ayumi.iter().filter(|t| !ch1_sora.contains(t)).count();
    assert!(
        sora_only > 5 && ayumi_only > 5,
        "双路径各自应有独占剧情对白 (sora_only={}, ayumi_only={})",
        sora_only,
        ayumi_only
    );

    // 3. 三条个人线脚本全流程跑通
    for name in ["arisa_01", "moemi_02", "sora_01"] {
        let lines = run_to_end(name, &[]);
        assert!(lines.len() > 100, "{} 对白数应 > 100 (实际 {})", name, lines.len());
    }
}

#[test]
fn test_arisa_02_h1_sys_reg_20_and_cg_unlock_and_all_scripts_sweep() {
    let mut vfs = VirtualFileSystem::new();
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");
    if !patch_dir.is_dir() {
        return;
    }
    vfs.mount_directory(patch_dir).expect("挂载汉化目录失败");

    // 1. 验证 arisa_02_h1 依据 config_regs[20] (set_hscene_mode) 走 0(由选项决定)/1(LABEL_IN)/2(LABEL_OUT) 三条路径，且触发 0x00E6 解锁 CG 230
    let h_bytes = vfs.read("arisa_02_h1").expect("读取 arisa_02_h1 失败");
    let h_script = BgiScript::parse(&h_bytes).expect("解析 arisa_02_h1 失败");

    let run_h = |mode: i32| -> (bool, Vec<String>, std::collections::BTreeSet<u32>) {
        let mut vm = VmEngine::new(&h_script);
        vm.set_script_name("arisa_02_h1");
        assert!(vm.set_hscene_mode(mode));
        // 隔离验证：故意写入 sys_regs[20]，绝不得串扰 config_regs[20]
        vm.set_sys_reg(20, (mode + 1) % 3);
        let mut saw_choice = false;
        let mut texts = Vec::new();
        let mut guard = 0usize;
        while *vm.state() != VmState::Terminated {
            guard += 1;
            assert!(guard < 10_000, "arisa_02_h1(mode={}) 超步数上限", mode);
            if let Some(dlg) = vm.next_dialogue().expect("arisa_02_h1 执行失败") {
                texts.push(dlg.text);
            } else if let VmState::WaitingForChoice { options, .. } = vm.state() {
                saw_choice = true;
                assert_eq!(options, &vec!["射里面".to_string(), "射外面".to_string()]);
                let sq = vm.choice_seq();
                assert!(vm.submit_choice(sq, 0));
            }
        }
        (saw_choice, texts, vm.unlocked_cgs().clone())
    };

    let (saw_choice_0, texts_0, cgs_0) = run_h(0);
    let (saw_choice_1, texts_1, cgs_1) = run_h(1);
    let (saw_choice_2, texts_2, cgs_2) = run_h(2);

    assert!(saw_choice_0, "config_regs[20]=0 必须弹出 _SelectEx 询问");
    assert!(!saw_choice_1, "config_regs[20]=1 必须直接 LABEL_IN 不弹窗");
    assert!(!saw_choice_2, "config_regs[20]=2 必须直接 LABEL_OUT 不弹窗");
    assert_eq!(texts_0, texts_1, "mode=0 选 0(射里面) 应与 mode=1(LABEL_IN) 对白一致");
    assert_ne!(texts_1, texts_2, "mode=1(LABEL_IN) 与 mode=2(LABEL_OUT) 对白分支必须不同");
    assert!(cgs_0.contains(&230) && cgs_1.contains(&230) && cgs_2.contains(&230), "arisa_02_h1 必须解锁 CG 230");

    // 2. 全游 199 个 BGI 编译脚本全扫：不得有未知 opcode/sys_id、越界或死循环熔断
    let mut script_count = 0usize;
    let mut entries: Vec<_> = std::fs::read_dir(patch_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        if !path.is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes.len() < 32 || !bytes.starts_with(b"BurikoCompiledScriptVer1.00\0") {
            continue;
        }
        script_count += 1;
        let sname = path.file_name().unwrap().to_string_lossy().into_owned();
        let script = BgiScript::parse(&bytes).unwrap_or_else(|e| panic!("解析 {} 失败: {:?}", sname, e));
        let mut vm = VmEngine::new(&script);
        vm.set_script_name(&sname);
        let mut guard = 0usize;
        let mut choice_pick = 0usize;
        while *vm.state() != VmState::Terminated {
            guard += 1;
            assert!(guard < 20_000, "脚本 {} 超过 20000 步未停机", sname);
            let _ = vm.next_dialogue().unwrap_or_else(|e| panic!("脚本 {} 执行失败: {:?}", sname, e));
            if let VmState::WaitingForChoice { ref options, .. } = vm.state() {
                let n = options.len().max(1);
                let idx = if guard > 20 { n - 1 } else { choice_pick % n };
                choice_pick += 1;
                let sq = vm.choice_seq();
                assert!(vm.submit_choice(sq, idx), "脚本 {} submit_choice 失败", sname);
            }
        }
        assert_eq!(vm.eval_stack_len(), 0, "脚本 {} 停机后栈未清零", sname);
    }
    assert!(script_count >= 190, "应扫描全游脚本，实际仅 {} 个", script_count);

    // 3. 验证 02_prologue2 / 04_chapter1_1 自我介绍、moemi_10_h1 无名语音分支及 4 处汉化漏改短语
    let run_all_dialogues = |sname: &str| -> Vec<pure_connect_core::DialogueEvent> {
        let bytes = vfs.read(sname).unwrap_or_else(|e| panic!("读取 {} 失败: {:?}", sname, e));
        let script = BgiScript::parse(&bytes).unwrap();
        let mut vm = VmEngine::new(&script);
        vm.set_script_name(sname);
        let mut out = Vec::new();
        while *vm.state() != VmState::Terminated {
            if let Some(dlg) = vm.next_dialogue().unwrap() {
                assert!(
                    !dlg.text.contains("主人公"),
                    "脚本 {} 台词不应残留未替换的『主人公』占位符: {}",
                    sname,
                    dlg.text
                );
                if let Some(ref spk) = dlg.speaker {
                    assert!(
                        !spk.contains("主人公"),
                        "脚本 {} 说话人不应残留未替换的『主人公』占位符: {}",
                        sname,
                        spk
                    );
                }
                out.push(dlg);
            }
            if let VmState::WaitingForChoice { .. } = vm.state() {
                let sq = vm.choice_seq();
                assert!(vm.submit_choice(sq, 0));
            }
        }
        out
    };

    let p2_dlgs = run_all_dialogues("02_prologue2");
    assert!(
        p2_dlgs.iter().any(|d| d.text.contains("我叫李骏，和空那家伙有段孽缘")),
        "02_prologue2 必须走自定义姓名分支并输出『我叫李骏』"
    );

    let c1_dlgs = run_all_dialogues("04_chapter1_1");
    assert!(
        c1_dlgs.iter().any(|d| d.text.contains("我是李骏，大家请多指教")),
        "04_chapter1_1 必须走自定义姓名分支并输出『我是李骏，大家请多指教』"
    );

    let mo10_dlgs = run_all_dialogues("moemi_10_h1");
    assert!(
        mo10_dlgs.iter().any(|d| d.voice.as_deref() == Some("MO_2894")
            && d.text.contains("今天我的男朋友真的")),
        "moemi_10_h1 @ 0x4a50 必须走无名语音 MO_2894 分支且无卡死"
    );

    let ar05_dlgs = run_all_dialogues("arisa_05");
    assert!(
        ar05_dlgs.iter().any(|d| d.text.contains("想要一直呆在前辈的身边")),
        "arisa_05 汉化漏改台词必须精准修正为『想要一直呆在前辈的身边』"
    );

    let mo03_dlgs = run_all_dialogues("moemi_03");
    assert!(
        mo03_dlgs
            .iter()
            .any(|d| d.text.contains("今天一整天都能和最喜欢的你一起约会")),
        "moemi_03 汉化漏改台词必须精准修正为『今天一整天都能和最喜欢的你一起约会』"
    );

    let mo07_dlgs = run_all_dialogues("moemi_07_h1");
    assert!(
        mo07_dlgs
            .iter()
            .any(|d| d.text.contains("在家的时候一直都在在意你的事")),
        "moemi_07_h1 汉化漏改台词必须精准修正为『在家的时候一直都在在意你的事』"
    );

    let mo08_dlgs = run_all_dialogues("moemi_08");
    assert!(
        mo08_dlgs.iter().any(|d| d.text.contains("总觉得你像小孩子一样")),
        "moemi_08 汉化漏改台词必须精准修正为『总觉得你像小孩子一样』"
    );
}
