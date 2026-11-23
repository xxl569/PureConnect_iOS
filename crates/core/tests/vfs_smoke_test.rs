use pure_connect_core::{BwAudio, CompressedBg, SpriteCompositor, VirtualFileSystem};
use std::path::Path;

#[test]
fn test_vfs_mount_and_lookup() {
    let mut vfs = VirtualFileSystem::new();

    let patch_dir = Path::new(r"U:\anti\temp\exe_unpacked");
    let base_dir = Path::new(r"U:\anti\projects\PURExCONNECT\Archive");

    // 1. 优先挂载汉化解包出来的补丁目录
    if patch_dir.is_dir() {
        vfs.mount_directory(patch_dir)
            .expect("挂载汉化补丁目录失败");
    }

    // 2. 依次挂载原版核心分卷
    let data01000 = base_dir.join("data01000.arc");
    let data02070 = base_dir.join("data02070.arc");
    let data02080 = base_dir.join("data02080.arc");
    let data03000 = base_dir.join("data03000.arc");

    if data01000.is_file() {
        vfs.mount_archive(data01000).expect("挂载 data01000 失败");
    }
    if data02070.is_file() {
        vfs.mount_archive(data02070).expect("挂载 data02070 失败");
    }
    if data02080.is_file() {
        vfs.mount_archive(data02080).expect("挂载 data02080 失败");
    }
    if data03000.is_file() {
        vfs.mount_archive(data03000).expect("挂载 data03000 失败");
    }

    assert!(vfs.source_count() >= 3, "挂载源数量不足");
    assert!(vfs.total_indexed_files() > 1500, "索引文件数目异常");

    // 3. 测试汉化脚本读取（大小写不敏感）
    if patch_dir.is_dir() {
        let script_bytes = vfs.read("05_chapter1_2").expect("无法读取汉化脚本");
        assert!(
            script_bytes.starts_with(b"BurikoCompiledScriptVer1.00\0"),
            "脚本魔数不匹配"
        );

        // 验证 GBK 中文解码
        let (cow, _, _) = encoding_rs::GBK.decode(&script_bytes);
        assert!(cow.contains("萩本泰三"), "未正确检索到汉化对白");
        println!("  [OK] 汉化脚本 05_chapter1_2 读取成功，中文文本对齐无误！");
    }

    // 4. 测试原版脚本零拷贝读取
    if vfs.contains("flow") {
        let flow_bytes = vfs.read("flow").expect("无法读取 flow");
        assert!(
            flow_bytes.starts_with(b"DSC FORMAT 1.00\0"),
            "flow 脚本魔数不匹配"
        );
        println!("  [OK] 原版脚本 flow 零拷贝读取成功！");
    }

    // 5. 测试图像封包读取与解码
    if vfs.contains("mo02pb_fh") {
        let img_bytes = vfs.read("mo02pb_fh").expect("无法读取图像");
        assert!(
            img_bytes.starts_with(b"CompressedBG___\0"),
            "图像魔数不匹配"
        );
        println!("  [OK] 图像 mo02pb_fh CompressedBG 封包切片读取成功！");

        // 核心实测：调用 CompressedBg::decode 还原立绘
        let decoded = CompressedBg::decode(&img_bytes).expect("解码 mo02pb_fh 失败");
        assert_eq!(decoded.width, 680, "立绘宽度不匹配");
        assert_eq!(decoded.height, 920, "立绘高度不匹配");
        assert_eq!(decoded.bpp, 32, "位深不匹配");
        assert_eq!(
            decoded.rgba_data.len(),
            680 * 920 * 4,
            "RGBA8 像素字节总数不匹配"
        );
        println!(
            "  [OK] 立绘 mo02pb_fh 成功解码！尺寸 {}x{}, 像素字节数: {}",
            decoded.width,
            decoded.height,
            decoded.rgba_data.len()
        );

        // 保存为 PNG 实体供抽样检验
        let out_png = Path::new(r"U:\anti\temp\mo02pb_fh.png");
        image::save_buffer(
            out_png,
            &decoded.rgba_data,
            decoded.width,
            decoded.height,
            image::ExtendedColorType::Rgba8,
        )
        .expect("保存 PNG 失败");
        println!("  [OK] 立绘已导出至: {:?}", out_png);
        // 6. 测试背景图解码与内存立绘差分合成
        if vfs.contains("_pc_bg_13w") {
            let bg_bytes = vfs.read("_pc_bg_13w").expect("无法读取背景 _pc_bg_13w");
            let mut bg_decoded =
                CompressedBg::decode(&bg_bytes).expect("解码背景 _pc_bg_13w 失败");
            assert_eq!(bg_decoded.width, 1380);
            assert_eq!(bg_decoded.height, 820);
            println!(
                "  [OK] 背景 _pc_bg_13w 解码成功！尺寸 {}x{}",
                bg_decoded.width, bg_decoded.height
            );

            // 将刚才解码的立绘合成在背景偏右居中位置 (x=400, y=0)
            SpriteCompositor::composite(&mut bg_decoded, &decoded, 400, 0)
                .expect("立绘背景合成失败");

            let scene_png = Path::new(r"U:\anti\temp\composed_scene.png");
            image::save_buffer(
                scene_png,
                &bg_decoded.rgba_data,
                bg_decoded.width,
                bg_decoded.height,
                image::ExtendedColorType::Rgba8,
            )
            .expect("保存合成场景图失败");
            println!("  [OK] 场景合成 (背景+立绘) 成功导出至: {:?}", scene_png);
        }
    }

    // 7. 测试音频封装剥离
    if vfs.contains("14th_chime_01") {
        let raw_audio = vfs.read("14th_chime_01").expect("无法读取音频");
        let stripped = BwAudio::strip_header(&raw_audio).expect("剥离 bw 头失败");
        assert!(
            BwAudio::is_ogg(stripped),
            "剥离后音频头非 OggS: {:?}",
            &stripped[..4]
        );
        println!("  [OK] 音频 14th_chime_01 成功剥离 0x40 头部并还原为纯净 OggS 流！");
    }
}
