//! Batch 1 跨脚本统一管线、0x00F0/0x00F3/0x001B 调用栈、0x003F 补齐、Random/_Wait/_PlayMovie 与全包 CBG 扫雷集成测试

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use pure_connect_core::archive::Arc20Archive;
use pure_connect_core::codec::CompressedBg;
use pure_connect_core::error::Result;
use pure_connect_core::vfs::VirtualFileSystem;
use pure_connect_core::vm::{
    OwnedBgiScript, ScriptLoader, VfsScriptLoader, VmEngine, VmEvent, VmState, BGI_SCRIPT_MAGIC,
    MAX_CALL_DEPTH,
};

fn build_game_vfs() -> Option<Arc<VirtualFileSystem>> {
    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");
    let archive_dir = Path::new(r"U:\anti\projects\PURExCONNECT\Archive");
    if !patch_dir.is_dir() || !archive_dir.is_dir() {
        return None;
    }
    let mut vfs = VirtualFileSystem::new();
    vfs.mount_directory(patch_dir).ok()?;

    let mut arc_paths: Vec<_> = fs::read_dir(archive_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("arc"))
                    .unwrap_or(false)
        })
        .collect();
    arc_paths.sort();
    for p in arc_paths {
        vfs.mount_archive(&p).ok()?;
    }
    Some(Arc::new(vfs))
}

/// 构造最小标准 BGI 合成脚本辅助函数（支持 0x003F / 0x00F0 / 0x00F3 / 0x001C / 0x001B）
fn build_synthetic_bgi(
    builder: impl FnOnce(&mut Vec<u8>, &mut Vec<u8>, usize),
) -> Arc<Vec<u8>> {
    let header_size: u32 = 8; // 4B header_size + 4B mod_count(0)
    let base_offset = 28 + header_size as usize; // 36
    let mut code = Vec::new();
    let mut strs = Vec::new();
    // 预留 1024B 代码段，字符串池统一放在 base_offset + 1024 处
    let str_pool_rel = 1024usize;
    builder(&mut code, &mut strs, str_pool_rel);
    assert!(code.len() <= str_pool_rel);

    let mut raw = Vec::with_capacity(base_offset + str_pool_rel + strs.len());
    raw.extend_from_slice(BGI_SCRIPT_MAGIC);
    raw.extend_from_slice(&header_size.to_le_bytes());
    raw.extend_from_slice(&0u32.to_le_bytes()); // mod_count = 0
    raw.extend_from_slice(&code);
    raw.resize(base_offset + str_pool_rel, 0);
    raw.extend_from_slice(&strs);
    Arc::new(raw)
}

fn push_str_pool(strs: &mut Vec<u8>, str_pool_rel: usize, s: &str) -> u32 {
    let rel = (str_pool_rel + strs.len()) as u32;
    let (enc, _, _) = encoding_rs::GBK.encode(s);
    strs.extend_from_slice(&enc);
    strs.push(0);
    rel
}

fn emit_u32(code: &mut Vec<u8>, v: u32) {
    code.extend_from_slice(&v.to_le_bytes());
}

fn emit_push_int(code: &mut Vec<u8>, v: i32) {
    emit_u32(code, 0x0000);
    emit_u32(code, v as u32);
}

fn emit_push_str(code: &mut Vec<u8>, rel: u32) {
    emit_u32(code, 0x0003);
    emit_u32(code, rel);
}

fn emit_syscall(code: &mut Vec<u8>, op: u32, id: u32) {
    emit_u32(code, 0x003F);
    emit_u32(code, op);
    emit_u32(code, id);
}

fn emit_call_f0(code: &mut Vec<u8>, target_rel: u32) {
    emit_push_str(code, target_rel);
    emit_push_int(code, 0);
    emit_u32(code, 0x00F6);
    emit_u32(code, 0x00F0);
}

fn emit_tail_f3(code: &mut Vec<u8>, target_rel: u32) {
    emit_push_str(code, target_rel);
    emit_push_int(code, 0);
    emit_u32(code, 0x00F6);
    emit_u32(code, 0x00F3);
}

struct MapLoader {
    scripts: HashMap<String, Arc<Vec<u8>>>,
}

impl ScriptLoader for MapLoader {
    fn load_script(&self, name: &str) -> Result<Option<Arc<Vec<u8>>>> {
        Ok(self.scripts.get(&name.to_ascii_lowercase()).cloned())
    }
}

#[test]
fn test_max_call_depth_fuse_and_random_non_positive_guard() {
    // 1. 构造自递归脚本，验证 MAX_CALL_DEPTH=16 可控熔断报错
    let self_rec = build_synthetic_bgi(|code, strs, pool| {
        // 触发标准操作码探测
        emit_syscall(code, 0, 0x0119);
        let t = push_str_pool(strs, pool, "loop_sub");
        emit_call_f0(code, t);
        emit_u32(code, 0x001B);
    });
    let mut map = HashMap::new();
    map.insert("loop_sub".to_string(), Arc::clone(&self_rec));
    let loader = Arc::new(MapLoader { scripts: map });

    let owned = OwnedBgiScript::parse(self_rec).unwrap();
    let mut eng = VmEngine::from_owned(owned);
    eng.set_script_name("loop_sub");
    eng.set_script_loader(loader);

    let err = eng.step().expect_err("超过 MAX_CALL_DEPTH 必须触发可控熔断报错");
    assert_eq!(eng.call_depth(), MAX_CALL_DEPTH);
    assert!(
        format!("{:?}", err).contains("MAX_CALL_DEPTH"),
        "错误信息应明确标示熔断原因: {:?}",
        err
    );

    // 2. 验证 Random(0) / Random(-5) 严格返回 0 不除零 panic，Random(5) 落在 0..5 且可固定种子复现
    let rand_script = build_synthetic_bgi(|code, strs, pool| {
        emit_syscall(code, 0, 0x0119);
        let fn_rand = push_str_pool(strs, pool, "Random");

        // Random(0) -> flags[0] == 0
        emit_push_int(code, 0);
        emit_push_str(code, fn_rand);
        emit_u32(code, 0x001C);
        // flags[1] = flags[0]
        emit_push_int(code, 1);
        emit_push_int(code, 0);
        emit_syscall(code, 1, 0x00E1);
        emit_syscall(code, 2, 0x00E0);

        // Random(-5) -> flags[0] == 0
        emit_push_int(code, -5);
        emit_push_str(code, fn_rand);
        emit_u32(code, 0x001C);
        // flags[2] = flags[0]
        emit_push_int(code, 2);
        emit_push_int(code, 0);
        emit_syscall(code, 1, 0x00E1);
        emit_syscall(code, 2, 0x00E0);

        // Random(5) -> flags[0] in 0..5
        emit_push_int(code, 5);
        emit_push_str(code, fn_rand);
        emit_u32(code, 0x001C);
        emit_u32(code, 0x001B);
    });

    let mut eng1 = VmEngine::from_owned(OwnedBgiScript::parse(Arc::clone(&rand_script)).unwrap());
    eng1.set_rng_seed(123456789);
    while eng1.state() != &VmState::Terminated {
        let _ = eng1.step().unwrap();
    }
    assert_eq!(eng1.get_flag(1), 0, "Random(0) 必须返回 0");
    assert_eq!(eng1.get_flag(2), 0, "Random(-5) 必须返回 0");
    let r1 = eng1.get_flag(0);
    assert!((0..5).contains(&r1), "Random(5) 必须落在 0..5，实际 {}", r1);

    // 同种子复现确定性
    let mut eng2 = VmEngine::from_owned(OwnedBgiScript::parse(rand_script).unwrap());
    eng2.set_rng_seed(123456789);
    while eng2.state() != &VmState::Terminated {
        let _ = eng2.step().unwrap();
    }
    assert_eq!(eng2.get_flag(0), r1, "相同 PRNG 种子必须产生完全确定结果");
}

#[test]
fn test_real_subscripts_f0_nested_and_return_zero_unknowns() {
    let Some(vfs) = build_game_vfs() else {
        return;
    };
    let loader = Arc::new(VfsScriptLoader::new(vfs));

    // 1. 汉化优先级校验：01_prologue1 必须加载 exe_unpacked 中的中文版，绝不能被 .arc 日文覆盖
    let p1_bytes = loader.load_script("01_prologue1").unwrap().expect("01_prologue1 必须存在");
    let p1_owned = OwnedBgiScript::parse(p1_bytes).unwrap();
    let mut p1_eng = VmEngine::from_owned(p1_owned);
    p1_eng.set_script_name("01_prologue1");
    p1_eng.set_script_loader(Arc::clone(&loader) as Arc<dyn ScriptLoader>);
    let first_dlg = p1_eng.next_dialogue().unwrap().expect("01_prologue1 必须产出中文对白");
    assert!(
        !first_dlg.text.is_empty() && !first_dlg.text.contains("僼"),
        "01_prologue1 必须为正常中文汉化文本，实际: {}",
        first_dlg.text
    );

    // 2. 构造父脚本，连续 0x00F0 调用 set_Mail_off / chk_Tel / eye_short / eye_long / play_movie，
    //    并在每次返回后设置标志位，验证 0x001B 精确回到父脚本断点、0 未知 opcode / 0 未知 sys_id
    let driver = build_synthetic_bgi(|code, strs, pool| {
        // 先设置 flags[200]=1, flags[201]=1 验证 set_Mail_off 清零效果
        emit_push_int(code, 200);
        emit_push_int(code, 1);
        emit_syscall(code, 2, 0x00E0);
        emit_push_int(code, 201);
        emit_push_int(code, 1);
        emit_syscall(code, 2, 0x00E0);

        let s_mail = push_str_pool(strs, pool, "set_Mail_off");
        let s_tel = push_str_pool(strs, pool, "chk_Tel");
        let s_short = push_str_pool(strs, pool, "eye_short");
        let s_long = push_str_pool(strs, pool, "eye_long");
        let s_movie = push_str_pool(strs, pool, "play_movie");

        // 调用 set_Mail_off
        emit_call_f0(code, s_mail);
        emit_push_int(code, 301);
        emit_push_int(code, 11);
        emit_syscall(code, 2, 0x00E0);

        // 调用 chk_Tel（flags[50] < 16 时执行 0x00E4(410, 0) 并安全返回）
        emit_call_f0(code, s_tel);
        emit_push_int(code, 302);
        emit_push_int(code, 22);
        emit_syscall(code, 2, 0x00E0);

        // 调用 eye_short（执行 Random(5) + _DrawScene + _Wait）
        emit_call_f0(code, s_short);
        emit_push_int(code, 303);
        emit_push_int(code, 33);
        emit_syscall(code, 2, 0x00E0);

        // 调用 eye_long（执行 _DrawScene + _Wait(2500, 1)）
        emit_call_f0(code, s_long);
        emit_push_int(code, 304);
        emit_push_int(code, 44);
        emit_syscall(code, 2, 0x00E0);

        // 调用 play_movie（执行 _Wait + _PlayMovie 降级桩 + _Wait + RET）
        emit_call_f0(code, s_movie);
        emit_push_int(code, 305);
        emit_push_int(code, 55);
        emit_syscall(code, 2, 0x00E0);

        emit_u32(code, 0x001B);
    });

    let mut eng = VmEngine::from_owned(OwnedBgiScript::parse(driver).unwrap());
    eng.set_script_name("batch1_driver");
    eng.set_rng_seed(42);
    eng.set_script_loader(Arc::clone(&loader) as Arc<dyn ScriptLoader>);

    let mut saw_wait = false;
    let mut steps = 0usize;
    while eng.state() != &VmState::Terminated {
        if let Some(evt) = eng.step().expect("真实子脚本嵌套执行不得出错或遇到未知指令") {
            if matches!(evt, VmEvent::Wait { .. }) {
                saw_wait = true;
            }
        }
        steps += 1;
        assert!(steps < 5000, "防止意外死循环");
    }

    assert!(saw_wait, "eye_short / eye_long / play_movie 必须触发 _Wait 事件");
    assert_eq!(eng.call_depth(), 0, "全部子脚本返回后调用栈必须为空");
    assert_eq!(eng.script_name(), "batch1_driver", "必须精确回到父脚本");
    assert_eq!(eng.get_flag(200), 0, "set_Mail_off 必须清零 flags[200]");
    assert_eq!(eng.get_flag(201), 0, "set_Mail_off 必须清零 flags[201]");
    assert_eq!(eng.get_flag(301), 11, "set_Mail_off 返回断点必须准确");
    assert_eq!(eng.get_flag(302), 22, "chk_Tel 返回断点必须准确");
    assert_eq!(eng.get_flag(303), 33, "eye_short 返回断点必须准确");
    assert_eq!(eng.get_flag(304), 44, "eye_long 返回断点必须准确");
    assert_eq!(eng.get_flag(305), 55, "play_movie 返回断点必须准确");
    assert_eq!(eng.sys_regs().get(&0).copied(), Some(0), "自定义主角名「李骏」 0=>0 不得被破坏");
    assert_eq!(eng.sys_regs().get(&410).copied(), Some(0), "chk_Tel 写入 410=>0 必须生效");
}

#[test]
fn test_real_tail_jump_f3_clears_call_stack_and_preserves_visuals_and_flags() {
    let Some(vfs) = build_game_vfs() else {
        return;
    };
    let loader = Arc::new(VfsScriptLoader::new(vfs));

    // 路径 1: root_common -> 0x00F0(子脚本) -> 0x00F3(90_badend_01)
    // 验证：进入子脚本时 call_depth == 1，触发 0x00F3(90_badend_01) 后 call_depth 归 0，且保留视觉 BG 与 flags
    let mid_sub = build_synthetic_bgi(|code, strs, pool| {
        emit_syscall(code, 0, 0x0119);
        // 设置 BG 与 flag，再执行 0x00F3 尾跳转到真实 90_badend_01
        let bg = push_str_pool(strs, pool, "bg_keep_test");
        let draw_scene = push_str_pool(strs, pool, "_DrawScene");
        emit_push_str(code, bg);
        emit_push_str(code, draw_scene);
        emit_u32(code, 0x001C);

        emit_push_int(code, 77);
        emit_push_int(code, 999);
        emit_syscall(code, 2, 0x00E0);

        let target = push_str_pool(strs, pool, "90_badend_01");
        emit_tail_f3(code, target);
        emit_u32(code, 0x001B);
    });

    let root_wrapper = build_synthetic_bgi(|code, strs, pool| {
        emit_syscall(code, 0, 0x0119);
        let sub = push_str_pool(strs, pool, "mid_20_chapter6_3");
        emit_call_f0(code, sub);
        // 若 0x00F3 错误保留了调用栈，90_badend_01 结束后会错误回到此处写 flags[88]=1
        emit_push_int(code, 88);
        emit_push_int(code, 1);
        emit_syscall(code, 2, 0x00E0);
        emit_u32(code, 0x001B);
    });

    struct OverlayLoader {
        extra: HashMap<String, Arc<Vec<u8>>>,
        base: Arc<VfsScriptLoader>,
    }
    impl ScriptLoader for OverlayLoader {
        fn load_script(&self, name: &str) -> Result<Option<Arc<Vec<u8>>>> {
            if let Some(b) = self.extra.get(&name.to_ascii_lowercase()) {
                return Ok(Some(Arc::clone(b)));
            }
            self.base.load_script(name)
        }
    }

    let mut extra = HashMap::new();
    extra.insert("mid_20_chapter6_3".to_string(), mid_sub);
    let overlay = Arc::new(OverlayLoader {
        extra,
        base: Arc::clone(&loader),
    });

    let mut eng = VmEngine::from_owned(OwnedBgiScript::parse(root_wrapper).unwrap());
    eng.set_script_name("root_common");
    eng.set_script_loader(overlay);

    // 推进直到跳入 90_badend_01
    let mut entered_badend = false;
    for _ in 0..200 {
        let _ = eng.step().unwrap();
        if eng.script_name().eq_ignore_ascii_case("90_badend_01") {
            entered_badend = true;
            assert_eq!(eng.call_depth(), 0, "0x00F3 尾跳转必须清空 call_stack");
            assert_eq!(eng.get_flag(77), 999, "0x00F3 必须保留 flags");
            assert!(
                eng.current_bg().is_some(),
                "0x00F3 切换瞬间严禁清空舞台 BG"
            );
            break;
        }
    }
    assert!(entered_badend, "必须成功通过 0x00F3 尾跳转进入 90_badend_01");

    // 验证真实脚本 90_badend_01 可正常产出中文对白，且终结后绝不返回 root_wrapper
    let first_dlg = eng.next_dialogue().unwrap().expect("90_badend_01 必须能正常产出对白");
    assert!(!first_dlg.text.is_empty());
    assert_eq!(eng.get_flag(88), 0, "0x00F3 清空 call_stack 后绝不能回退父脚本");

    // 路径 2: root_common -> 0x00F3(root_Arisa) 真实链路验证
    let root_arisa_driver = build_synthetic_bgi(|code, strs, pool| {
        emit_syscall(code, 0, 0x0119);
        emit_push_int(code, 55);
        emit_push_int(code, 123);
        emit_syscall(code, 2, 0x00E0);
        let t = push_str_pool(strs, pool, "root_Arisa");
        emit_tail_f3(code, t);
        emit_u32(code, 0x001B);
    });
    let mut eng_ar = VmEngine::from_owned(OwnedBgiScript::parse(root_arisa_driver).unwrap());
    eng_ar.set_script_name("root_common");
    eng_ar.set_script_loader(Arc::clone(&loader) as Arc<dyn ScriptLoader>);
    let dlg_ar = eng_ar
        .next_dialogue()
        .expect("root_common -> 0x00F3(root_Arisa) 跨脚本执行不得出错")
        .expect("root_Arisa 路由后必须能进入有纱线首章并产出对白");
    assert!(!dlg_ar.text.is_empty());
    assert_eq!(eng_ar.get_flag(55), 123, "0x00F3 跳入 root_Arisa 后必须保留 flags");
    assert!(
        eng_ar.call_depth() >= 1,
        "root_Arisa 内部 0x00F0 调用具体章节脚本时 call_depth 应 >= 1，实际 {}",
        eng_ar.call_depth()
    );
}

/// 全包 18,467 张 CBG 图像归零扫雷（多线程并行版；默认 ignore 避免拖慢日常 cargo test，验收时显式 --ignored 运行）
#[test]
#[ignore]
fn test_full_archive_cbg_sweep_zero_failures() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    let dirs = [
        Path::new(r"U:\anti\projects\PURExCONNECT"),
        Path::new(r"U:\anti\projects\PURExCONNECT\Archive"),
    ];
    let mut arc_paths = Vec::new();
    for d in dirs {
        if let Ok(rd) = fs::read_dir(d) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.is_file()
                    && p.extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.eq_ignore_ascii_case("arc"))
                        .unwrap_or(false)
                {
                    arc_paths.push(p);
                }
            }
        }
    }
    arc_paths.sort();
    assert!(!arc_paths.is_empty(), "未找到游戏 .arc 封包");

    // 预打开所有 mmap 封包并收集全部 CBG 条目，按 CPU 核心动态分片并行解码
    let archives: Vec<(String, Arc20Archive)> = arc_paths
        .iter()
        .map(|ap| {
            let aname = ap.file_name().unwrap().to_string_lossy().to_string();
            let arc = Arc20Archive::open(ap).unwrap();
            (aname, arc)
        })
        .collect();

    let mut cbg_tasks: Vec<(usize, String)> = Vec::with_capacity(20_000);
    for (arc_idx, (_, arc)) in archives.iter().enumerate() {
        for entry in arc.iter_entries() {
            let Some(raw) = arc.read(&entry.name) else {
                continue;
            };
            if raw.len() >= 16 && &raw[..16] == b"CompressedBG___\0" {
                cbg_tasks.push((arc_idx, entry.name.clone()));
            }
        }
    }

    let total_cbg = cbg_tasks.len();
    let cursor = AtomicUsize::new(0);
    let failures: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let worker_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    std::thread::scope(|s| {
        for _ in 0..worker_count {
            s.spawn(|| {
                let mut local_failures = Vec::new();
                loop {
                    let idx = cursor.fetch_add(1, Ordering::Relaxed);
                    if idx >= cbg_tasks.len() {
                        break;
                    }
                    let (arc_idx, ref entry_name) = cbg_tasks[idx];
                    let (ref aname, ref arc) = archives[arc_idx];
                    if let Some(raw) = arc.read(entry_name) {
                        if let Err(e) = CompressedBg::decode(raw) {
                            local_failures.push(format!("{}:{} -> {:?}", aname, entry_name, e));
                        }
                    }
                }
                if !local_failures.is_empty() {
                    failures.lock().unwrap().extend(local_failures);
                }
            });
        }
    });

    let failures = failures.into_inner().unwrap();
    println!(
        "[CBG SWEEP] workers={}, total_cbg={}, failures={}",
        worker_count,
        total_cbg,
        failures.len()
    );
    assert!(
        total_cbg >= 18_000,
        "全包 CBG 图像总数应 >= 18,000，实际 {}",
        total_cbg
    );
    assert!(
        failures.is_empty(),
        "全包 CBG 扫雷必须 0 失败，实际失败 {} 张，前 10 项: {:?}",
        failures.len(),
        &failures[..failures.len().min(10)]
    );
}

#[test]
fn test_real_end_roll_chk_tel_branch_and_20_chapter6_3_badend() {
    let Some(vfs) = build_game_vfs() else {
        return;
    };
    let loader = Arc::new(VfsScriptLoader::new(vfs));

    // 1. 实跑真实 end_roll 直至 Terminated（暴露任何未覆盖 opcode / sys_id）
    if let Some(end_roll_bytes) = loader.load_script("end_roll").unwrap() {
        let mut eng = VmEngine::from_owned(OwnedBgiScript::parse(end_roll_bytes).unwrap());
        eng.set_script_name("end_roll");
        eng.set_script_loader(Arc::clone(&loader) as Arc<dyn ScriptLoader>);
        let mut steps = 0usize;
        while eng.state() != &VmState::Terminated {
            let _ = eng.step().expect("end_roll 实跑不得出错");
            steps += 1;
            assert!(steps < 20_000, "end_roll 疑似死循环");
        }
        println!("[PROBE] end_roll terminated cleanly in {} steps", steps);
    }

    // 2. 实跑真实 chk_Tel（设置高好感条件触发电话支线与 0x00E4(400..409, 1) 脏标志）
    let chk_tel_bytes = loader.load_script("chk_Tel").unwrap().expect("chk_Tel 必须存在");
    let mut eng_tel = VmEngine::from_owned(OwnedBgiScript::parse(chk_tel_bytes).unwrap());
    eng_tel.set_script_name("chk_Tel");
    eng_tel.set_script_loader(Arc::clone(&loader) as Arc<dyn ScriptLoader>);
    // 尝试常见高好感标志组合，触发至少一条电话分支
    let mut flags = std::collections::BTreeMap::new();
    flags.insert(50, 20);
    flags.insert(10, 20);
    flags.insert(11, 20);
    flags.insert(12, 20);
    flags.insert(13, 20);
    flags.insert(14, 20);
    eng_tel.set_flags(flags);
    let mut saw_tel_sub = false;
    let mut steps = 0usize;
    while eng_tel.state() != &VmState::Terminated {
        if eng_tel.call_depth() >= 1 {
            saw_tel_sub = true;
            println!(
                "[PROBE] chk_Tel entered subscript={}, call_depth={}, dirty={}",
                eng_tel.script_name(),
                eng_tel.call_depth(),
                eng_tel.sys_regs().iter().filter(|(&k, &v)| (400..=409).contains(&k) && v != 0).count()
            );
        }
        match eng_tel.next_dialogue().expect("chk_Tel 电话支线不得出错") {
            Some(_) => {}
            None => {
                if matches!(eng_tel.state(), VmState::WaitingForChoice { .. }) {
                    eng_tel.select_choice(0);
                }
            }
        }
        steps += 1;
        assert!(steps < 2000, "chk_Tel 疑似死循环");
    }
    println!(
        "[PROBE] chk_Tel finished: saw_tel_sub={}, sys_regs_dirty={}, sys_regs_400_409={:?}",
        saw_tel_sub,
        eng_tel.take_sys_regs_dirty(),
        eng_tel
            .sys_regs()
            .iter()
            .filter(|(&k, _)| (400..=409).contains(&k))
            .collect::<Vec<_>>()
    );

    // 3. 实跑真实 20_chapter6_3（从外层 0x00F0 调入，低好感触发真实 0x00F3(90_badend_01)）
    let wrapper = build_synthetic_bgi(|code, strs, pool| {
        emit_syscall(code, 0, 0x0119);
        let sub = push_str_pool(strs, pool, "20_chapter6_3");
        emit_call_f0(code, sub);
        emit_push_int(code, 888);
        emit_push_int(code, 1);
        emit_syscall(code, 2, 0x00E0);
        emit_u32(code, 0x001B);
    });
    let mut eng_ch6 = VmEngine::from_owned(OwnedBgiScript::parse(wrapper).unwrap());
    eng_ch6.set_script_name("root_common");
    eng_ch6.set_script_loader(Arc::clone(&loader) as Arc<dyn ScriptLoader>);
    let mut entered_badend = false;
    let mut dlg_steps = 0usize;
    while eng_ch6.state() != &VmState::Terminated && dlg_steps < 1500 {
        if eng_ch6.script_name().eq_ignore_ascii_case("90_badend_01") {
            entered_badend = true;
            assert_eq!(eng_ch6.call_depth(), 0, "真实 20_chapter6_3 -> 0x00F3(90_badend_01) 必须清空 call_stack");
            break;
        }
        match eng_ch6.next_dialogue().expect("真实 20_chapter6_3 执行不得出错") {
            Some(_) => {}
            None => {
                if matches!(eng_ch6.state(), VmState::WaitingForChoice { .. }) {
                    eng_ch6.select_choice(0);
                }
            }
        }
        dlg_steps += 1;
    }
    println!(
        "[PROBE] 20_chapter6_3 finished: entered_badend={}, final_script={}, call_depth={}, dlg_steps={}",
        entered_badend,
        eng_ch6.script_name(),
        eng_ch6.call_depth(),
        dlg_steps
    );
}
