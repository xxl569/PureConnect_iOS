//! 绑定层单元与增量 Delta 回归测试集

use std::collections::HashMap;
use std::path::Path;

use pure_connect_core::vm::{CharacterSlot, VmState};

use crate::protocol::{
    compute_slot_delta, KEY_BG_TEXTURE, KEY_CHANGED_SLOTS, KEY_CHARACTER_INTRO, KEY_CHOICE_OPTIONS,
    KEY_CHOICE_PROMPT, KEY_CLEARED_SLOTS, KEY_INDEX, KEY_SPEAKER, KEY_STATE, KEY_TEXT,
    KEY_VOICE_STREAM,
};
use crate::runtime::{BgUpdate, PureConnectRuntime, SprUpdate, StepResult};
use crate::session::ScriptSession;

#[test]
fn test_protocol_key_constants() {
    assert_eq!(KEY_TEXT, "text");
    assert_eq!(KEY_SPEAKER, "speaker");
    assert_eq!(KEY_VOICE_STREAM, "voice_stream");
    assert_eq!(KEY_BG_TEXTURE, "bg_texture");
    assert_eq!(KEY_CHANGED_SLOTS, "changed_slots");
    assert_eq!(KEY_CLEARED_SLOTS, "cleared_slots");
    assert_eq!(KEY_STATE, "state");
    assert_eq!(KEY_CHOICE_PROMPT, "choice_prompt");
    assert_eq!(KEY_CHOICE_OPTIONS, "choice_options");
    assert_eq!(KEY_INDEX, "index");
    assert_eq!(KEY_CHARACTER_INTRO, "character_intro");
}

#[test]
fn test_slot_delta_calculation_edge_cases() {
    // 场景 1: 从无到有 (初始加载立绘槽位 1 与 2)
    let last_slots = HashMap::new();
    let mut current_slots = HashMap::new();
    current_slots.insert(
        1,
        CharacterSlot {
            slot_id: 1,
            sprites: vec!["SO01_L_S_A11".to_string()],
        },
    );
    current_slots.insert(
        2,
        CharacterSlot {
            slot_id: 2,
            sprites: vec!["AY01_L_S_A11".to_string()],
        },
    );

    let (changed, cleared) = compute_slot_delta(&last_slots, &current_slots);
    assert_eq!(changed, vec![1, 2], "首次渲染槽位 1 与 2 必须全部进入 changed");
    assert!(cleared.is_empty(), "无槽位被清除");

    // 场景 2: 保持不变 (下一句台词未修改立绘)
    let mut last_slots: HashMap<u32, Vec<String>> = HashMap::new();
    last_slots.insert(1, vec!["SO01_L_S_A11".to_string()]);
    last_slots.insert(2, vec!["AY01_L_S_A11".to_string()]);

    let (changed, cleared) = compute_slot_delta(&last_slots, &current_slots);
    assert!(changed.is_empty(), "未改变的槽位绝不应重新合成");
    assert!(cleared.is_empty(), "无槽位被清除");

    // 场景 3: 槽位 1 表情差分更新，槽位 2 保持不变
    current_slots.insert(
        1,
        CharacterSlot {
            slot_id: 1,
            sprites: vec!["SO01_L_S_A11".to_string(), "SO01_L_M_A14".to_string()],
        },
    );

    let (changed, cleared) = compute_slot_delta(&last_slots, &current_slots);
    assert_eq!(changed, vec![1], "仅槽位 1 表情发生变化");
    assert!(cleared.is_empty());

    // 场景 4: 槽位 2 退场 (清空)，槽位 3 新登场
    last_slots.insert(
        1,
        vec!["SO01_L_S_A11".to_string(), "SO01_L_M_A14".to_string()],
    );
    current_slots.remove(&2);
    current_slots.insert(
        3,
        CharacterSlot {
            slot_id: 3,
            sprites: vec!["MK01_L_S_A11".to_string()],
        },
    );

    let (changed, cleared) = compute_slot_delta(&last_slots, &current_slots);
    assert_eq!(changed, vec![3], "槽位 3 为新登场");
    assert_eq!(cleared, vec![2], "槽位 2 必须被标记为 cleared");

    // 场景 5: 全场立绘清空 (例如 clear_bs)
    last_slots.insert(3, vec!["MK01_L_S_A11".to_string()]);
    current_slots.clear();

    let (changed, cleared) = compute_slot_delta(&last_slots, &current_slots);
    assert!(changed.is_empty());
    assert_eq!(cleared, vec![1, 2, 3]);
}

#[test]
fn test_script_session_with_real_script() {
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");
    if !patch_dir.is_dir() {
        eprintln!("补丁目录不存在，跳过此测试");
        return;
    }

    let script_path = patch_dir.join("05_chapter1_2");
    if !script_path.is_file() {
        eprintln!("剧本文件不存在，跳过此测试");
        return;
    }

    let script_bytes = std::fs::read(&script_path).expect("读取剧本失败");
    let mut session = ScriptSession::new(script_bytes).expect("创建会话失败");

    assert_eq!(*session.state(), VmState::Ready);

    // 连续流式推进 50 句对白
    for i in 1..=50 {
        let dlg = session
            .next_dialogue()
            .expect("推进执行失败")
            .expect("未能获取到对白");
        assert_eq!(dlg.index, i);
        assert!(!dlg.text.is_empty(), "台词内容为空");
    }

    assert_eq!(*session.state(), VmState::WaitingForInput);
}

#[test]
fn test_synthetic_script_session_flow() {
    let mut script_bytes = Vec::new();
    script_bytes.extend_from_slice(pure_connect_core::vm::BGI_SCRIPT_MAGIC);
    script_bytes.extend_from_slice(&8u32.to_le_bytes()); // header_size = 8
    script_bytes.extend_from_slice(&0u32.to_le_bytes()); // module_count = 0

    let code_len = 20usize;
    let speaker_offset = code_len as u32;

    let speaker_str = "秋乃";
    let (spk_gbk, _, _) = encoding_rs::GBK.encode(speaker_str);
    let text_str = "这是一句合成台词测试！";
    let (txt_gbk, _, _) = encoding_rs::GBK.encode(text_str);

    let text_offset = speaker_offset + spk_gbk.len() as u32 + 1;

    // 1. PushString 说话人 (0x0003, speaker_offset)
    script_bytes.extend_from_slice(&3u32.to_le_bytes());
    script_bytes.extend_from_slice(&speaker_offset.to_le_bytes());

    // 2. PushString 台词 (0x0003, text_offset)
    script_bytes.extend_from_slice(&3u32.to_le_bytes());
    script_bytes.extend_from_slice(&text_offset.to_le_bytes());

    // 3. 对白指令 0x0140
    script_bytes.extend_from_slice(&0x0140u32.to_le_bytes());

    // 追加字符串池
    script_bytes.extend_from_slice(&spk_gbk);
    script_bytes.push(0);
    script_bytes.extend_from_slice(&txt_gbk);
    script_bytes.push(0);

    let mut session = ScriptSession::new(script_bytes).expect("创建会话失败");
    assert_eq!(*session.state(), VmState::Ready);

    let dlg = session
        .next_dialogue()
        .expect("推进失败")
        .expect("未获得对白");
    assert_eq!(dlg.speaker.as_deref(), Some("秋乃"));
    assert_eq!(dlg.text, "这是一句合成台词测试！");
    assert_eq!(*session.state(), VmState::WaitingForInput);
}

#[test]
fn test_synthetic_script_session_empty_data_fails_safely() {
    let empty_data = Vec::new();
    assert!(ScriptSession::new(empty_data).is_err());
}

#[test]
fn test_runtime_uninitialized_and_safe_fallback() {
    let mut rt = PureConnectRuntime::new();
    assert_eq!(rt.get_state(), "Unloaded");
    assert!(matches!(rt.next_step(), StepResult::Unloaded));
    assert!(!rt.select_choice(0));
    assert!(!rt.select_choice(-1));

    // 未初始化 VFS 时加载剧本应返回 Err
    assert!(rt.load_script("test").is_err());
    assert_eq!(rt.get_state(), "Unloaded");
}

#[test]
fn test_runtime_choice_branch_state_machine_and_bounds_checking() {
    let mut data = Vec::new();
    data.extend_from_slice(pure_connect_core::vm::BGI_SCRIPT_MAGIC);
    data.extend_from_slice(&8u32.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());

    let code_len = 150usize;
    let mut strings = Vec::new();

    let mut add_str = |s: &str| -> u32 {
        let rel_offset = (code_len + strings.len()) as u32;
        let (gbk, _, _) = encoding_rs::GBK.encode(s);
        strings.extend_from_slice(&gbk);
        strings.push(0);
        rel_offset
    };

    let opt1 = add_str("去休息室");
    let opt2 = add_str("去事务所");
    let prompt = add_str("去哪里呢？");
    let fn_select = add_str("_SelectEx");
    let after_txt = add_str("分支选择后的台词");

    let mut code = Vec::new();
    // 压入分支选项与标头
    code.extend_from_slice(&3u32.to_le_bytes());
    code.extend_from_slice(&opt1.to_le_bytes());
    code.extend_from_slice(&3u32.to_le_bytes());
    code.extend_from_slice(&opt2.to_le_bytes());
    code.extend_from_slice(&3u32.to_le_bytes());
    code.extend_from_slice(&prompt.to_le_bytes());
    code.extend_from_slice(&3u32.to_le_bytes());
    code.extend_from_slice(&fn_select.to_le_bytes());
    code.extend_from_slice(&0x1cu32.to_le_bytes()); // CALL_USER_FUNC

    // 分支后续对白
    code.extend_from_slice(&3u32.to_le_bytes());
    code.extend_from_slice(&after_txt.to_le_bytes());
    code.extend_from_slice(&0x140u32.to_le_bytes());

    code.resize(code_len, 0);
    data.extend_from_slice(&code);
    data.extend_from_slice(&strings);

    // 将合成剧本放入内存作为散落文件挂载测试
    let temp_dir = std::env::temp_dir().join("pure_connect_choice_test");
    let _ = std::fs::create_dir_all(&temp_dir);
    let script_file = temp_dir.join("choice_test.bs5");
    std::fs::write(&script_file, &data).expect("写入临时剧本失败");

    let mut rt = PureConnectRuntime::new();
    assert!(rt.init_vfs("", temp_dir.to_str().unwrap()).unwrap());
    assert!(rt.load_script("choice_test").unwrap());

    assert_eq!(rt.get_state(), "Ready");

    // 推进到达分支选择
    let step = rt.next_step();
    match step {
        StepResult::Choice { prompt: p, options, .. } => {
            assert_eq!(p.as_deref(), Some("去哪里呢？"));
            assert_eq!(options, vec!["去休息室", "去事务所"]);
        }
        other => panic!("预期 StepResult::Choice，实际得到: {:?}", other),
    }

    assert_eq!(rt.get_state(), "WaitingForChoice");

    // 边界鲁棒性攻击：越界选择必须安全拦截并返回 false，且不破坏 WaitingForChoice 状态
    assert!(!rt.select_choice(-1), "负数索引必须被拒绝");
    assert!(!rt.select_choice(2), "越界索引 2 必须被拒绝（选项总数为 2）");
    assert!(!rt.select_choice(100), "超大越界索引必须被拒绝");
    assert_eq!(rt.get_state(), "WaitingForChoice", "越界后状态必须保持不变");

    // 合法选择选项 0
    assert!(rt.select_choice(0), "合法索引 0 必须成功提交");
    assert_eq!(rt.get_state(), "Running");

    // 恢复后流式执行可到达分支后对白
    let after_step = rt.next_step();
    match after_step {
        StepResult::Dialogue(dlg) => {
            assert_eq!(dlg.index, 1);
            assert_eq!(dlg.text, "分支选择后的台词");
        }
        other => panic!("预期 StepResult::Dialogue，实际得到: {:?}", other),
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_runtime_with_real_assets_vfs_audio_and_bg_resolution() {
    let archive_dir = Path::new(r"U:\anti\projects\PURExCONNECT\Archive");
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");

    if !archive_dir.is_dir() || !patch_dir.is_dir() {
        eprintln!("真实游戏资产目录不存在，跳过资产测试");
        return;
    }

    let mut rt = PureConnectRuntime::new();
    let mounted = rt
        .init_vfs(
            archive_dir.to_str().unwrap(),
            patch_dir.to_str().unwrap(),
        )
        .expect("初始化 VFS 失败");
    assert!(mounted, "必须成功挂载资产");

    // 1. 测试纯色白/黑背景生成 (1280x720 RGBA8)
    let white_img = rt.resolve_image("white").expect("white 背景解析失败");
    assert_eq!(white_img.width, 1280);
    assert_eq!(white_img.height, 720);
    assert_eq!(white_img.rgba_data.len(), 1280 * 720 * 4);
    assert_eq!(white_img.rgba_data[0..4], [255, 255, 255, 255]);

    let black_img = rt.resolve_image("black").expect("black 背景解析失败");
    assert_eq!(black_img.width, 1280);
    assert_eq!(black_img.height, 720);
    assert_eq!(black_img.rgba_data[0..4], [0, 0, 0, 255]);

    // 2. 测试真实背景 CompressedBG 解码 (_pc_bg_13w / pc_bg_13a)
    // 剧本传入不带下划线的前缀名，必须成功匹配带下划线的封包条目 _pc_bg_13w
    let bg_img = rt
        .resolve_image("pc_bg_13w")
        .expect("pc_bg_13w 背景解析失败");
    assert_eq!(bg_img.width, 1380);
    assert_eq!(bg_img.height, 820);
    assert_eq!(bg_img.bpp, 32);

    // 3. 测试立绘单图解码 mo02pb_fh
    let char_img = rt
        .resolve_image("mo02pb_fh")
        .expect("mo02pb_fh 立绘解析失败");
    assert_eq!(char_img.width, 680);
    assert_eq!(char_img.height, 920);

    // 4. 测试真实语音 OGG 剥离与解析 (_so_0162 / ay_0202)
    let voice_ogg = rt
        .resolve_audio("so_0162")
        .expect("so_0162 语音解析失败");
    assert!(!voice_ogg.is_empty());
    assert_eq!(&voice_ogg[0..4], b"OggS", "剥离头部后必须为标准 OggS 流");

    let voice_ogg2 = rt
        .resolve_audio("ay_0202")
        .expect("ay_0202 语音解析失败");
    assert!(!voice_ogg2.is_empty());
    assert_eq!(&voice_ogg2[0..4], b"OggS", "剥离头部后必须为标准 OggS 流");

    // 5. 测试立绘宏代号解析与两级回退 (SO01_N002 -> so01aa_n 全身底图 1080x920, AR01_N042 -> ar01aa_n 全身底图 1080x920)
    let sora_macro_img = rt
        .resolve_image("SO01_N002")
        .expect("SO01_N002 宏立绘解析必须成功回退至全身底图");
    assert_eq!(sora_macro_img.width, 1080);
    assert_eq!(sora_macro_img.height, 920);

    let arisa_macro_img = rt
        .resolve_image("AR01_N042")
        .expect("AR01_N042 宏立绘解析必须成功回退至全身底图");
    assert_eq!(arisa_macro_img.width, 1080);
    assert_eq!(arisa_macro_img.height, 920);

    // 5b. 测试特殊服装 (如 06 系列无 Version 1 身体时自动回退至 01 基础服装)
    let moemi_06_img = rt
        .resolve_image("MO06_M031")
        .expect("MO06_M031 特殊服装必须成功回退至 mo01/mo03 基础身体底图");
    assert!(moemi_06_img.width > 0 && moemi_06_img.height > 0);

    // 6. 测试多部件差分立绘合成 (以差分开头的序列，CBG v2 底图 ar01aa_n 为 1080x920)
    let comp_img = rt
        .composite_slot_sprites(&[
            "AR01_N_F_A05".to_string(),
            "AR01_N_M_A10".to_string(),
            "AR01_N_E_A05".to_string(),
        ])
        .expect("差分立绘合成必须成功");
    assert_eq!(comp_img.width, 1080);
    assert_eq!(comp_img.height, 920);

    // 7. 测试角色登场专属音频解析 (so_title / mo_title)
    let so_title_audio = rt.resolve_audio("so_title").expect("so_title 必须成功解析");
    assert_eq!(&so_title_audio[0..4], b"OggS");

    // 8. 测试汉化补丁 16 字节头未压缩位图解析 (ch1_arisa0a / ch1_siho0a)
    let ch1_arisa = rt
        .resolve_image("ch1_arisa0a")
        .expect("exe_unpacked 汉化版 ch1_arisa0a 必须成功解码");
    assert_eq!(ch1_arisa.width, 1280);
    assert_eq!(ch1_arisa.height, 720);

    let ch1_siho = rt
        .resolve_image("ch1_siho0a")
        .expect("exe_unpacked 汉化版 ch1_siho0a 必须成功解码");
    assert_eq!(ch1_siho.width, 1280);
    assert_eq!(ch1_siho.height, 720);

    // 9. 测试宏展开 + 五官差分合成确实写入了面部区域，且 3 帧图带不会把第 1/2 帧绘到胸口
    let faceless_si = rt.resolve_image("si01aa_n").expect("si01aa_n 必须存在");
    let full_si_n013 = rt
        .composite_slot_sprites(&["SI01_N013".to_string()])
        .expect("SI01_N013 合成必须成功");
    assert_eq!(full_si_n013.width, 1080);
    assert_eq!(full_si_n013.height, 920);
    // 眼部区域 (x=470..590, y=220..280) 必须与无五官底图存在显著像素差异
    let mut eye_diff_px = 0usize;
    for y in 220..280usize {
        for x in 470..590usize {
            let idx = (y * 1080 + x) * 4;
            if full_si_n013.rgba_data[idx..idx + 3] != faceless_si.rgba_data[idx..idx + 3] {
                eye_diff_px += 1;
            }
        }
    }
    assert!(
        eye_diff_px > 200,
        "SI01_N013 眼部必须合成五官像素 (当前差异像素: {})",
        eye_diff_px
    );

    // 校验 3 帧图带第 1/2 帧没有溢出到身体下方 (y=580..650 眼部水平位置)
    let mut torso_leak_px = 0usize;
    for y in 580..650usize {
        for x in 470..590usize {
            let idx = (y * 1080 + x) * 4;
            if full_si_n013.rgba_data[idx..idx + 3] != faceless_si.rgba_data[idx..idx + 3] {
                torso_leak_px += 1;
            }
        }
    }
    assert_eq!(
        torso_leak_px, 0,
        "SI01_N013 第 1/2 帧眼睛严禁漏画到躯干上"
    );

    // 10. 测试宏 + 覆盖部件混合合成 (AR01_N042 + AR01_N_F_A05 + AR01_N_M_A10 + AR01_N_E_A05 + AR01_N_S_A02)
    let faceless_ar = rt.resolve_image("ar01aa_n").expect("ar01aa_n 必须存在");
    let full_ar_comp = rt
        .composite_slot_sprites(&[
            "AR01_N042".to_string(),
            "AR01_N_F_A05".to_string(),
            "AR01_N_M_A10".to_string(),
            "AR01_N_E_A05".to_string(),
            "AR01_N_S_A02".to_string(),
        ])
        .expect("AR01_N042 + 覆盖部件合成必须成功");
    let mut ar_eye_diff = 0usize;
    for y in 260..310usize {
        for x in 450..625usize {
            let idx = (y * 1080 + x) * 4;
            if full_ar_comp.rgba_data[idx..idx + 3] != faceless_ar.rgba_data[idx..idx + 3] {
                ar_eye_diff += 1;
            }
        }
    }
    assert!(ar_eye_diff > 500, "AR01 眼部必须合成五官像素");
}

#[test]
fn test_runtime_real_script_dialogue_pipeline_and_snapshots() {
    let archive_dir = Path::new(r"U:\anti\projects\PURExCONNECT\Archive");
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");

    if !archive_dir.is_dir() || !patch_dir.is_dir() {
        eprintln!("真实游戏资产目录不存在，跳过此测试");
        return;
    }

    let mut rt = PureConnectRuntime::new();
    let mounted = rt
        .init_vfs(
            archive_dir.to_str().unwrap(),
            patch_dir.to_str().unwrap(),
        )
        .expect("初始化 VFS 失败");
    assert!(mounted);

    let loaded = rt.load_script("05_chapter1_2").expect("加载剧本失败");
    assert!(loaded, "剧本 05_chapter1_2 必须成功加载");
    assert_eq!(rt.get_state(), "Ready");

    // 连续流式推进 50 句对白，检验数据桥接字段完整性
    for i in 1..=50 {
        let step = rt.next_step();
        match step {
            StepResult::Dialogue(dlg) => {
                assert_eq!(dlg.index, i, "对白序号必须严格连续 (1-based)");
                assert!(!dlg.text.is_empty(), "第 {} 句台词不能为空", i);

                // 检验说话人与语音数据合法性
                if let Some(ref spk) = dlg.speaker {
                    assert!(!spk.is_empty(), "说话人名字不能为空字符串");
                }
                if let Some(ref voice_bytes) = dlg.voice_data {
                    assert_eq!(&voice_bytes[0..4], b"OggS", "语音数据必须为纯净 OggS 字节流");
                }

                // 首次出现背景时必须发出 BgUpdate::New
                if i == 1 {
                    assert!(
                        matches!(dlg.bg_update, BgUpdate::New(_)),
                        "第 1 句对白必须初始化场景背景"
                    );
                }
            }
            other => panic!("第 {} 句对白推进异常: {:?}", i, other),
        }
        assert_eq!(rt.get_state(), "WaitingForInput");
    }
}

#[test]
fn test_stage5_godot_project_and_presentation_assembly() {
    let project_dir = Path::new(r"U:\anti\projects\pure_connect_gal2\project");
    assert!(project_dir.is_dir(), "project 目录必须存在");

    // 1. project.godot 结构检查
    let project_godot = project_dir.join("project.godot");
    assert!(project_godot.is_file(), "project.godot 必须存在");
    let godot_cfg = std::fs::read_to_string(&project_godot).expect("读取 project.godot 失败");
    assert!(godot_cfg.contains("run/main_scene=\"res://Main.tscn\""));
    assert!(godot_cfg.contains("window/size/viewport_width=1280"));
    assert!(godot_cfg.contains("window/size/viewport_height=720"));
    assert!(godot_cfg.contains("window/stretch/mode=\"canvas_items\""));
    assert!(godot_cfg.contains("pointing/emulate_touch_from_mouse=true"));

    // 2. pure_connect.gdextension 检查
    let gdext_path = project_dir.join("pure_connect.gdextension");
    assert!(gdext_path.is_file(), "pure_connect.gdextension 必须存在于 project 目录");
    let gdext_cfg = std::fs::read_to_string(&gdext_path).expect("读取 gdextension 失败");
    assert!(gdext_cfg.contains("compatibility_minimum = \"4.3\""));
    assert!(gdext_cfg.contains("windows.debug.x86_64 = \"res://target/debug/pure_connect_godot.dll\""));

    // 3. Main.tscn 节点树检查
    let main_tscn = project_dir.join("Main.tscn");
    assert!(main_tscn.is_file(), "Main.tscn 必须存在");
    let tscn_content = std::fs::read_to_string(&main_tscn).expect("读取 Main.tscn 失败");
    assert!(tscn_content.contains("[node name=\"Background\" type=\"TextureRect\""));
    assert!(tscn_content.contains("[node name=\"CharacterSlots\" type=\"Control\""));
    for slot_id in 1..=10 {
        assert!(
            tscn_content.contains(&format!("[node name=\"Slot{}\" type=\"TextureRect\"", slot_id)),
            "Main.tscn 必须定义立绘槽位 Slot{}",
            slot_id
        );
    }
    assert!(tscn_content.contains("[node name=\"DialogueBox\" type=\"PanelContainer\""));
    assert!(tscn_content.contains("[node name=\"SpeakerLabel\" type=\"Label\""));
    assert!(tscn_content.contains("[node name=\"TextLabel\" type=\"RichTextLabel\""));
    assert!(tscn_content.contains("[node name=\"ChoiceBox\" type=\"PanelContainer\""));
    assert!(tscn_content.contains("[node name=\"VoicePlayer\" type=\"AudioStreamPlayer\""));
    assert!(tscn_content.contains("[node name=\"BgmPlayer\" type=\"AudioStreamPlayer\""));

    // 4. main.gd 脚本结构检查
    let main_gd = project_dir.join("main.gd");
    assert!(main_gd.is_file(), "main.gd 必须存在");
    let gd_content = std::fs::read_to_string(&main_gd).expect("读取 main.gd 失败");
    assert!(gd_content.contains("PureConnectEngine.new()"));
    assert!(gd_content.contains("init_vfs"));
    assert!(gd_content.contains("load_script"));
    assert!(gd_content.contains("next_dialogue"));
    assert!(gd_content.contains("select_choice"));
    assert!(gd_content.contains("changed_slots"));
    assert!(gd_content.contains("cleared_slots"));
    assert!(gd_content.contains("voice_stream"));
    assert!(gd_content.contains("bg_texture"));
    assert!(gd_content.contains("_is_ready"), "main.gd 必须具备生命周期幂等防护");
    assert!(gd_content.contains("_exit_tree"), "main.gd 必须实现退出树资源释放以杜绝 ObjectDB 泄漏");

    // 5. 启动Windows试玩.bat 检查
    let start_bat = Path::new(r"U:\anti\projects\pure_connect_gal2\启动Windows试玩.bat");
    assert!(start_bat.is_file(), "启动Windows试玩.bat 必须存在于根工程");
    let bat_content = std::fs::read_to_string(start_bat).expect("读取批处理脚本失败");
    assert!(bat_content.contains("Godot_v4.3"));
    assert!(bat_content.contains("--path \"project\" res://Main.tscn"));

    // 6. 验证套件 tools/verify_stage5.gd
    let verify_tool = Path::new(r"U:\anti\projects\pure_connect_gal2\tools\verify_stage5.gd");
    assert!(verify_tool.is_file(), "验收脚本 verify_stage5.gd 必须存在");
    let tool_content = std::fs::read_to_string(verify_tool).expect("读取验收脚本失败");
    assert!(tool_content.contains("WaitingForChoice"), "验收脚本必须包含分支选择弹窗状态机断言");
}

#[test]
fn test_pc_parity_sysgrp_dsc_bg_pan_moemi_slot9_and_save_restore() {
    let base_dir = r"U:\anti\projects\PURExCONNECT\Archive";
    let patch_dir = r"U:\anti\temp\exe_unpacked";
    if !Path::new(base_dir).exists() || !Path::new(patch_dir).exists() {
        return;
    }

    let mut rt = PureConnectRuntime::new();
    assert!(rt.init_vfs(base_dir, patch_dir).expect("init_vfs失败"));

    // 1. 验证父目录 sysgrp.arc DSC 解码（标题/设置/滑块/存档卡片皮肤）与 syssnd.arc 音效
    for ui_name in [
        "SGTitle990000",
        "SGTitle000000",
        "SGSysWnd000000",
        "SGCnfgDsp020000",
        "SGCnfgSnd010002",
        "SGUsDtWnd800000",
    ] {
        let img = rt
            .load_ui_image(ui_name)
            .unwrap_or_else(|| panic!("sysgrp.arc DSC 资产解码失败: {}", ui_name));
        assert!(img.width > 0 && img.height > 0);
    }
    for sfx_name in ["Select_01", "Cancel", "Cursor", "sound_test_se", "sound_test_all"] {
        let audio = rt
            .resolve_audio(sfx_name)
            .unwrap_or_else(|| panic!("音频解包失败: {}", sfx_name));
        assert_eq!(&audio[0..4], b"OggS", "{} 必须为 OggS", sfx_name);
    }

    // 2. 验证 01_prologue1 开场背景平移动画 (pc_bg_04B_9: -1480,-340 -> -920,0) 与 BGM 下发
    assert!(rt.load_script("01_prologue1").expect("加载01_prologue1失败"));
    let mut saw_pan = false;
    let mut saw_bgm = false;
    let mut moemi_index = 0usize;
    let mut shiho_intro_layers = 0usize;

    for _ in 0..260 {
        match rt.next_step() {
            StepResult::Dialogue(dlg) => {
                if dlg.bgm_changed && dlg.bgm_name.is_some() {
                    saw_bgm = true;
                }
                if !dlg.bg_motions.is_empty() {
                    if let Some(ref pose) = dlg.bg_pose {
                        if pose.x == Some(-1480) && pose.y == Some(-340) {
                            let m = &dlg.bg_motions[0];
                            assert_eq!(m.target_x, Some(-920));
                            assert_eq!(m.target_y, Some(0));
                            assert_eq!(m.duration_ms, 40000);
                            saw_pan = true;
                        }
                    }
                }
                if let Some(ref intro) = dlg.character_intro {
                    if intro.character == "shiho" {
                        shiho_intro_layers = intro.layers.iter().filter(|l| l.is_some()).count();
                    }
                }
                if dlg.text.contains("什锦烧") {
                    assert!(dlg.face_changed || dlg.face_texture.is_some());
                    assert_eq!(dlg.face_offset, (-541, 451), "萌美半身像偏移应精准匹配 (-541, 451)");
                    let face_img = dlg.face_texture.as_ref().expect("萌美台词必须返回合成半身头像").clone();
                    assert_eq!((face_img.width, face_img.height), (680, 920));
                    assert!(
                        !dlg.changed_slots.contains_key(&1) && !dlg.changed_slots.contains_key(&9),
                        "slot 9 萌美半身像绝不可污染舞台中央 Slot1"
                    );
                    // 校验五官非空：与空白素体 MO01PC_F 比对眼部/眉毛/嘴巴区域，确保五官完整合成
                    let base_moemi = rt.resolve_image("mo01pc_f").expect("mo01pc_f 必须存在");
                    let mut eye_diff_px = 0usize;
                    for y in 315..352usize {
                        for x in 285..396usize {
                            let idx = (y * 680 + x) * 4;
                            let dr = face_img.rgba_data[idx].abs_diff(base_moemi.rgba_data[idx]) as u32;
                            let dg = face_img.rgba_data[idx + 1].abs_diff(base_moemi.rgba_data[idx + 1]) as u32;
                            let db = face_img.rgba_data[idx + 2].abs_diff(base_moemi.rgba_data[idx + 2]) as u32;
                            if dr + dg + db > 25 {
                                eye_diff_px += 1;
                            }
                        }
                    }
                    assert!(
                        eye_diff_px > 1200,
                        "萌美左下角头像 MO01PC_F+MO01_F004 眼部必须合成清晰五官像素 (实际差异像素: {})",
                        eye_diff_px
                    );
                    let mut mouth_diff_px = 0usize;
                    for y in 368..382usize {
                        for x in 325..356usize {
                            let idx = (y * 680 + x) * 4;
                            let dr = face_img.rgba_data[idx].abs_diff(base_moemi.rgba_data[idx]) as u32;
                            let dg = face_img.rgba_data[idx + 1].abs_diff(base_moemi.rgba_data[idx + 1]) as u32;
                            let db = face_img.rgba_data[idx + 2].abs_diff(base_moemi.rgba_data[idx + 2]) as u32;
                            if dr + dg + db > 20 {
                                mouth_diff_px += 1;
                            }
                        }
                    }
                    assert!(
                        mouth_diff_px > 20,
                        "萌美左下角头像 MO01PC_F+MO01_F004 嘴巴必须合成五官像素 (实际差异像素: {})",
                        mouth_diff_px
                    );
                    moemi_index = dlg.index;
                    break;
                }
            }
            StepResult::Terminated => break,
            other => panic!("推进异常: {:?}", other),
        }
    }
    assert!(saw_pan, "必须捕获到 pc_bg_04B_9 的 40 秒平滑运镜参数");
    assert!(saw_bgm, "必须正确触发开场 BGM 变更事件");
    assert_eq!(shiho_intro_layers, 5, "志穗登场介绍必须完整加载 ec_sh_01..05 共 5 层原生图层");
    assert!(moemi_index > 0, "必须推进到萌美什锦烧台词");

    // 3. 验证快进读档快照恢复精准还原相同句号、BGM 与左下角萌美头像
    match rt.load_save_state("01_prologue1", moemi_index, &[]) {
        StepResult::Dialogue(restored) => {
            assert_eq!(restored.index, moemi_index);
            assert!(restored.text.contains("什锦烧"));
            assert!(restored.face_texture.is_some(), "读档恢复后必须重构萌美半身像");
            assert!(matches!(restored.bg_update, BgUpdate::New(_)), "读档恢复后必须重建背景");
        }
        other => panic!("读档恢复结果异常: {:?}", other),
    }
}

#[test]
fn test_sprite_layer_category_no_size_collision() {
    // 1. _F_ 头像尺寸系列：绝不可将尺寸码 _f_ 误判为眉毛部件 f
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo01_f_f_a09"), Some("f"));
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo01_f_r_00"), Some("r"));
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo01_f_m_a09"), Some("m"));
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo01_f_e_a09"), Some("e"));
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo01_f_s_a09"), Some("s"));
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo01_f_hl"), Some("hl"));
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo01_f_h"), Some("h"));
    assert_eq!(PureConnectRuntime::sprite_layer_category("ay01_f_h_sa"), Some("h"));
    assert_eq!(PureConnectRuntime::sprite_layer_category("si01_f_t_01"), Some("t"));

    // 2. _M_ 中景尺寸系列：绝不可将尺寸码 _m_ 误判为嘴巴部件 m
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo05_m_f_b02"), Some("f"));
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo05_m_m_b06"), Some("m"));
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo05_m_e_b02"), Some("e"));

    // 3. 底图与宏名不应返回局部部件分类
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo01pc_f"), None);
    assert_eq!(PureConnectRuntime::sprite_layer_category("mo01_f004"), None);
}

#[test]
fn test_all_heroines_face_avatar_and_medium_sprite_compositing() {
    let archive_dir = Path::new(r"U:\anti\projects\PURExCONNECT\Archive");
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");
    if !archive_dir.is_dir() || !patch_dir.is_dir() {
        return;
    }

    let mut rt = PureConnectRuntime::new();
    assert!(rt
        .init_vfs(archive_dir.to_str().unwrap(), patch_dir.to_str().unwrap())
        .expect("init_vfs 失败"));

    // 1. 验证全部主要女性角色 _F 小头像组合均写入了显著面部五官像素
    let face_cases: &[(&str, &[&str])] = &[
        ("mo01pc_f", &["MO01PC_F", "MO01_F004"]),
        ("so03pd_f", &["SO03PD_F", "SO03_F004"]),
        ("ka01pa_f", &["KA01PA_F", "KA01_F002", "KA01_F_M_A08"]),
        ("ay01pa_f", &["AY01PA_F", "AY01_F047", "AY01_F_S_A05", "AY01_F_H_SA"]),
        ("ar01aa_f", &["AR01AA_F", "AR01_F001"]),
        ("si02pc_f", &["SI02PC_F", "SI01_F042"]),
        ("mo01aa_f", &["MO01AA_F", "MO01_F001", "MO01_F_R_00AA", "MO01_F_H_AA"]),
        ("ar01pc_f", &["AR01PC_F", "AR01_F003", "AR01_F_B_01", "AR01_F_M_A06"]),
        ("se01ab_f", &["SE01AB_F", "SE01_F003", "SE01_F_F_A06", "SE01_F_E_A06"]),
        ("ki01pa_f", &["KI01PA_F", "KI01_F005", "KI01_F_B_01"]),
        ("ke01pa_f", &["KE01PA_F", "KE01_F001"]),
    ];

    for (base_name, sprites) in face_cases {
        let base_img = rt
            .resolve_image(base_name)
            .unwrap_or_else(|| panic!("底图 {} 解析失败", base_name));
        let sprite_vec: Vec<String> = sprites.iter().map(|s| (*s).to_string()).collect();
        let comp_img = rt
            .composite_slot_sprites(&sprite_vec)
            .unwrap_or_else(|| panic!("半身头像合成失败: {:?}", sprites));
        assert_eq!(
            (comp_img.width, comp_img.height),
            (base_img.width, base_img.height)
        );

        // 在面部区间 (x=120..540, y=120..500) 统计与无五官底图存在明显色差的五官像素数
        let w = comp_img.width as usize;
        let mut face_diff_px = 0usize;
        for y in 120..500.min(comp_img.height as usize) {
            for x in 120..540.min(w) {
                let idx = (y * w + x) * 4;
                let dr = comp_img.rgba_data[idx].abs_diff(base_img.rgba_data[idx]) as u32;
                let dg = comp_img.rgba_data[idx + 1].abs_diff(base_img.rgba_data[idx + 1]) as u32;
                let db = comp_img.rgba_data[idx + 2].abs_diff(base_img.rgba_data[idx + 2]) as u32;
                if dr + dg + db > 30 {
                    face_diff_px += 1;
                }
            }
        }
        assert!(
            face_diff_px > 900,
            "半身头像 {:?} 面部五官像素不足（可能发生图层碰撞覆盖），差异像素={}",
            sprites,
            face_diff_px
        );
    }

    // 2. 验证 _M_ 中景立绘不因 _m_ 碰撞丢失眼睛、眉毛或嘴巴
    let base_m = rt.resolve_image("mo05pd_m").expect("mo05pd_m 必须存在");
    let comp_m = rt
        .composite_slot_sprites(&[
            "MO05PD_M".to_string(),
            "MO05_M041".to_string(),
            "MO05_M_F_B02".to_string(),
            "MO05_M_M_B06".to_string(),
            "MO05_M_E_B02".to_string(),
        ])
        .expect("MO05PD_M 合成失败");
    let comp_m_direct = rt
        .composite_slot_sprites(&[
            "MO05PD_M".to_string(),
            "MO05_M_F_B02".to_string(),
            "MO05_M_M_B06".to_string(),
            "MO05_M_E_B02".to_string(),
        ])
        .expect("MO05PD_M 部件直拼失败");
    let comp_m_no_eye = rt
        .composite_slot_sprites(&[
            "MO05PD_M".to_string(),
            "MO05_M_F_B02".to_string(),
            "MO05_M_M_B06".to_string(),
        ])
        .expect("MO05PD_M 无眼部合成失败");
    let comp_m_no_mouth = rt
        .composite_slot_sprites(&[
            "MO05PD_M".to_string(),
            "MO05_M_F_B02".to_string(),
            "MO05_M_E_B02".to_string(),
        ])
        .expect("MO05PD_M 无嘴部合成失败");
    let wm = comp_m.width as usize;
    let hm = comp_m.height as usize;
    let mut m_diff_px = 0usize;
    let mut m_eye_diff_px = 0usize;
    let mut m_mouth_diff_px = 0usize;
    for y in 150..580.min(hm) {
        for x in 200..(wm.saturating_sub(200)) {
            let idx = (y * wm + x) * 4;
            let dr = comp_m.rgba_data[idx].abs_diff(base_m.rgba_data[idx]) as u32;
            let dg = comp_m.rgba_data[idx + 1].abs_diff(base_m.rgba_data[idx + 1]) as u32;
            let db = comp_m.rgba_data[idx + 2].abs_diff(base_m.rgba_data[idx + 2]) as u32;
            if dr + dg + db > 30 {
                m_diff_px += 1;
            }
            let edr = comp_m_direct.rgba_data[idx].abs_diff(comp_m_no_eye.rgba_data[idx]) as u32;
            let edg =
                comp_m_direct.rgba_data[idx + 1].abs_diff(comp_m_no_eye.rgba_data[idx + 1]) as u32;
            let edb =
                comp_m_direct.rgba_data[idx + 2].abs_diff(comp_m_no_eye.rgba_data[idx + 2]) as u32;
            if edr + edg + edb > 30 {
                m_eye_diff_px += 1;
            }
            let mdr = comp_m_direct.rgba_data[idx].abs_diff(comp_m_no_mouth.rgba_data[idx]) as u32;
            let mdg = comp_m_direct.rgba_data[idx + 1]
                .abs_diff(comp_m_no_mouth.rgba_data[idx + 1]) as u32;
            let mdb = comp_m_direct.rgba_data[idx + 2]
                .abs_diff(comp_m_no_mouth.rgba_data[idx + 2]) as u32;
            if mdr + mdg + mdb > 30 {
                m_mouth_diff_px += 1;
            }
        }
    }
    assert!(
        m_diff_px > 700,
        "中景立绘 MO05PD_M 面部五官差异像素不足: {}",
        m_diff_px
    );
    assert!(
        m_eye_diff_px > 200,
        "中景立绘 MO05PD_M 眼部覆层 MO05_M_E_B02 未生效（疑似被 _m_ 碰撞覆盖）: {}",
        m_eye_diff_px
    );
    assert!(
        m_mouth_diff_px > 50,
        "中景立绘 MO05PD_M 嘴部覆层 MO05_M_M_B06 未生效（疑似被 _m_ 碰撞覆盖）: {}",
        m_mouth_diff_px
    );
}

#[test]
fn test_save_v2_choice_restore_v1_compat_and_global_cg_unlock() {
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");
    if !patch_dir.is_dir() {
        return;
    }

    let mut rt = PureConnectRuntime::new();
    assert!(rt.init_vfs("", patch_dir.to_str().unwrap()).expect("init_vfs 失败"));

    // 1. 跨脚本保留 flags，仅从头开始清空
    assert!(rt.load_script("mail_chapter111").expect("加载 mail_chapter111 失败"));
    match rt.next_step() {
        StepResult::Choice { options, .. } => {
            assert_eq!(options, vec!["给空发邮件", "给步海发邮件", "不发邮件"]);
        }
        other => panic!("预期 StepResult::Choice，实际 {:?}", other),
    }
    let seq1 = rt.current_choice_seq();
    assert!(rt.submit_choice(seq1, 0), "选邮件1失败");

    // 推进至第2次弹出选项界面 (此时 flag[300] == 1, flag[301] == 0)
    match rt.next_step() {
        StepResult::Choice { options, .. } => {
            assert_eq!(options, vec!["给空发邮件", "给步海发邮件", "不发邮件"]);
        }
        other => panic!("预期第2次 StepResult::Choice，实际 {:?}", other),
    }
    let seq_before_save = rt.current_choice_seq();
    assert!(seq_before_save > seq1);
    assert_eq!(rt.get_flag(300), 1);

    // 2. 在选项界面存档 (Schema v2)
    let save_v2_json = rt.export_save_v2_json();
    assert!(save_v2_json.contains("\"save_version\":2"));
    assert!(save_v2_json.contains("\"pending_choice\""));

    // 3. 模拟 0x00E6 全局 CG 解锁，并验证读取旧档与从头开始不会覆盖回滚全局解锁
    assert!(rt.import_system_flags_json(r#"{"version":1,"unlocked_cgs":[230,240]}"#));
    assert!(rt.unlocked_cgs().contains(&230));
    assert!(rt.unlocked_cgs().contains(&240));

    // 切脚本保留 flags 测试
    assert!(rt.load_script("04_chapter1_1").expect("切至 04_chapter1_1 失败"));
    assert_eq!(rt.get_flag(300), 1, "切换下一脚本时 flags[300] 必须保留");
    // 从头开始清零 flags，但不清全局 CG 解锁
    rt.reset_session_flags();
    assert_eq!(rt.get_flag(300), 0, "从头开始必须清零 flags");
    assert!(rt.unlocked_cgs().contains(&230), "从头开始绝不应清空全局 CG 解锁");

    // 4. 读取刚才在选项界面保存的 Schema v2 存档
    let (restored_step, _visuals) = rt.load_save_json(&save_v2_json);
    match restored_step {
        StepResult::Choice { prompt, options, .. } => {
            assert!(prompt.is_none());
            assert_eq!(options, vec!["给空发邮件", "给步海发邮件", "不发邮件"]);
        }
        other => panic!("读档应直接恢复 WaitingForChoice，实际 {:?}", other),
    }
    assert_eq!(rt.get_flag(300), 1, "读档必须还原 flag[300]=1");
    assert!(rt.unlocked_cgs().contains(&230), "读取旧档绝不能覆盖回滚全局 CG 解锁");

    // 读档后 choice_seq 必须递增，读档前的旧回调 seq_before_save 必须被拒绝
    let seq_after_load = rt.current_choice_seq();
    assert!(seq_after_load > seq_before_save, "读档后 choice_seq 必须递增");
    assert!(!rt.submit_choice(seq_before_save, 1), "读档前旧 choice_seq 必须被拦截");

    // 用新 choice_seq 选择邮件2 -> 完成闭环退出
    assert!(rt.submit_choice(seq_after_load, 1), "用新 choice_seq 选择邮件2必须成功");
    assert!(matches!(rt.next_step(), StepResult::Terminated));
    assert_eq!(rt.get_flag(301), 1);

    // 5. v1 旧档 JSON 向后兼容测试（无 save_version / flags / pending_choice / suspended_pc）
    let v1_json = r#"{"script_name":"01_prologue1","dialogue_index":3,"choices":[],"speaker":"美由纪","text":"旧档测试"}"#;
    let (v1_step, _) = rt.load_save_json(v1_json);
    match v1_step {
        StepResult::Dialogue(dlg) => {
            assert_eq!(dlg.index, 3, "v1 旧档应退化重放至第 3 句");
        }
        other => panic!("v1 旧档加载失败: {:?}", other),
    }
    assert!(rt.unlocked_cgs().contains(&230), "加载 v1 旧档绝不可清空全局 CG 解锁");

    // 6. 验证 peek_upcoming 遇到 WaitingForChoice 自然止步，不预读未选分支
    assert!(rt.load_script("mail_chapter111").expect("加载失败"));
    let _ = rt.next_step(); // 停在 WaitingForChoice
    assert!(
        rt.session().unwrap().peek_upcoming(3).is_empty(),
        "处于 WaitingForChoice 时 peek_upcoming 应自然止步"
    );

    // 7. 验证 dialogue_index > 0 时的选项画面 Save v2（以 03_prologue3 真实剧本测试 visual_snapshot + choice 恢复）
    rt.reset_session_flags();
    assert!(rt.load_script("03_prologue3").expect("加载 03_prologue3 失败"));
    let mut dlg_count = 0usize;
    loop {
        match rt.next_step() {
            StepResult::Dialogue(_) => dlg_count += 1,
            StepResult::Choice { prompt, options, .. } => {
                assert!(dlg_count > 100, "03_prologue3 选项前应有 >100 句对白");
                assert_eq!(prompt.as_deref(), Some("我想想……"));
                assert_eq!(options, vec!["20岁出头！", "有30岁了！"]);
                break;
            }
            other => panic!("03_prologue3 预期遇到选项，实际 {:?}", other),
        }
    }
    let p3_save_json = rt.export_save_v2_json();
    let p3_seq_before = rt.current_choice_seq();
    let (p3_step, p3_visuals) = rt.load_save_json(&p3_save_json);
    assert!(p3_visuals.is_some(), "dialogue_index > 0 的选项画面读档必须返回视觉上下文");
    assert_eq!(p3_visuals.as_ref().unwrap().index, dlg_count);
    match p3_step {
        StepResult::Choice { prompt, options, .. } => {
            assert_eq!(prompt.as_deref(), Some("我想想……"));
            assert_eq!(options, vec!["20岁出头！", "有30岁了！"]);
        }
        other => panic!("预期恢复 WaitingForChoice，实际 {:?}", other),
    }
    let p3_seq_after = rt.current_choice_seq();
    assert!(p3_seq_after > p3_seq_before);
    assert!(!rt.submit_choice(p3_seq_before, 1));
    assert!(rt.submit_choice(p3_seq_after, 1), "恢复后提交选项 1 必须成功");
    match rt.next_step() {
        StepResult::Dialogue(dlg) => assert_eq!(dlg.index, dlg_count + 1),
        other => panic!("选定后应正常推进下一句对白，实际 {:?}", other),
    }

    // 8. 验证真实 H 脚本 arisa_02_h1 对 config_regs[20] (set_hscene_mode) 与 0x00E6 全局 CG 解锁脏标志闭环
    let mut rt_h = PureConnectRuntime::new();
    assert!(rt_h.init_vfs("", patch_dir.to_str().unwrap()).expect("init_vfs 失败"));
    // 边界校验：越界值不得被接受，且 set_sys_reg(20, ...) 不得串扰 0xE3(20)
    assert!(!rt_h.set_hscene_mode(-1));
    assert!(!rt_h.set_hscene_mode(3));
    assert_eq!(rt_h.get_hscene_mode(), 0);
    rt_h.set_sys_reg(20, 1); // 故意污染 sys_regs[20]，验证与 config_regs 完全隔离
    assert!(rt_h.set_hscene_mode(2)); // LABEL_OUT
    assert_eq!(rt_h.get_hscene_mode(), 2);
    assert!(rt_h.load_script("arisa_02_h1").expect("加载 arisa_02_h1 失败"));
    let mut texts_out = Vec::new();
    loop {
        match rt_h.next_step() {
            StepResult::Dialogue(d) => texts_out.push(d.text),
            StepResult::Terminated => break,
            other => panic!("config_regs[20]=2 时不应弹窗或出错，实际 {:?}", other),
        }
    }
    assert!(rt_h.consume_system_flags_dirty(), "执行 0x00E6 后必须触发 system_flags_dirty");
    assert!(rt_h.unlocked_cgs().contains(&230));
    let exported_sys = rt_h.export_system_flags_json();
    assert!(exported_sys.contains("230"));
    // 读回 v1 旧档绝不可覆盖已解锁的 CG 230
    let _ = rt_h.load_save_json(v1_json);
    assert!(rt_h.unlocked_cgs().contains(&230));

    // 9. 验证 set_hscene_mode 0(由选项决定) / 1(中出 LABEL_IN) / 2(外射 LABEL_OUT) 与 sys_regs 完全隔离
    let mut rt_iso = PureConnectRuntime::new();
    assert!(rt_iso.init_vfs("", patch_dir.to_str().unwrap()).expect("init_vfs 失败"));
    // sys_regs[20]=2 不应影响 0xE3(20)=0 弹窗分支
    rt_iso.set_sys_reg(20, 2);
    assert!(rt_iso.load_script("arisa_02_h1").expect("加载 arisa_02_h1 失败"));
    let mut saw_choice = false;
    loop {
        match rt_iso.next_step() {
            StepResult::Dialogue(_) => {}
            StepResult::Choice { options, .. } => {
                saw_choice = true;
                assert_eq!(options, vec!["射里面".to_string(), "射外面".to_string()]);
                let seq = rt_iso.current_choice_seq();
                assert!(rt_iso.submit_choice(seq, 0));
            }
            StepResult::Terminated => break,
            other => panic!("arisa_02_h1(mode=0) 执行异常: {:?}", other),
        }
    }
    assert!(saw_choice, "sys_regs[20]=2 绝不能污染 0xE3(20)=0 的弹窗行为");

    // 先设 set_hscene_mode(1) 再新建脚本会话 (load_script)，验证持久化与 LABEL_IN 路径
    assert!(rt_iso.set_hscene_mode(1));
    assert!(rt_iso.load_script("arisa_02_h1").expect("加载 arisa_02_h1 失败"));
    let mut texts_in = Vec::new();
    loop {
        match rt_iso.next_step() {
            StepResult::Dialogue(d) => texts_in.push(d.text),
            StepResult::Terminated => break,
            other => panic!("set_hscene_mode(1) 时不应弹窗或出错，实际 {:?}", other),
        }
    }
    assert_ne!(texts_in, texts_out, "mode=1(中出 LABEL_IN) 与 mode=2(外射 LABEL_OUT) 对白必须不同");
    // 确认 sys_regs[0](主角名=0 自定义名「李骏」) 与 sys_regs[410](正式版=0) 未被 config_regs 影响
    assert_eq!(rt_iso.get_sys_reg(0), 0);
    assert_eq!(rt_iso.get_sys_reg(410), 0);
}

#[test]
fn test_batch1_subscript_save_load_return_and_legacy_v1_v2_compat() {
    let base_dir = r"U:\anti\projects\PURExCONNECT\Archive";
    let patch_dir = r"U:\anti\temp\exe_unpacked";
    if !Path::new(base_dir).is_dir() || !Path::new(patch_dir).is_dir() {
        return;
    }

    // 1. 实测“在 0x00F0 子脚本内存档 -> 读档 -> 执行至 0x001B 准确返回父脚本后续剧情”闭环
    //    从 root_Arisa 进入 0x00F0(arisa_01) 后推进数句对白，此时 call_stack 栈顶为 root_Arisa
    let mut rt = PureConnectRuntime::new();
    assert!(rt.init_vfs(base_dir, patch_dir).unwrap());
    assert!(rt.load_script("root_Arisa").unwrap());

    let mut entered_sub = false;
    let mut saved_dlg_text = String::new();
    for _ in 0..10 {
        match rt.next_step() {
            StepResult::Dialogue(d) => {
                saved_dlg_text = d.text;
                let cur = rt.get_save_state().0;
                if !cur.eq_ignore_ascii_case("root_arisa") {
                    entered_sub = true;
                    break;
                }
            }
            other => panic!("推进 root_Arisa 首章应产出对白，实际 {:?}", other),
        }
    }
    assert!(entered_sub, "root_Arisa 必须通过 0x00F0 进入子章节脚本");

    let save_json = rt.export_save_v2_json();
    let parsed: crate::SaveStateV2 =
        serde_json::from_str(&save_json).expect("导出存档 JSON 必须合法");
    assert!(
        !parsed.call_stack.is_empty(),
        "在 0x00F0 子脚本内存档时 call_stack 必须非空"
    );
    assert!(
        parsed.call_stack[0].script_name.eq_ignore_ascii_case("root_arisa"),
        "call_stack 帧应记录父脚本 root_Arisa，实际 {:?}",
        parsed.call_stack
    );

    // 使用全新运行时读档
    let mut rt2 = PureConnectRuntime::new();
    assert!(rt2.init_vfs(base_dir, patch_dir).unwrap());
    let (res, _) = rt2.load_save_json(&save_json);
    match res {
        StepResult::Dialogue(d) => {
            assert_eq!(d.text, saved_dlg_text, "读档后必须精确还原存档点的子脚本对白");
        }
        other => panic!("读档应直接恢复子脚本对白画面，实际 {:?}", other),
    }

    // 验证读档后继续推进直到当前子脚本 0x001B 返回父脚本 root_Arisa 并进入下一章（而非过早 Terminated）
    let sub_script_name = parsed.script_name.clone();
    let mut switched_to_next_chapter = false;
    for _ in 0..2500 {
        match rt2.next_step() {
            StepResult::Dialogue(_) => {
                let cur = rt2.get_save_state().0;
                if !cur.eq_ignore_ascii_case(&sub_script_name) {
                    switched_to_next_chapter = true;
                    break;
                }
            }
            StepResult::Choice { .. } => {
                assert!(rt2.select_choice(0));
            }
            StepResult::Terminated => {
                panic!("子脚本 0x001B 必须返回父脚本 root_Arisa 推进下一章，绝不能直接 Terminated");
            }
            other => panic!("读档继续推进异常: {:?}", other),
        }
    }
    assert!(
        switched_to_next_chapter,
        "读档后执行至子脚本 0x001B 必须准确返回父脚本 root_Arisa 并推进后续章节"
    );

    // 2. 旧版 v1 / 旧版 v2（不含 call_stack / script_dialogue_index 字段）存档 JSON 兼容验证
    let legacy_v1 = r#"{"script_name":"01_prologue1","dialogue_index":2,"choices":[]}"#;
    let mut rt_v1 = PureConnectRuntime::new();
    assert!(rt_v1.init_vfs(base_dir, patch_dir).unwrap());
    let (r1, _) = rt_v1.load_save_json(legacy_v1);
    assert!(matches!(r1, StepResult::Dialogue(_)), "旧版 v1 存档必须无崩溃加载，实际 {:?}", r1);

    let legacy_v2 = r#"{"save_version":2,"script_name":"01_prologue1","dialogue_index":3,"choices":[],"entry_flags":{},"flags":{"50":5},"choice_seq":1}"#;
    let mut rt_v2 = PureConnectRuntime::new();
    assert!(rt_v2.init_vfs(base_dir, patch_dir).unwrap());
    let (r2, _) = rt_v2.load_save_json(legacy_v2);
    assert!(matches!(r2, StepResult::Dialogue(_)), "旧版 v2 存档必须无崩溃加载，实际 {:?}", r2);
    assert_eq!(rt_v2.get_flag(50), 5);
}

#[test]
fn test_batch1_review_tail_jump_save_load_and_nested_depth2_and_sys_regs_guard() {
    let base_dir = r"U:\anti\projects\PURExCONNECT\Archive";
    let patch_dir = r"U:\anti\temp\exe_unpacked";
    if !Path::new(base_dir).is_dir() || !Path::new(patch_dir).is_dir() {
        return;
    }

    // 1. 复现 Bug 1：20_chapter6_3 执行大量对白后通过 0x00F3 尾跳转到 90_badend_01（此时 call_stack 为空，
    //    但全局 dialogue_index >> script_dialogue_index），在 90_badend_01 第 2 句存档并读档，
    //    必须精确还原 90_badend_01 第 2 句，绝不能按全局 dialogue_index 跑飞或 Terminated！
    let mut rt = PureConnectRuntime::new();
    assert!(rt.init_vfs(base_dir, patch_dir).unwrap());
    assert!(rt.load_script("20_chapter6_3").unwrap());

    let mut badend_dlg_count = 0usize;
    let mut saved_text = String::new();
    let mut saved_global_idx = 0usize;
    for _ in 0..1000 {
        match rt.next_step() {
            StepResult::Dialogue(d) => {
                let cur = rt.get_save_state().0;
                if cur.eq_ignore_ascii_case("90_badend_01") {
                    badend_dlg_count += 1;
                    if badend_dlg_count == 2 {
                        saved_text = d.text.clone();
                        saved_global_idx = d.index;
                        break;
                    }
                }
            }
            StepResult::Choice { .. } => {
                assert!(rt.select_choice(0));
            }
            other => panic!("推进 20_chapter6_3 -> 90_badend_01 异常: {:?}", other),
        }
    }
    assert_eq!(badend_dlg_count, 2, "必须推进到 90_badend_01 第 2 句");
    assert!(
        saved_global_idx > 100,
        "尾跳转后全局 dialogue_index 应远大于 2，实际 {}",
        saved_global_idx
    );

    let save_json = rt.export_save_v2_json();
    let mut rt_reload = PureConnectRuntime::new();
    assert!(rt_reload.init_vfs(base_dir, patch_dir).unwrap());
    let (res, _) = rt_reload.load_save_json(&save_json);
    match res {
        StepResult::Dialogue(d) => {
            assert_eq!(
                d.text, saved_text,
                "0x00F3 尾跳转后存档读档必须精确还原 90_badend_01 第 2 句"
            );
            assert_eq!(
                d.index, saved_global_idx,
                "读档返回快照的全局 dialogue_index 必须与存档一致"
            );
        }
        other => panic!("0x00F3 尾跳转后读档失败，实际 {:?}", other),
    }

    // 2. 验证 import_system_flags_json 强制归一 sys_regs[0]=0（覆写旧存档/旧 system_flags.json 中的 "0":1）
    let mut rt_sys = PureConnectRuntime::new();
    assert!(rt_sys.import_system_flags_json(r#"{"version":1,"unlocked_cgs":[],"sys_regs":{"0":1,"400":1,"410":1}}"#));
    assert_eq!(rt_sys.get_sys_reg(0), 0, "sys_regs[0] 必须强制归一为 0（自定义名「李骏」）");
    assert_eq!(rt_sys.get_sys_reg(410), 0, "sys_regs[410] 正式版判定必须始终保底为 0");
    assert_eq!(rt_sys.get_sys_reg(400), 1, "跨周目持久寄存器 400 必须正常导入");

    // 2b. 验证 load_save_json 恢复旧版选项挂起存档 (pending_choice 含未替换的主人公占位符) 时，返回给 UI 的 StepResult::Choice 也被替换为「李骏」
    assert!(rt_sys.init_vfs(base_dir, patch_dir).unwrap());
    let legacy_choice_save = r#"{
        "save_version": 2,
        "script_name": "01_prologue1",
        "dialogue_index": 1,
        "script_dialogue_index": 1,
        "choices": [],
        "speaker": "主人公（名）",
        "text": "主人公（名）的回合",
        "entry_flags": {},
        "flags": {},
        "pending_choice": {
            "prompt": "主人公（名）的抉择",
            "options": ["我是主人公（姓）　主人公（名）", "呼叫主人公（姓）同学"]
        },
        "choice_seq": 7,
        "suspended_pc": 64,
        "call_stack": []
    }"#;
    let (choice_step, _) = rt_sys.load_save_json(legacy_choice_save);
    match choice_step {
        StepResult::Choice { prompt, options, .. } => {
            assert_eq!(
                prompt.as_deref(),
                Some("李骏的抉择"),
                "load_save_json 返回的选项 prompt 必须清洗为「李骏」"
            );
            assert_eq!(
                options,
                vec!["我是李骏".to_string(), "呼叫李同学".to_string()],
                "load_save_json 返回的选项 options 必须清洗为「李骏/李」"
            );
        }
        other => panic!("加载旧选项存档应返回 StepResult::Choice，实际 {:?}", other),
    }

    // 3. 验证 chk_Tel -> 0x00F0(50_TEL_SORA_01) 跨脚本 entry_flags 保留 + 子脚本存档读档返回 + 0x00E4(400, 1) 持久落盘
    let mut rt_tel = PureConnectRuntime::new();
    assert!(rt_tel.init_vfs(base_dir, patch_dir).unwrap());
    rt_tel.set_flag(50, 20);
    rt_tel.set_flag(10, 20);
    assert!(rt_tel.load_script("chk_Tel").unwrap());
    let mut tel_saved_text = String::new();
    let mut entered_tel_sub = false;
    for _ in 0..20 {
        match rt_tel.next_step() {
            StepResult::Dialogue(d) => {
                let cur = rt_tel.get_save_state().0;
                if cur.eq_ignore_ascii_case("50_tel_sora_01") {
                    tel_saved_text = d.text;
                    entered_tel_sub = true;
                    break;
                }
            }
            other => panic!("chk_Tel 推进至 50_TEL_SORA_01 异常: {:?}", other),
        }
    }
    assert!(entered_tel_sub, "chk_Tel 必须通过 0x00F0 进入 50_TEL_SORA_01");
    let tel_save_json = rt_tel.export_save_v2_json();
    let tel_save_parsed: crate::SaveStateV2 = serde_json::from_str(&tel_save_json).unwrap();
    assert_eq!(
        tel_save_parsed.entry_flags.get(&50).copied(),
        Some(20),
        "0x00F0 进入子脚本时必须快照父脚本的 flags 到 entry_flags"
    );

    let mut rt_tel2 = PureConnectRuntime::new();
    assert!(rt_tel2.init_vfs(base_dir, patch_dir).unwrap());
    let (tel_res, _) = rt_tel2.load_save_json(&tel_save_json);
    match tel_res {
        StepResult::Dialogue(d) => assert_eq!(d.text, tel_saved_text),
        other => panic!("50_TEL_SORA_01 读档失败: {:?}", other),
    }
    // 继续推进至 50_TEL_SORA_01 返回 chk_Tel，执行 0x00E4(400, 1) 并结束
    for _ in 0..300 {
        match rt_tel2.next_step() {
            StepResult::Dialogue(_) => {}
            StepResult::Choice { .. } => {
                assert!(rt_tel2.select_choice(0));
            }
            StepResult::Terminated => break,
            other => panic!("50_TEL_SORA_01 返回 chk_Tel 异常: {:?}", other),
        }
    }
    assert_eq!(rt_tel2.get_sys_reg(400), 1, "50_TEL_SORA_01 返回 chk_Tel 后必须执行 0x00E4(400, 1)");
    assert!(rt_tel2.consume_system_flags_dirty(), "0x00E4(400, 1) 必须触发 system_flags_dirty");
    let exported_sys = rt_tel2.export_system_flags_json();
    assert!(exported_sys.contains("\"400\": 1"), "system_flags.json 必须持久化 400=>1");
    assert!(!exported_sys.contains("\"410\""), "system_flags.json 不得导出瞬态 410");
}

#[test]
fn test_batch1_main_story_mail_closed_loop() {
    use crate::runtime::SprUpdate;
    use pure_connect_core::archive::Arc20Archive;
    use pure_connect_core::vfs::VirtualFileSystem;
    use std::collections::BTreeSet;

    let base_dir = r"U:\anti\projects\PURExCONNECT";
    let patch_dir = r"U:\anti\temp\exe_unpacked";
    if !std::path::Path::new(base_dir).exists() || !std::path::Path::new(patch_dir).exists() {
        eprintln!("跳过主线睡前短信集成测试：未检测到本地原版资产目录");
        return;
    }

    // 1. 实跑 Mail_chapter1_Arisa 多分支组合，覆盖全部 23 屏贴图 + 验证 WaitClick / SE / 0x0167 / cls_Mail_RS
    let mut arisa_unique_screens: BTreeSet<String> = BTreeSet::new();
    let mut total_wait_clicks: usize = 0;
    let mut saw_hide_window_on_choice = false;
    let mut saw_cls_fade_200 = false;
    let mut saw_monologue_with_phone = false;

    let branch_cases: &[(usize, usize, i32)] = &[
        (0, 0, 0),
        (0, 1, 10),
        (1, 0, 20),
        (1, 1, 30),
    ];

    for &(c1, c2, love_flag) in branch_cases {
        let mut rt = PureConnectRuntime::new();
        assert!(rt.init_vfs(base_dir, patch_dir).unwrap());
        rt.set_flag(10, love_flag); // 控制结尾 love_Arisaa/b/c 分支
        assert!(rt.load_script("Mail_chapter1_Arisa").unwrap());

        let mut choice_idx = 0usize;
        let mut phone_active = false;
        for _ in 0..200 {
            match rt.next_step() {
                StepResult::WaitClick(ws) => {
                    total_wait_clicks += 1;
                    // 前 4 屏开篇翻页期间零污染 Backlog
                    if total_wait_clicks <= 4 {
                        assert_eq!(rt.get_history(0, 10).len(), 0, "_Wait(-1, 1) 严禁污染 Backlog");
                    }
                    match ws.spr_update {
                        SprUpdate::New {
                            asset,
                            image,
                            fade_ms,
                        } => {
                            assert_eq!(image.width, 1280, "手机贴图宽度应为 1280: {}", asset);
                            assert_eq!(image.height, 720, "手机贴图高度应为 720: {}", asset);
                            assert_eq!(fade_ms, 300, "set_Mail_RS 淡入耗时应为 300ms");
                            arisa_unique_screens.insert(asset.to_ascii_lowercase());
                            phone_active = true;
                        }
                        other => panic!("set_Mail_RS WaitClick 应产出 SprUpdate::New，实际: {:?}", other),
                    }
                    assert!(
                        ws.se_plays
                            .iter()
                            .any(|(n, bytes)| n.eq_ignore_ascii_case("keitai-key_01") && !bytes.is_empty()),
                        "set_Mail_RS 必须解析并附带 keitai-Key_01 音效流"
                    );
                }
                StepResult::Choice {
                    options,
                    hide_window_when_selecting,
                    ..
                } => {
                    assert!(!options.is_empty());
                    if hide_window_when_selecting {
                        saw_hide_window_on_choice = true;
                    }
                    let pick = if choice_idx == 0 { c1 } else { c2 };
                    choice_idx += 1;
                    assert!(rt.select_choice(pick as i64));
                }
                StepResult::Dialogue(d) => {
                    match d.spr_update {
                        SprUpdate::Cleared { fade_ms } => {
                            assert_eq!(fade_ms, 200, "cls_Mail_RS 淡出耗时应为 200ms");
                            saw_cls_fade_200 = true;
                            phone_active = false;
                        }
                        SprUpdate::Unchanged => {
                            if phone_active && !d.text.trim().is_empty() {
                                saw_monologue_with_phone = true;
                            }
                        }
                        SprUpdate::New { .. } => {
                            phone_active = true;
                        }
                    }
                }
                StepResult::Terminated => {
                    if rt.current_spr_asset().is_none() && rt.last_spr_fade_ms() == 200 {
                        saw_cls_fade_200 = true;
                    }
                    break;
                }
                other => panic!("Mail_chapter1_Arisa 执行异常: {:?}", other),
            }
        }
    }

    assert!(
        total_wait_clicks >= 23,
        "多分支实跑 WaitClick 次数应 >= 23，实际: {}",
        total_wait_clicks
    );
    assert!(
        arisa_unique_screens.len() >= 21,
        "Mail_chapter1_Arisa 多分支应覆盖全部来信/回信屏，实际覆盖 {} 屏: {:?}",
        arisa_unique_screens.len(),
        arisa_unique_screens
    );
    assert!(saw_hide_window_on_choice, "必须命中 0x0167 HideWindowWhenItemSelecting(1)");
    assert!(saw_monologue_with_phone, "手机在屏期间必须出现男主吐槽对白");
    assert!(saw_cls_fade_200, "必须命中 cls_Mail_RS 200ms 淡出清屏");

    // 2. 实跑第 2 份脚本 Mail_chapter3_Moemi
    let mut rt_mo = PureConnectRuntime::new();
    assert!(rt_mo.init_vfs(base_dir, patch_dir).unwrap());
    assert!(rt_mo.load_script("Mail_chapter3_Moemi").unwrap());
    let mut mo_wait_clicks = 0usize;
    let mut mo_cleared = false;
    for _ in 0..200 {
        match rt_mo.next_step() {
            StepResult::WaitClick(ws) => {
                mo_wait_clicks += 1;
                if let SprUpdate::New { image, .. } = ws.spr_update {
                    assert_eq!((image.width, image.height), (1280, 720));
                } else {
                    panic!("Mail_chapter3_Moemi WaitClick 缺少贴图");
                }
            }
            StepResult::Choice { .. } => {
                assert!(rt_mo.select_choice(0));
            }
            StepResult::Dialogue(d) => {
                if matches!(d.spr_update, SprUpdate::Cleared { fade_ms: 200 }) {
                    mo_cleared = true;
                }
            }
            StepResult::Terminated => {
                if rt_mo.current_spr_asset().is_none() && rt_mo.last_spr_fade_ms() == 200 {
                    mo_cleared = true;
                }
                break;
            }
            other => panic!("Mail_chapter3_Moemi 执行异常: {:?}", other),
        }
    }
    assert!(mo_wait_clicks >= 10, "Mail_chapter3_Moemi 应至少经历 10 屏 WaitClick，实际: {}", mo_wait_clicks);
    assert!(mo_cleared, "Mail_chapter3_Moemi 结尾应通过 cls_Mail_RS 200ms 淡出清屏");

    // 3. 验证 data02530.arc 全部 615 张主线短信贴图在 VFS 中 100% 命中，并抽样解码校验 1280x720
    let arc_path = format!(r"{}\Archive\data02530.arc", base_dir);
    let arc = Arc20Archive::open(&arc_path).expect("应能打开 Archive/data02530.arc");
    let entries: Vec<String> = arc.iter_entries().map(|e| e.name.clone()).collect();
    assert_eq!(entries.len(), 615, "data02530.arc 应包含 615 张主线睡前短信贴图");
    let mut vfs = VirtualFileSystem::new();
    vfs.mount_directory(patch_dir).unwrap();
    vfs.mount_archive(&arc_path).unwrap();
    let mut hit_count = 0usize;
    for name in &entries {
        if vfs.read(name).is_ok() {
            hit_count += 1;
        }
    }
    assert_eq!(hit_count, 615, "615 张主线睡前短信贴图必须 100% VFS 命中");

    // 4. 验证 active_mail_spr 在 _SelectEx 与男主吐槽对白存档后的读档补绘 + 旧存档向后兼容
    let mut rt_save = PureConnectRuntime::new();
    assert!(rt_save.init_vfs(base_dir, patch_dir).unwrap());
    assert!(rt_save.load_script("Mail_chapter1_Arisa").unwrap());
    let mut reached_first_choice = false;
    for _ in 0..20 {
        match rt_save.next_step() {
            StepResult::WaitClick(_) => {}
            StepResult::Choice {
                hide_window_when_selecting,
                ..
            } => {
                assert!(hide_window_when_selecting);
                reached_first_choice = true;
                break;
            }
            other => panic!("推进至第1次短信选项异常: {:?}", other),
        }
    }
    assert!(reached_first_choice);
    let choice_save_json = rt_save.export_save_v2_json();
    let choice_save_parsed: crate::SaveStateV2 = serde_json::from_str(&choice_save_json).unwrap();
    assert!(
        choice_save_parsed
            .active_mail_spr
            .as_deref()
            .unwrap_or("")
            .eq_ignore_ascii_case("ch1_arisa0d"),
        "第 1 次选项存档必须记录 active_mail_spr=ch1_Arisa0d，实际: {:?}",
        choice_save_parsed.active_mail_spr
    );
    assert!(choice_save_parsed.hide_window_when_selecting);

    let mut rt_load_choice = PureConnectRuntime::new();
    assert!(rt_load_choice.init_vfs(base_dir, patch_dir).unwrap());
    let (loaded_choice, _) = rt_load_choice.load_save_json(&choice_save_json);
    match loaded_choice {
        StepResult::Choice {
            spr_update,
            hide_window_when_selecting,
            se_plays,
            ..
        } => {
            assert!(hide_window_when_selecting, "读档恢复短信选项必须还原 0x0167 标志");
            assert!(se_plays.is_empty(), "读档恢复不得重播历史按键音");
            match spr_update {
                SprUpdate::New { asset, image, .. } => {
                    assert!(asset.eq_ignore_ascii_case("ch1_arisa0d"));
                    assert_eq!((image.width, image.height), (1280, 720));
                }
                other => panic!("短信选项读档必须补绘手机底图，实际: {:?}", other),
            }
        }
        other => panic!("短信选项读档应返回 StepResult::Choice，实际: {:?}", other),
    }

    // 验证旧存档 JSON（无 active_mail_spr / hide_window_when_selecting 字段）100% 兼容反序列化
    let legacy_json = r#"{"save_version":2,"script_name":"01_prologue1","dialogue_index":1,"choices":[],"speaker":null,"text":"hi"}"#;
    let legacy_parsed: crate::SaveStateV2 = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(legacy_parsed.active_mail_spr, None);
    assert!(!legacy_parsed.hide_window_when_selecting);

    // 5. 验证非短信 02_prologue2 的非零偏移 _DrawSPR 不污染 SprOverlay
    let mut rt_p2 = PureConnectRuntime::new();
    assert!(rt_p2.init_vfs(base_dir, patch_dir).unwrap());
    assert!(rt_p2.load_script("02_prologue2").unwrap());
    for _ in 0..80 {
        match rt_p2.next_step() {
            StepResult::Dialogue(d) => {
                assert!(
                    matches!(d.spr_update, SprUpdate::Unchanged),
                    "02_prologue2 非短信偏移 _DrawSPR 必须被白名单拦截，不得产出 SprUpdate::New"
                );
            }
            StepResult::Choice { .. } => {
                let _ = rt_p2.select_choice(0);
            }
            StepResult::Terminated => break,
            _ => {}
        }
    }

    // 6. 实跑 01_prologue1 全篇字节码验证 eye_long 的 _SetLocalShortStrings(0, ...) + _DrawScene("%0", ...) 真实替换生效
    {
        use pure_connect_core::vm::{
            OwnedBgiScript, ScriptLoader, VfsScriptLoader, VmEngine, VmEvent, VmState,
        };
        use std::sync::Arc;
        let vfs_arc = Arc::new({
            let mut v = VirtualFileSystem::new();
            v.mount_directory(patch_dir).unwrap();
            let mut arcs: Vec<_> = std::fs::read_dir(format!(r"{}\Archive", base_dir))
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.eq_ignore_ascii_case("arc"))
                        .unwrap_or(false)
                })
                .collect();
            arcs.sort();
            for p in arcs {
                let _ = v.mount_archive(&p);
            }
            v
        });
        let loader = Arc::new(VfsScriptLoader::new(Arc::clone(&vfs_arc)));
        let bytes = loader
            .load_script("01_prologue1")
            .unwrap()
            .expect("应能加载 01_prologue1");
        let owned = OwnedBgiScript::parse(bytes).unwrap();
        let mut vm = VmEngine::from_owned(owned);
        vm.set_script_loader(loader);
        vm.set_script_name("01_prologue1");

        let mut saw_eye_long_bg_expanded = false;
        let mut expanded_bg_name = String::new();
        for _ in 0..200_000 {
            if vm.state() == &VmState::Terminated {
                break;
            }
            if vm.state() == &VmState::WaitingForInput || vm.state() == &VmState::WaitingForWaitClick {
                vm.resume();
            } else if matches!(vm.state(), VmState::WaitingForChoice { .. }) {
                let seq = vm.choice_seq();
                vm.submit_choice(seq, 0);
            }
            if let Some(ev) = vm.step().unwrap() {
                if let VmEvent::SceneChange { bg } = ev {
                    assert!(
                        !bg.contains('%'),
                        "_DrawScene 背景名不得残留未展开占位符: {}",
                        bg
                    );
                    if let Some(slot0) = vm.local_short_string(0) {
                        if vm.script_name().eq_ignore_ascii_case("eye_long")
                            && bg.eq_ignore_ascii_case(slot0)
                        {
                            saw_eye_long_bg_expanded = true;
                            expanded_bg_name = bg;
                            break;
                        }
                    }
                }
            }
        }
        assert!(
            saw_eye_long_bg_expanded,
            "01_prologue1 必须触发 _SetLocalShortStrings(0, ...) + eye_long _DrawScene(\"%0\") 展开"
        );
        let img = rt_p2
            .resolve_image(&expanded_bg_name)
            .expect("eye_long 展开后的背景图必须在 VFS 中成功解码");
        assert!(
            img.width >= 1280 && img.height >= 720,
            "eye_long 展开背景 {} 尺寸异常: {}x{}",
            expanded_bg_name,
            img.width,
            img.height
        );
    }
}

#[test]
fn test_batch2_window_mail_multi_receive_cancel_writeback_and_zero_pollution() {
    let base_dir = Path::new(r"U:\anti\projects\PURExCONNECT");
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");
    if !base_dir.is_dir() || !patch_dir.is_dir() {
        return;
    }

    // 1. 直接验证 Re_Mail_chapter1 经 0x001A/0x0016/0x00F7/0x0017/0x001B 多封连读且 eval_stack 严格归零
    {
        let mut rt_raw = PureConnectRuntime::new();
        assert!(rt_raw
            .init_vfs(base_dir.to_str().unwrap(), patch_dir.to_str().unwrap())
            .expect("init_vfs 失败"));
        assert!(rt_raw.load_script("Re_Mail_chapter1").expect("加载 Re_Mail_chapter1 失败"));
        rt_raw.set_flag(200, 2);
        rt_raw.set_flag(201, 1);
        rt_raw.set_flag(300, 2);
        rt_raw.set_flag(301, 2);

        let mut received_assets = Vec::new();
        for _ in 0..20 {
            match rt_raw.next_step() {
                StepResult::WaitClick(w) => {
                    if let SprUpdate::New { asset, .. } = w.spr_update {
                        received_assets.push(asset.to_ascii_lowercase());
                    }
                    let stack_len = rt_raw.session().unwrap().engine().eval_stack_len();
                    assert_eq!(stack_len, 0, "多封未读回信挂起时 eval_stack 必须严格归零");
                }
                StepResult::Terminated => break,
                other => panic!("Re_Mail_chapter1 预期 WaitClick/Terminated，实际 {:?}", other),
            }
        }
        assert_eq!(
            received_assets.len(),
            2,
            "2 封未读回信必须精确展示 2 屏手机图，实际: {:?}",
            received_assets
        );
        assert!(received_assets[0].contains("so_chaptar1_1b"));
        assert!(received_assets[1].contains("ay_chaptar1_1b"));
        assert_eq!(rt_raw.get_flag(300), 3);
        assert_eq!(rt_raw.get_flag(301), 3);
        assert_eq!(rt_raw.get_flag(200), 1, "读完回信后 FLAG[200] 应还原为 FLAG[201]=1");
        assert_eq!(
            rt_raw.session().unwrap().engine().eval_stack_len(),
            0,
            "Re_Mail_chapter1 结束后 eval_stack 必须严格归零"
        );
    }

    // 2. 验证方案 4A 独立 mail_vm：先读 2 封未读回信 (FLAG[200]: 2->1) 后在发信选单选“不发邮件”取消退回，
    //    仍无条件回写 flags[200..=201] 与 flags[300..=419]，且主引擎 call_stack/FLAG[0]/pending_dialogue/Backlog 100% 零污染
    let mut rt = PureConnectRuntime::new();
    assert!(rt
        .init_vfs(base_dir.to_str().unwrap(), patch_dir.to_str().unwrap())
        .expect("init_vfs 失败"));
    assert!(rt.load_script("04_chapter1_1").expect("加载 04_chapter1_1 失败"));

    // 推进到主线第 1 句对白 (WaitingForInput)
    let first_dlg = match rt.next_step() {
        StepResult::Dialogue(d) => d,
        other => panic!("预期 04_chapter1_1 首句对白，实际 {:?}", other),
    };
    assert_eq!(rt.get_state(), "WaitingForInput");

    // 注入哨兵：FLAG[0]=77 校验退出后不被 _SelectEx 污染；FLAG[80]=1, FLAG[200]=2, FLAG[201]=1, FLAG[210]=111, FLAG[300]=2, FLAG[301]=2
    rt.set_flag(0, 77);
    rt.set_flag(80, 1);
    rt.set_flag(200, 2);
    rt.set_flag(201, 1);
    rt.set_flag(210, 111);
    rt.set_flag(300, 2);
    rt.set_flag(301, 2);

    let main_pc_before = rt.session().unwrap().engine().pc();
    let main_depth_before = rt.session().unwrap().engine().call_depth();
    let main_dlg_cnt_before = rt.session().unwrap().engine().dialogue_counter();
    let main_hist_len_before = rt.get_history(0, 1000).len();
    let main_pending_text_before = rt
        .session()
        .unwrap()
        .engine()
        .pending_dialogue()
        .map(|d| d.text.clone());

    // 打开独立 mail_vm：第 1 步应为第 1 封未读回信 WaitClick
    let s1 = rt.open_mail_window();
    assert!(rt.is_mail_window_open(), "打开后 mail_vm 必须处于活跃态");
    match s1 {
        StepResult::WaitClick(w) => {
            assert!(w.mail_mode);
            assert!(matches!(w.spr_update, SprUpdate::New { .. }));
        }
        other => panic!("预期第 1 封未读回信 WaitClick，实际 {:?}", other),
    }

    // 第 2 步应为第 2 封未读回信 WaitClick
    match rt.next_step() {
        StepResult::WaitClick(w) => {
            assert!(w.mail_mode);
            assert!(matches!(w.spr_update, SprUpdate::New { .. }));
        }
        other => panic!("预期第 2 封未读回信 WaitClick，实际 {:?}", other),
    }

    // 第 3 步读完回信进入 Mail_chapter111 发信选单
    let (choice_seq, cancel_idx) = match rt.next_step() {
        StepResult::Choice {
            options,
            disabled_options,
            mail_mode,
            ..
        } => {
            assert!(mail_mode);
            assert_eq!(options, vec!["给空发邮件", "给步海发邮件", "不发邮件"]);
            assert!(disabled_options.is_empty());
            (rt.current_choice_seq(), options.len() - 1)
        }
        other => panic!("预期进入 Mail_chapter111 发信选单，实际 {:?}", other),
    };

    // 点选“不发邮件”取消退回
    assert!(rt.submit_choice(choice_seq, cancel_idx as i64));
    match rt.next_step() {
        StepResult::MailClosed { mail_btn_state, .. } => {
            assert_eq!(mail_btn_state, 1, "读完 2 封未读回信后取消发信，FLAG[200] 应从 2 降为 1");
        }
        other => panic!("预期取消后返回 MailClosed，实际 {:?}", other),
    }
    assert!(!rt.is_mail_window_open(), "退出后 mail_vm 必须立即释放");

    // 校验无条件回写
    assert_eq!(rt.get_flag(200), 1);
    assert_eq!(rt.get_flag(201), 1);
    assert_eq!(rt.get_flag(300), 3);
    assert_eq!(rt.get_flag(301), 3);

    // 校验主引擎 100% 零污染
    assert_eq!(rt.get_flag(0), 77, "主引擎 FLAG[0] 绝不得被 window_Mail 的 _SelectEx 覆写");
    assert_eq!(rt.get_state(), "WaitingForInput");
    assert_eq!(rt.session().unwrap().engine().pc(), main_pc_before);
    assert_eq!(rt.session().unwrap().engine().call_depth(), main_depth_before);
    assert_eq!(
        rt.session().unwrap().engine().dialogue_counter(),
        main_dlg_cnt_before
    );
    assert_eq!(rt.get_history(0, 1000).len(), main_hist_len_before);
    assert_eq!(
        rt.session()
            .unwrap()
            .engine()
            .pending_dialogue()
            .map(|d| d.text.clone()),
        main_pending_text_before
    );
    assert_eq!(
        main_pending_text_before.as_deref(),
        Some(first_dlg.text.as_str())
    );
}

#[test]
fn test_batch2_mail_chapter111_disable_item_and_mail_vm_guards() {
    let base_dir = Path::new(r"U:\anti\projects\PURExCONNECT");
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");
    if !base_dir.is_dir() || !patch_dir.is_dir() {
        return;
    }

    let mut rt = PureConnectRuntime::new();
    assert!(rt
        .init_vfs(base_dir.to_str().unwrap(), patch_dir.to_str().unwrap())
        .expect("init_vfs 失败"));
    assert!(rt.load_script("04_chapter1_1").expect("加载 04_chapter1_1 失败"));
    let _ = rt.next_step(); // WaitingForInput

    // 门禁校验：FLAG[200]==0 与 FLAG[200]==3 均不可打开
    rt.set_flag(200, 0);
    assert!(!rt.can_open_mail_window());
    assert!(matches!(rt.open_mail_window(), StepResult::Error(_)));
    rt.set_flag(200, 3);
    assert!(!rt.can_open_mail_window());
    assert!(matches!(rt.open_mail_window(), StepResult::Error(_)));

    // 构造可发信时段 111
    rt.set_flag(80, 1);
    rt.set_flag(200, 1);
    rt.set_flag(201, 1);
    rt.set_flag(210, 111);
    rt.set_flag(300, 0);
    rt.set_flag(301, 0);
    assert!(rt.can_open_mail_window());

    let cseq1 = match rt.open_mail_window() {
        StepResult::Choice {
            options,
            disabled_options,
            mail_mode,
            ..
        } => {
            assert!(mail_mode);
            assert_eq!(options, vec!["给空发邮件", "给步海发邮件", "不发邮件"]);
            assert!(disabled_options.is_empty());
            rt.current_choice_seq()
        }
        other => panic!("预期打开即进入 Mail_chapter111 选单，实际 {:?}", other),
    };

    // mail_vm 活跃期间：严禁存档、严禁重复打开
    assert!(
        rt.try_build_save_v2().is_none(),
        "mail_vm 活跃期间 try_build_save_v2 必须拦截返回 None"
    );
    assert!(
        matches!(rt.open_mail_window(), StepResult::Error(_)),
        "mail_vm 活跃期间禁止重复打开"
    );

    // 发给下标 0（空）
    assert!(rt.submit_choice(cseq1, 0));
    match rt.next_step() {
        StepResult::WaitClick(w) => {
            assert!(w.mail_mode);
            match w.spr_update {
                SprUpdate::New { asset, .. } => {
                    assert!(asset.to_ascii_lowercase().contains("so_chaptar1_1a"));
                }
                other => panic!("预期发信手机图 SO_chaptar1_1a，实际 {:?}", other),
            }
        }
        other => panic!("预期发信手机屏 WaitClick，实际 {:?}", other),
    }

    // 翻页后回到同一选单：下标 0 必须被 0x0166 禁用，且 3 个选项完整保留绝不删项
    let cseq2 = match rt.next_step() {
        StepResult::Choice {
            options,
            disabled_options,
            mail_mode,
            ..
        } => {
            assert!(mail_mode);
            assert_eq!(
                options,
                vec!["给空发邮件", "给步海发邮件", "不发邮件"],
                "禁用项绝不得从选项数组中剔除，确保下标不错位"
            );
            assert_eq!(disabled_options, vec![0], "已发过邮件的 0 号选项必须被 0x0166 禁用");
            rt.current_choice_seq()
        }
        other => panic!("预期返回 Mail_chapter111 选单，实际 {:?}", other),
    };

    // 强行提交被禁用的下标 0 必须被底层拒绝
    assert!(
        !rt.submit_choice(cseq2, 0),
        "底层 submit_choice 必须拒绝已禁用下标 0"
    );

    // 提交下标 1（步海），验证下标 100% 精确不串位
    assert!(rt.submit_choice(cseq2, 1));
    match rt.next_step() {
        StepResult::WaitClick(w) => {
            assert!(w.mail_mode);
            match w.spr_update {
                SprUpdate::New { asset, .. } => {
                    assert!(asset.to_ascii_lowercase().contains("ay_chaptar1_1a"));
                }
                other => panic!("预期发信手机图 AY_chaptar1_1a，实际 {:?}", other),
            }
        }
        other => panic!("预期步海发信手机屏 WaitClick，实际 {:?}", other),
    }

    // 全部发完后自动调用 set_Mail_off 并退出
    match rt.next_step() {
        StepResult::MailClosed { mail_btn_state, .. } => {
            assert_eq!(mail_btn_state, 0, "本时段全发完后 set_Mail_off 应将 FLAG[200] 置 0");
        }
        other => panic!("预期全发完后退出返回 MailClosed，实际 {:?}", other),
    }
    assert_eq!(rt.get_flag(300), 1);
    assert_eq!(rt.get_flag(301), 1);
    assert_eq!(rt.get_flag(200), 0);
    assert_eq!(rt.get_flag(201), 0);
}

#[test]
fn test_batch2_mail_on_notify_assets_and_load_replay_suppression() {
    let base_dir = Path::new(r"U:\anti\projects\PURExCONNECT");
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");
    if !base_dir.is_dir() || !patch_dir.is_dir() {
        return;
    }

    let mut rt = PureConnectRuntime::new();
    assert!(rt
        .init_vfs(base_dir.to_str().unwrap(), patch_dir.to_str().unwrap())
        .expect("init_vfs 失败"));

    // 1. 验证原版信封三态图标 (48x48) 与 mail_on_a (195x240) 均可直接无损解码
    for icon in ["SGMsgWnd000200", "SGMsgWnd000201", "SGMsgWnd000202"] {
        let img = rt.load_ui_image(icon).unwrap_or_else(|| panic!("{} 解码失败", icon));
        assert_eq!((img.width, img.height), (48, 48), "{} 尺寸应为 48x48", icon);
    }
    let bubble = rt.load_ui_image("mail_on_a").expect("mail_on_a 解码失败");
    assert_eq!((bubble.width, bubble.height), (195, 240));

    // 2. 跑 04_chapter1_1 直到首次遇到 set_Mail_on 触发 mail_on_notify == true 且 mail_btn_state == 1
    assert!(rt.load_script("04_chapter1_1").expect("加载 04_chapter1_1 失败"));
    let mut triggered_save_json: Option<String> = None;
    for _ in 0..800 {
        match rt.next_step() {
            StepResult::Dialogue(d) => {
                if d.mail_on_notify {
                    assert_eq!(d.mail_btn_state, 1, "set_Mail_on 后 FLAG[200] 必须为 1");
                    triggered_save_json = Some(rt.export_save_v2_json());
                    break;
                }
            }
            StepResult::Choice { .. } => {
                let seq = rt.current_choice_seq();
                let _ = rt.submit_choice(seq, 0);
            }
            StepResult::WaitClick(_) => {}
            StepResult::Terminated => break,
            StepResult::Error(e) => panic!("04_chapter1_1 推进异常: {}", e),
            _ => {}
        }
    }
    let save_json = triggered_save_json.expect("04_chapter1_1 必须触发 set_Mail_on 的 mail_on_notify");

    // 3. 读档重放验证：FLAG[200]==1 立即还原，但 mail_on_notify 必须为 false（禁止读档误弹旧气泡）
    let mut rt_reload = PureConnectRuntime::new();
    assert!(rt_reload
        .init_vfs(base_dir.to_str().unwrap(), patch_dir.to_str().unwrap())
        .expect("init_vfs 失败"));
    let (step, _) = rt_reload.load_save_json(&save_json);
    match step {
        StepResult::Dialogue(d) => {
            assert_eq!(d.mail_btn_state, 1, "读档后必须立即恢复 FLAG[200]==1");
            assert!(!d.mail_on_notify, "读档重放严禁误触旧 mail_on_A 气泡动画");
        }
        other => panic!("读档预期恢复为 Dialogue，实际 {:?}", other),
    }
}

/// 辅助：推进 05_chapter1_2 直至通过 0x00F0 进入 Mail_chapter1_Arisa 的 _SelectEx 选项挂起态
/// （同时确保此前至少经历过若干前置全局选项，验证红线 1/2/3 不串分支、不截断历史、不丢视觉）
fn advance_05_to_arisa_mail_choice(
    base_dir: &str,
    patch_dir: &str,
) -> (PureConnectRuntime, String, String, Vec<usize>) {
    let mut rt = PureConnectRuntime::new();
    assert!(rt.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    // 先在 05_chapter1_2 之前置入若干历史选项（模拟从序章/第1章前半自然玩到此处）
    assert!(rt.load_script("05_chapter1_2").expect("加载 05_chapter1_2 失败"));

    let mut entered_arisa_mail = false;
    let mut last_parent_bg = String::new();
    let mut last_parent_bgm = String::new();
    for _ in 0..2500 {
        match rt.next_step() {
            StepResult::Dialogue(d) => {
                let cur = rt.get_save_state().0;
                if cur.eq_ignore_ascii_case("05_chapter1_2") {
                    if let Some(bg) = d.bg_key.clone() {
                        last_parent_bg = bg;
                    }
                    if let Some(bgm) = d.bgm_name.clone() {
                        last_parent_bgm = bgm;
                    }
                }
            }
            StepResult::WaitClick(_) => {}
            StepResult::Choice {
                options,
                hide_window_when_selecting,
                ..
            } => {
                let cur = rt.get_save_state().0;
                if cur.eq_ignore_ascii_case("Mail_chapter1_Arisa") {
                    assert!(
                        hide_window_when_selecting,
                        "Mail_chapter1_Arisa _SelectEx 必须开启 hide_window_when_selecting"
                    );
                    entered_arisa_mail = true;
                    break;
                }
                let seq = rt.current_choice_seq();
                // 05_chapter1_2 中 5 选 1 节点：0=空, 1=志帆, 2=萌美, 3=亚里沙 (进入 Mail_chapter1_Arisa), 4=步海
                let pick = if options.len() >= 4 { 3 } else { 0 };
                assert!(rt.submit_choice(seq, pick), "05_chapter1_2 前置选项提交失败");
            }
            StepResult::Terminated => break,
            StepResult::Error(e) => panic!("05_chapter1_2 推进异常: {}", e),
            _ => {}
        }
    }
    assert!(
        entered_arisa_mail,
        "05_chapter1_2 必须通过 0x00F0 自然进入 Mail_chapter1_Arisa 的 _SelectEx"
    );
    let pre_choices = rt.get_save_state().2;
    (rt, last_parent_bg, last_parent_bgm, pre_choices)
}

#[test]
fn test_audit_fix_1b_and_1c_subscript_mail_save_load_and_return_resave() {
    let base_dir = r"U:\anti\projects\PURExCONNECT\Archive";
    let patch_dir = r"U:\anti\temp\exe_unpacked";
    if !Path::new(base_dir).is_dir() || !Path::new(patch_dir).is_dir() {
        return;
    }

    // 1. 推进 05_chapter1_2 进入 Mail_chapter1_Arisa _SelectEx，并刻意选择非 0 分支 (choice=1) 严查分支错位
    let (mut rt, parent_bg, parent_bgm, pre_choices) =
        advance_05_to_arisa_mail_choice(base_dir, patch_dir);
    let seq = rt.current_choice_seq();
    assert!(rt.submit_choice(seq, 1), "Mail_chapter1_Arisa 选择分支 1 失败");
    let mut expected_sub_choices = vec![1usize];

    // 推进至子脚本第 5 句男主吐槽对白（期间若遇第 2 次短信回复选项则继续选分支 1）
    let mut target_text = String::new();
    let mut target_idx = 0usize;
    let mut dlg_count = 0usize;
    for _ in 0..40 {
        match rt.next_step() {
            StepResult::Dialogue(d) => {
                dlg_count += 1;
                target_text = d.text;
                target_idx = d.index;
                if dlg_count == 5 {
                    break;
                }
            }
            StepResult::WaitClick(_) => {}
            StepResult::Choice { .. } => {
                let s = rt.current_choice_seq();
                assert!(rt.submit_choice(s, 1), "Mail_chapter1_Arisa 子脚本后续选项提交失败");
                expected_sub_choices.push(1);
            }
            other => panic!("Mail_chapter1_Arisa 第 5 句吐槽前不应异常中断: {:?}", other),
        }
    }
    assert_eq!(dlg_count, 5, "Mail_chapter1_Arisa 必须能推进到第 5 句吐槽");

    // 2. 用例 1B：在子脚本第 5 句存档 -> 校验 SaveStateV2 元数据 -> 新会话读档验证
    let save_1b_json = rt.export_save_v2_json();
    let save_1b: crate::SaveStateV2 =
        serde_json::from_str(&save_1b_json).expect("1B 存档 JSON 解析失败");
    assert!(
        save_1b.script_name.eq_ignore_ascii_case("Mail_chapter1_Arisa"),
        "1B 存档应位于 Mail_chapter1_Arisa，实际 {}",
        save_1b.script_name
    );
    assert_eq!(save_1b.script_dialogue_index, Some(5));
    assert_eq!(
        save_1b.script_choices, expected_sub_choices,
        "1B 存档 script_choices 必须仅记录子脚本局部选项"
    );
    assert_eq!(
        save_1b.call_stack.len(),
        1,
        "1B 存档 call_stack 必须保留父帧 05_chapter1_2"
    );
    assert!(
        save_1b.call_stack[0].script_dialogue_index > 0,
        "CallFrameSave 必须持久化父帧非零 script_dialogue_index"
    );
    assert_eq!(
        save_1b.call_stack[0].script_choices, pre_choices,
        "CallFrameSave 必须持久化父帧 script_choices"
    );
    assert!(
        save_1b.active_bg.is_some(),
        "1B 存档必须持久化父脚本 active_bg"
    );
    assert!(
        save_1b.active_bgm.is_some(),
        "1B 存档必须持久化父脚本 active_bgm"
    );

    let mut rt_1b = PureConnectRuntime::new();
    assert!(rt_1b.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    let (res_1b, _) = rt_1b.load_save_json(&save_1b_json);
    match res_1b {
        StepResult::Dialogue(d) => {
            assert_eq!(d.text, target_text, "1B 读档必须精准落回第 5 句吐槽且分支未错位");
            assert_eq!(d.index, target_idx, "1B 读档全局 dialogue_index 必须一致");
            assert!(
                matches!(d.bg_update, BgUpdate::New(_)),
                "1B 读档必须还原父脚本夜晚房间 active_bg"
            );
            if !parent_bg.is_empty() {
                assert_eq!(d.bg_key.as_deref(), Some(parent_bg.as_str()));
            }
            assert!(
                d.bgm_changed && d.bgm_name.is_some() && d.bgm_data.is_some(),
                "1B 读档必须还原父脚本夜晚房间 active_bgm"
            );
            if !parent_bgm.is_empty() {
                assert_eq!(d.bgm_name.as_deref(), Some(parent_bgm.as_str()));
            }
        }
        other => panic!("1B 读档预期恢复为 Dialogue，实际 {:?}", other),
    }
    assert_eq!(
        rt_1b.get_save_state().2,
        save_1b.choices,
        "1B 读档后全局 choice_history 严禁截断"
    );

    // 3. 用例 1C：接续 1B 读档会话打完短信 0x001B RET 回 05_chapter1_2，再推 3 句存新档并读新档
    let mut returned_to_parent = false;
    let mut post_return_count = 0usize;
    let mut post_return_text = String::new();
    for _ in 0..80 {
        match rt_1b.next_step() {
            StepResult::Dialogue(d) => {
                let cur = rt_1b.get_save_state().0;
                if cur.eq_ignore_ascii_case("05_chapter1_2") {
                    returned_to_parent = true;
                    post_return_count += 1;
                    post_return_text = d.text;
                    if post_return_count == 3 {
                        break;
                    }
                }
            }
            StepResult::WaitClick(_) => {}
            StepResult::Choice { .. } => {
                let s = rt_1b.current_choice_seq();
                assert!(rt_1b.submit_choice(s, 0));
            }
            other => panic!("1C 接续推进异常: {:?}", other),
        }
    }
    assert!(
        returned_to_parent && post_return_count == 3,
        "1C 必须经 0x001B RET 返回 05_chapter1_2 并推进 3 句"
    );

    let save_1c_json = rt_1b.export_save_v2_json();
    let save_1c: crate::SaveStateV2 =
        serde_json::from_str(&save_1c_json).expect("1C 存档 JSON 解析失败");
    assert!(save_1c.script_name.eq_ignore_ascii_case("05_chapter1_2"));
    assert!(save_1c.call_stack.is_empty());
    assert!(
        save_1c.script_choices.ends_with(&expected_sub_choices),
        "0x001B RET 必须将子脚本选项 {:?} 归并到父帧 script_choices 尾部，实际 {:?}",
        expected_sub_choices,
        save_1c.script_choices
    );
    assert!(
        save_1c.script_dialogue_index.unwrap_or(0) > save_1b.call_stack[0].script_dialogue_index,
        "1C 存档 script_dialogue_index 必须严格递增不倒流"
    );

    let mut rt_1c = PureConnectRuntime::new();
    assert!(rt_1c.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    let (res_1c, _) = rt_1c.load_save_json(&save_1c_json);
    match res_1c {
        StepResult::Dialogue(d) => {
            assert_eq!(
                d.text, post_return_text,
                "1C 新会话读档必须穿过 0x00F0 与 0x001B 精准落回第 3 句不倒流"
            );
        }
        other => panic!("1C 读档预期恢复为 Dialogue，实际 {:?}", other),
    }
    assert_eq!(
        rt_1c.get_save_state().2,
        save_1c.choices,
        "1C 读档后全局 choice_history 必须完整保留"
    );
}

#[test]
fn test_audit_fix_1a_cold_load_waiting_for_choice_mail_subscript() {
    let base_dir = r"U:\anti\projects\PURExCONNECT\Archive";
    let patch_dir = r"U:\anti\temp\exe_unpacked";
    if !Path::new(base_dir).is_dir() || !Path::new(patch_dir).is_dir() {
        return;
    }

    let (mut rt, parent_bg, parent_bgm, _) =
        advance_05_to_arisa_mail_choice(base_dir, patch_dir);
    let save_json = rt.export_save_v2_json();
    let parsed: crate::SaveStateV2 = serde_json::from_str(&save_json).expect("JSON 解析失败");
    assert!(parsed.pending_choice.is_some());
    assert!(parsed.active_bg.is_some());
    assert!(parsed.active_bgm.is_some());
    assert!(parsed.active_mail_spr.is_some());

    // 标题冷启动（全新 Runtime）读 WaitingForChoice 睡前短信选项档
    let mut rt_cold = PureConnectRuntime::new();
    assert!(rt_cold.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    let (step, visuals) = rt_cold.load_save_json(&save_json);
    match step {
        StepResult::Choice {
            spr_update,
            hide_window_when_selecting,
            ..
        } => {
            assert!(hide_window_when_selecting);
            assert!(
                matches!(spr_update, SprUpdate::New { .. }),
                "1A 冷启动读睡前短信选项档必须还原 active_mail_spr，实际 {:?}",
                spr_update
            );
        }
        other => panic!("1A 冷启动读档应返回 Choice，实际 {:?}", other),
    }
    let vis = visuals.expect("1A 冷启动读睡前短信选项档必须返回舞台视觉 visuals");
    assert!(
        matches!(vis.bg_update, BgUpdate::New(_)),
        "1A 冷启动读睡前短信选项档必须还原父脚本 active_bg"
    );
    if !parent_bg.is_empty() {
        assert_eq!(vis.bg_key.as_deref(), Some(parent_bg.as_str()));
    }
    assert!(
        vis.bgm_changed && vis.bgm_name.is_some() && vis.bgm_data.is_some(),
        "1A 冷启动读睡前短信选项档必须还原父脚本 active_bgm"
    );
    if !parent_bgm.is_empty() {
        assert_eq!(vis.bgm_name.as_deref(), Some(parent_bgm.as_str()));
    }
}

#[test]
fn test_audit_fix_1a_cold_load_waiting_for_choice_normal_with_sprites() {
    let base_dir = r"U:\anti\projects\PURExCONNECT\Archive";
    let patch_dir = r"U:\anti\temp\exe_unpacked";
    if !Path::new(base_dir).is_dir() || !Path::new(patch_dir).is_dir() {
        return;
    }

    // 推进 03_prologue3 寻找普通剧情带立绘的 WaitingForChoice 节点
    let mut rt = PureConnectRuntime::new();
    assert!(rt.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    assert!(rt.load_script("03_prologue3").expect("加载 03_prologue3 失败"));

    let mut choice_save_json: Option<String> = None;
    for _ in 0..800 {
        match rt.next_step() {
            StepResult::Dialogue(_) | StepResult::WaitClick(_) => {}
            StepResult::Choice {
                hide_window_when_selecting,
                ..
            } => {
                if !hide_window_when_selecting {
                    let v2 = rt.build_save_v2();
                    if !v2.active_sprites.is_empty() && v2.active_bg.is_some() {
                        choice_save_json = Some(rt.export_save_v2_json());
                        break;
                    }
                }
                let seq = rt.current_choice_seq();
                let _ = rt.submit_choice(seq, 0);
            }
            StepResult::Terminated => break,
            StepResult::Error(e) => panic!("03_prologue3 推进异常: {}", e),
            _ => {}
        }
    }
    let save_json =
        choice_save_json.expect("03_prologue3 必须包含带立绘与背景的普通剧情选项节点");
    let parsed: crate::SaveStateV2 = serde_json::from_str(&save_json).expect("JSON 解析失败");
    assert!(!parsed.active_sprites.is_empty());

    // 标题冷启动（全新 Runtime）读普通剧情带立绘选项档
    let mut rt_cold = PureConnectRuntime::new();
    assert!(rt_cold.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    let (step, visuals) = rt_cold.load_save_json(&save_json);
    assert!(
        matches!(step, StepResult::Choice { .. }),
        "1A 普通选项档读档必须返回 Choice，实际 {:?}",
        step
    );
    let vis = visuals.expect("1A 普通带立绘选项档读档必须返回 visuals");
    assert!(
        matches!(vis.bg_update, BgUpdate::New(_)),
        "1A 普通带立绘选项档必须还原 active_bg"
    );
    assert!(
        !vis.changed_slots.is_empty(),
        "1A 普通带立绘选项档必须还原 active_sprites"
    );
    for sp in &parsed.active_sprites {
        assert!(
            vis.changed_slots.contains_key(&sp.slot_id),
            "立绘槽位 {} 必须完整还原",
            sp.slot_id
        );
    }
}

#[test]
fn test_audit_fix_legacy_v1_v2_compat_and_subscript_no_misconsume() {
    let base_dir = r"U:\anti\projects\PURExCONNECT\Archive";
    let patch_dir = r"U:\anti\temp\exe_unpacked";
    if !Path::new(base_dir).is_dir() || !Path::new(patch_dir).is_dir() {
        return;
    }

    // 1. 旧版 v1 存档兼容
    let legacy_v1 = r#"{"script_name":"01_prologue1","dialogue_index":4,"choices":[]}"#;
    let mut rt1 = PureConnectRuntime::new();
    assert!(rt1.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    let (r1, _) = rt1.load_save_json(legacy_v1);
    assert!(matches!(r1, StepResult::Dialogue(_)));

    // 2. 旧版 v2 顶层存档（不含 script_choices / active_bg / active_bgm / active_sprites）兼容
    let legacy_v2_top = r#"{"save_version":2,"script_name":"04_chapter1_1","dialogue_index":5,"choices":[0],"entry_flags":{},"flags":{"80":1},"choice_seq":2}"#;
    let mut rt2 = PureConnectRuntime::new();
    assert!(rt2.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    let (r2, _) = rt2.load_save_json(legacy_v2_top);
    assert!(matches!(r2, StepResult::Dialogue(_)));
    assert_eq!(rt2.get_save_state().2, vec![0], "旧版 v2 顶层存档必须保留 choices");

    // 3. 红线 2 精确降级边界：旧版 v2 子脚本存档（call_stack 非空、script_choices 为空、choices 非空）
    //    在重放子脚本 Mail_chapter1_Arisa 时绝不误吞顶层旧历史 choices[0]=1，而是停在子脚本选项态并完整保留 choices；
    //    并且必须恢复 call_stack，确保玩家重新选完打完短信后 0x001B RET 正常回到父脚本 05_chapter1_2 而非 Terminated 死机！
    let legacy_v2_sub = r#"{"save_version":2,"script_name":"Mail_chapter1_Arisa","dialogue_index":50,"script_dialogue_index":3,"choices":[1,0],"entry_flags":{},"flags":{},"choice_seq":3,"call_stack":[{"script_name":"05_chapter1_2","return_pc":1024,"is_local":false}]}"#;
    let mut rt3 = PureConnectRuntime::new();
    assert!(rt3.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    let (r3, _) = rt3.load_save_json(legacy_v2_sub);
    assert!(
        matches!(r3, StepResult::Choice { .. }),
        "旧版 v2 子脚本存档（无 script_choices）不应误吞顶层 choices[0] 越过子脚本选项，实际 {:?}",
        r3
    );
    assert_eq!(
        rt3.get_save_state().2,
        vec![1, 0],
        "旧版 v2 子脚本存档读档后必须完整保留全局 choice_history"
    );
    // 验证降级停在子脚本选项后，call_stack 仍已恢复（导出 v2 存档 call_stack 长度为 1 且指向 05_chapter1_2）
    let v2_after_degrade = rt3.build_save_v2();
    assert_eq!(
        v2_after_degrade.call_stack.len(),
        1,
        "旧版 v2 子脚本降级停在 WaitingForChoice 时必须恢复 call_stack，防止 0x001B RET 死机"
    );
    assert!(
        v2_after_degrade.call_stack[0]
            .script_name
            .eq_ignore_ascii_case("05_chapter1_2"),
        "恢复的 call_stack 父帧必须为 05_chapter1_2"
    );
}

#[test]
fn test_review_fix_silent_save_stops_title_bgm_and_clears_stale_choice_visuals() {
    let base_dir = r"U:\anti\projects\PURExCONNECT\Archive";
    let patch_dir = r"U:\anti\temp\exe_unpacked";
    if !Path::new(base_dir).is_dir() || !Path::new(patch_dir).is_dir() {
        return;
    }

    // 1. 冷启动（prev_bgm == None，模拟标题画面正在播 title_bgm）读入静音普通存档：
    //    必须返回 bgm_changed == true && bgm_name.is_none()，通知前端停掉 title_bgm 防串音
    let silent_dialogue_v2 = r#"{"save_version":2,"script_name":"01_prologue1","dialogue_index":1,"script_dialogue_index":1,"choices":[],"entry_flags":{},"flags":{},"choice_seq":1}"#;
    let mut rt_silent = PureConnectRuntime::new();
    assert!(rt_silent.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    let (res, _) = rt_silent.load_save_json(silent_dialogue_v2);
    match res {
        StepResult::Dialogue(d) => {
            if d.bgm_name.is_none() {
                assert!(
                    d.bgm_changed,
                    "冷启动读入静音普通存档时必须置 bgm_changed=true 以停掉标题页 title_bgm"
                );
            }
        }
        other => panic!("预期恢复 Dialogue，实际 {:?}", other),
    }

    // 2. 在 03_prologue3 找到带立绘的普通选项档，然后构造“弹选项前已清空立绘/头像/BGM，仅留 active_bg”的变体存档：
    //    冷启动读档后，build_visual_snapshot_from_save 必须清空重放前句残留的 changed_slots / face_texture / bgm_name
    let mut rt = PureConnectRuntime::new();
    assert!(rt.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    assert!(rt.load_script("03_prologue3").expect("加载 03_prologue3 失败"));
    let mut base_v2: Option<crate::SaveStateV2> = None;
    for _ in 0..800 {
        match rt.next_step() {
            StepResult::Dialogue(_) | StepResult::WaitClick(_) => {}
            StepResult::Choice {
                hide_window_when_selecting,
                ..
            } => {
                if !hide_window_when_selecting {
                    let v2 = rt.build_save_v2();
                    if !v2.active_sprites.is_empty() && v2.active_bg.is_some() {
                        base_v2 = Some(v2);
                        break;
                    }
                }
                let seq = rt.current_choice_seq();
                let _ = rt.submit_choice(seq, 0);
            }
            StepResult::Terminated => break,
            StepResult::Error(e) => panic!("03_prologue3 推进异常: {}", e),
            _ => {}
        }
    }
    let mut cleared_v2 = base_v2.expect("03_prologue3 必须存在带立绘选项节点");
    cleared_v2.active_sprites.clear();
    cleared_v2.active_face = None;
    cleared_v2.active_face_sprites.clear();
    cleared_v2.active_bgm = None;

    let mut rt_cold = PureConnectRuntime::new();
    assert!(rt_cold.init_vfs(base_dir, patch_dir).expect("init_vfs 失败"));
    let (step, visuals) = rt_cold.load_save_v2(&cleared_v2);
    assert!(matches!(step, StepResult::Choice { .. }));
    let vis = visuals.expect("带 active_bg 的选项档必须返回 visuals");
    assert!(
        matches!(vis.bg_update, BgUpdate::New(_)),
        "active_bg 必须正常还原"
    );
    assert!(
        vis.changed_slots.is_empty(),
        "存档瞬间 active_sprites 为空时，必须清除重放前句残留立绘，实际 {:?}",
        vis.changed_slots.keys().collect::<Vec<_>>()
    );
    assert!(
        vis.face_texture.is_none() && !vis.face_changed,
        "存档瞬间 active_face 为空时，必须清除重放前句残留头像"
    );
    assert!(
        vis.bgm_changed && vis.bgm_name.is_none(),
        "存档瞬间 active_bgm 为空时，必须置 bgm_changed=true 且 bgm_name=None 停掉前句/标题 BGM"
    );
}
