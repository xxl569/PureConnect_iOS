use crate::error::{CoreError, Result};

/// BGI 音频头解析与解封装工具
pub struct BwAudio;

impl BwAudio {
    /// 剥离 BGI 专有的 0x40 (64) 字节 `bw  ` 头部，提取纯净的 Ogg/PCM 字节流
    pub fn strip_header(data: &[u8]) -> Result<&[u8]> {
        if data.len() < 8 {
            return Err(CoreError::OutOfBounds);
        }

        // 情况 1: 带有 0x40 字节头的 bw 封装 (例如 40 00 00 00 62 77 20 20)
        if data.len() >= 64 && &data[4..8] == b"bw  " {
            let header_size = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
            if data.len() >= header_size {
                return Ok(&data[header_size..]);
            }
        }

        // 情况 2: 直接以 bw 头部开头 (例如 62 77 20 20)
        if &data[0..4] == b"bw  " && data.len() >= 64 {
            return Ok(&data[64..]);
        }

        // 情况 3: 已经是原生 OggS 格式，直接返回
        if data.starts_with(b"OggS") {
            return Ok(data);
        }

        // 默认放行其他格式
        Ok(data)
    }

    /// 检查是否为原生 Ogg Vorbis 音频流
    pub fn is_ogg(data: &[u8]) -> bool {
        data.starts_with(b"OggS")
    }
}
