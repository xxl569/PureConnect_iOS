use crate::codec::compressed_bg::DecodedImage;
use crate::error::{CoreError, Result};

/// 立绘与表情差分内存级合成器 (Dirty Rect Blit & Alpha Blending)
pub struct SpriteCompositor;

impl SpriteCompositor {
    /// 将差分图 `diff` 混合到基准图 `base` 的指定偏移 `(dst_x, dst_y)` 处
    ///
    /// 采用严格的定点数整型 Alpha Blending，完全杜绝浮点抖动与跨平台舍入差异。
    pub fn composite(
        base: &mut DecodedImage,
        diff: &DecodedImage,
        dst_x: u32,
        dst_y: u32,
    ) -> Result<()> {
        Self::composite_cropped(base, diff, dst_x, dst_y, diff.height)
    }

    /// 将差分图 `diff` 顶部最多 `src_h` 行混合到基准图 `base` 的指定偏移 `(dst_x, dst_y)` 处
    ///
    /// 用于从 BGI 3 帧纵向动画图带中裁取第 0 帧（睁眼/初始状态）合成至全身立绘底图。
    pub fn composite_cropped(
        base: &mut DecodedImage,
        diff: &DecodedImage,
        dst_x: u32,
        dst_y: u32,
        src_h: u32,
    ) -> Result<()> {
        let base_w = base.width as usize;
        let base_h = base.height as usize;
        let diff_w = diff.width as usize;
        let diff_h = (diff.height.min(src_h)) as usize;
        let ox = dst_x as usize;
        let oy = dst_y as usize;

        if ox >= base_w || oy >= base_h {
            return Err(CoreError::OutOfBounds);
        }

        // 计算实际可 Blit 的有效包围盒（边界安全裁剪）
        let copy_w = diff_w.min(base_w - ox);
        let copy_h = diff_h.min(base_h - oy);

        let base_rgba = &mut base.rgba_data;
        let diff_rgba = &diff.rgba_data;

        for dy in 0..copy_h {
            let by = oy + dy;
            let base_row_start = (by * base_w + ox) * 4;
            let diff_row_start = (dy * diff_w) * 4;

            for dx in 0..copy_w {
                let bi = base_row_start + dx * 4;
                let di = diff_row_start + dx * 4;

                let da = diff_rgba[di + 3] as u32;
                if da == 255 {
                    // 全不透明：直接覆盖
                    base_rgba[bi..bi + 4].copy_from_slice(&diff_rgba[di..di + 4]);
                } else if da > 0 {
                    // 半透明：无除法精确 Alpha 合成 (((x + 1 + (x >> 8)) >> 8) == x / 255 for 0..=65025)
                    let inv_a = 255 - da;
                    let dr = diff_rgba[di] as u32;
                    let dg = diff_rgba[di + 1] as u32;
                    let db = diff_rgba[di + 2] as u32;

                    let br = base_rgba[bi] as u32;
                    let bg = base_rgba[bi + 1] as u32;
                    let bb = base_rgba[bi + 2] as u32;
                    let ba = base_rgba[bi + 3] as u32;

                    let r_sum = dr * da + br * inv_a;
                    let g_sum = dg * da + bg * inv_a;
                    let b_sum = db * da + bb * inv_a;
                    let a_sum = ba * inv_a;

                    base_rgba[bi] = ((r_sum + 1 + (r_sum >> 8)) >> 8) as u8;
                    base_rgba[bi + 1] = ((g_sum + 1 + (g_sum >> 8)) >> 8) as u8;
                    base_rgba[bi + 2] = ((b_sum + 1 + (b_sum >> 8)) >> 8) as u8;
                    base_rgba[bi + 3] = (da + ((a_sum + 1 + (a_sum >> 8)) >> 8)).min(255) as u8;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alpha_blending() {
        let mut base = DecodedImage {
            width: 2,
            height: 2,
            bpp: 32,
            rgba_data: vec![
                255, 0, 0, 255, // 红 (0,0)
                0, 255, 0, 255, // 绿 (1,0)
                0, 0, 255, 255, // 蓝 (0,1)
                255, 255, 255, 255, // 白 (1,1)
            ],
        };

        let diff = DecodedImage {
            width: 1,
            height: 1,
            bpp: 32,
            rgba_data: vec![0, 0, 0, 255], // 纯黑
        };

        // 将纯黑 Blit 到 (1, 1)
        SpriteCompositor::composite(&mut base, &diff, 1, 1).unwrap();
        assert_eq!(&base.rgba_data[12..16], &[0, 0, 0, 255]);
    }
}
