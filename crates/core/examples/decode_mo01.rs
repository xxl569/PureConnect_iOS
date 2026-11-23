use pure_connect_core::codec::{CompressedBg, SpriteCompositor};
use pure_connect_core::VirtualFileSystem;
use std::path::Path;

fn main() {
    let mut vfs = VirtualFileSystem::new();
    let base_dir = Path::new(r"U:\anti\projects\PURExCONNECT\Archive");
    vfs.mount_archive(base_dir.join("data02300.arc")).expect("mount 02300");
    
    // Check if mo01pa_fh exists
    if let Ok(fh_data) = vfs.read("mo01pa_fh") {
        let mut fh_img = CompressedBg::decode(&fh_data).expect("decode fh");
        println!("Decoded fh: {}x{}", fh_img.width, fh_img.height);
        
        if let Ok(f_data) = vfs.read("mo01pa_f") {
            let f_img = CompressedBg::decode(&f_data).expect("decode f");
            println!("Decoded f: {}x{}", f_img.width, f_img.height);
            SpriteCompositor::composite(&mut fh_img, &f_img, 0, 0).expect("composite");
        }
        
        image::save_buffer(
            r"U:\anti\temp\mo01pa_clean.png",
            &fh_img.rgba_data,
            fh_img.width,
            fh_img.height,
            image::ExtendedColorType::Rgba8,
        ).expect("save png");
        println!("Saved mo01pa_clean.png!");
    } else {
        println!("mo01pa_fh not found");
    }
}
