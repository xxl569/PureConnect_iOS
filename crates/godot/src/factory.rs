//! 主线程纹理与音频对象工厂
//!
//! 铁律约束：严禁在非 Godot 主线程上下文调用此处任何方法！
//! 后台解码输出的 RGBA8 与 OGG 原始数据必须在此处同步转换为 Godot 原生引擎对象。

use godot::classes::{image::Format, AudioStreamOggVorbis, Image, ImageTexture};
use godot::prelude::*;
use pure_connect_core::codec::DecodedImage;

pub struct MainThreadFactory;

impl MainThreadFactory {
    /// 从 RGBA8 像素缓冲在主线程构造 Godot ImageTexture
    pub fn create_texture_from_image(decoded: &DecodedImage) -> Option<Gd<ImageTexture>> {
        if decoded.width == 0 || decoded.height == 0 || decoded.rgba_data.is_empty() {
            return None;
        }

        let packed_data = PackedByteArray::from(decoded.rgba_data.as_slice());
        let img = Image::create_from_data(
            decoded.width as i32,
            decoded.height as i32,
            false,
            Format::RGBA8,
            &packed_data,
        )?;

        ImageTexture::create_from_image(&img)
    }

    /// 从 RGBA8 像素缓冲构造背景 ImageTexture
    pub fn create_bg_texture_from_image(decoded: &DecodedImage) -> Option<Gd<ImageTexture>> {
        Self::create_texture_from_image(decoded)
    }

    /// 创建纯色纹理 (例如 "white", "black")
    pub fn create_solid_color_texture(
        width: u32,
        height: u32,
        r: u8,
        g: u8,
        b: u8,
        a: u8,
    ) -> Option<Gd<ImageTexture>> {
        let size = (width * height * 4) as usize;
        let mut pixels = Vec::with_capacity(size);
        for _ in 0..(width * height) {
            pixels.push(r);
            pixels.push(g);
            pixels.push(b);
            pixels.push(a);
        }

        let packed = PackedByteArray::from(pixels.as_slice());
        let img = Image::create_from_data(
            width as i32,
            height as i32,
            false,
            Format::RGBA8,
            &packed,
        )?;

        ImageTexture::create_from_image(&img)
    }

    /// 从已剥离 bw 头的原生 Ogg Vorbis 字节流创建 Godot AudioStreamOggVorbis
    pub fn create_ogg_stream(ogg_bytes: &[u8]) -> Option<Gd<AudioStreamOggVorbis>> {
        if ogg_bytes.is_empty() {
            return None;
        }

        let packed = PackedByteArray::from(ogg_bytes);
        AudioStreamOggVorbis::load_from_buffer(&packed)
    }
}
