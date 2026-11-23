use crate::error::{CoreError, Result};

/// BGI CompressedBG (v1) 图像解码器
///
/// 还原管线：
/// 1. 解析 48 字节文件头，提取几何参数与加密参数
/// 2. UpdateKey 状态机解密 0x30 开始的 EncLength 字节，并进行 CheckSum/CheckXor 验算
/// 3. 从解密块中通过 LEB128 读取 256 个叶子节点权重，线性构建 511 节点规范 Huffman 树
/// 4. 从比特流中 MSB-first 解码 Huffman Token 填充 IntermediateLength 字节
/// 5. Zero-RLE 状态机解压为原始像素字节 (Stride * Height)
/// 6. 2D 差分平均逆滤波 (ReverseAverageSampling) 还原平滑像素色彩
/// 7. 色彩通道标准化转换（BGRA/BGR -> RGBA8）
pub struct CompressedBg;

#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
    pub rgba_data: Vec<u8>,
}

#[derive(Clone, Copy)]
struct HuffmanNode {
    weight: u32,
    left: u16,
    right: u16,
    valid: bool,
}

impl CompressedBg {
    /// 解码 CompressedBG 封包切片 (支持 Version 1 与 Version 2)
    pub fn decode(data: &[u8]) -> Result<DecodedImage> {
        if data.len() < 0x30 {
            return Err(CoreError::InvalidFormat("CompressedBG 数据长度不足 48 字节"));
        }

        // 1. 验证魔数
        if &data[0x00..0x10] != b"CompressedBG___\0" {
            return Err(CoreError::InvalidFormat("魔数不是 CompressedBG___"));
        }

        // 解析头部元数据
        let width = u16::from_le_bytes(data[0x10..0x12].try_into().unwrap()) as u32;
        let height = u16::from_le_bytes(data[0x12..0x14].try_into().unwrap()) as u32;
        let bpp = u32::from_le_bytes(data[0x14..0x18].try_into().unwrap());
        let intermediate_len =
            u32::from_le_bytes(data[0x20..0x24].try_into().unwrap()) as usize;
        let key = u32::from_le_bytes(data[0x24..0x28].try_into().unwrap());
        let enc_len = u32::from_le_bytes(data[0x28..0x2C].try_into().unwrap()) as usize;
        let check_sum = data[0x2C];
        let check_xor = data[0x2D];
        let version = u16::from_le_bytes(data[0x2E..0x30].try_into().unwrap());

        if version != 1 && version != 2 {
            return Err(CoreError::InvalidFormat("仅支持 CompressedBG Version 1 或 2"));
        }
        if version == 1 {
            if bpp != 8 && bpp != 24 && bpp != 32 {
                return Err(CoreError::InvalidFormat("CBG v1 仅支持 8/24/32 位图像"));
            }
        } else if bpp != 24 && bpp != 32 {
            return Err(CoreError::InvalidFormat("CBG v2 仅支持 24 位或 32 位图像"));
        }

        if data.len() < 0x30 + enc_len {
            return Err(CoreError::InvalidFormat("加密数据块超出文件大小"));
        }

        // 2. 解密头部附加数据块并做双重校验和验算
        let mut rolling_key = key;
        let mut dec_data = data[0x30..0x30 + enc_len].to_vec();
        let mut sum = 0u8;
        let mut xor = 0u8;

        for byte in &mut dec_data {
            let k = Self::update_key(&mut rolling_key);
            *byte = byte.wrapping_sub(k);
            sum = sum.wrapping_add(*byte);
            xor ^= *byte;
        }

        if sum != check_sum || xor != check_xor {
            return Err(CoreError::InvalidFormat(
                "CompressedBG 校验和验算失败，密钥或流损坏",
            ));
        }

        if version == 2 {
            return Self::decode_v2(width, height, bpp, &dec_data, &data[0x30 + enc_len..]);
        }

        let pixel_size = (bpp / 8) as usize;
        let stride = (width as usize) * pixel_size;
        let total_pixel_bytes = stride * (height as usize);

        // 3. 从解密流中读取 256 个叶子节点权重 (LEB128)
        let mut leaf_weights = [0u32; 256];
        let mut dec_pos = 0;
        for i in 0..256 {
            let mut v = 0u32;
            let mut code_len = 0;
            loop {
                if dec_pos >= dec_data.len() || code_len >= 32 {
                    return Err(CoreError::InvalidFormat("权重表 LEB128 解码越界"));
                }
                let code = dec_data[dec_pos];
                dec_pos += 1;
                v |= ((code & 0x7F) as u32) << code_len;
                code_len += 7;
                if (code & 0x80) == 0 {
                    break;
                }
            }
            leaf_weights[i] = v;
        }

        // 4. 构建规范 Huffman 树 (仅包含权重非零的有效节点)
        let mut node_list = Vec::with_capacity(512);
        let mut root_weight = 0u32;
        for &w in leaf_weights.iter() {
            let valid = w != 0;
            node_list.push(HuffmanNode {
                weight: w,
                left: 0,
                right: 0,
                valid,
            });
            root_weight = root_weight.wrapping_add(w);
        }

        loop {
            let mut child = [usize::MAX, usize::MAX];
            let mut weight = 0u32;

            for i in 0..2 {
                let mut min_weight = u32::MAX;
                for n in 0..node_list.len() {
                    if node_list[n].valid && node_list[n].weight < min_weight {
                        min_weight = node_list[n].weight;
                        child[i] = n;
                    }
                }
                if child[i] == usize::MAX {
                    break;
                }
                node_list[child[i]].valid = false;
                weight = weight.wrapping_add(min_weight);
            }

            if child[1] == usize::MAX {
                break;
            }

            let parent_node = HuffmanNode {
                weight,
                left: child[0] as u16,
                right: child[1] as u16,
                valid: true,
            };
            node_list.push(parent_node);
            if weight == root_weight {
                break;
            }
        }

        let root_node = node_list.len() - 1;

        // 5. 解码 Huffman 比特流 (MSB-first) 至 Intermediate 缓冲区
        let stream = &data[0x30 + enc_len..];
        let mut packed = vec![0u8; intermediate_len];
        let mut stream_pos = 0;
        let mut bit_pos = 0;

        for dst in 0..intermediate_len {
            let mut curr_node = root_node;
            while curr_node >= 256 {
                if stream_pos >= stream.len() {
                    return Err(CoreError::InvalidFormat("Huffman 比特流意外终止"));
                }
                let bit = (stream[stream_pos] >> (7 - bit_pos)) & 1;
                bit_pos += 1;
                if bit_pos == 8 {
                    bit_pos = 0;
                    stream_pos += 1;
                }
                if bit == 0 {
                    curr_node = node_list[curr_node].left as usize;
                } else {
                    curr_node = node_list[curr_node].right as usize;
                }
            }
            packed[dst] = curr_node as u8;
        }

        // 6. UnpackZeros: Zero-RLE 解压还原为原始像素空间
        let mut output = vec![0u8; total_pixel_bytes];
        let mut dst = 0usize;
        let mut dec_zero = false;
        let mut src = 0usize;

        while dst < total_pixel_bytes {
            let mut code_len = 0;
            let mut count = 0usize;
            loop {
                if src >= packed.len() {
                    break;
                }
                let code = packed[src];
                src += 1;
                count |= ((code & 0x7F) as usize) << code_len;
                code_len += 7;
                if (code & 0x80) == 0 {
                    break;
                }
            }
            if dst + count > total_pixel_bytes {
                break;
            }
            if !dec_zero {
                if src + count > packed.len() {
                    break;
                }
                output[dst..dst + count].copy_from_slice(&packed[src..src + count]);
                src += count;
            } else {
                output[dst..dst + count].fill(0);
            }
            dec_zero = !dec_zero;
            dst += count;
        }

        // 7. ReverseAverageSampling: 2D 差分平均逆滤波
        for y in 0..(height as usize) {
            let line = y * stride;
            for x in 0..(width as usize) {
                let pixel = line + x * pixel_size;
                for p in 0..pixel_size {
                    let mut avg = 0u32;
                    if x > 0 {
                        avg += output[pixel + p - pixel_size] as u32;
                    }
                    if y > 0 {
                        avg += output[pixel + p - stride] as u32;
                    }
                    if x > 0 && y > 0 {
                        avg /= 2;
                    }
                    if avg != 0 {
                        output[pixel + p] = output[pixel + p].wrapping_add(avg as u8);
                    }
                }
            }
        }

        // 8. 通道转换：标准输出统一转换为 RGBA8 布局
        let mut rgba_data = Vec::with_capacity((width * height * 4) as usize);
        if bpp == 32 {
            // 输入为 BGRA8，转为 RGBA8 (B 与 R 交换)
            for chunk in output.chunks_exact(4) {
                let b = chunk[0];
                let g = chunk[1];
                let r = chunk[2];
                let a = chunk[3];
                rgba_data.extend_from_slice(&[r, g, b, a]);
            }
        } else if bpp == 24 {
            // 输入为 BGR24，转为 RGBA8 (B 与 R 交换，A 填 255)
            for chunk in output.chunks_exact(3) {
                let b = chunk[0];
                let g = chunk[1];
                let r = chunk[2];
                rgba_data.extend_from_slice(&[r, g, b, 255]);
            }
        } else {
            // 输入为 8bpp 灰度遮罩，扩展为标准 RGBA8 ([v, v, v, 255])
            for &v in &output {
                rgba_data.extend_from_slice(&[v, v, v, 255]);
            }
        }

        Ok(DecodedImage {
            width,
            height,
            bpp,
            rgba_data,
        })
    }

    /// 解码 BGI 未压缩 16 字节头原始位图 (常见于汉化补丁散落 CG/卡片，如 `ch1_arisa0a`)
    ///
    /// 头部布局：`width (u16 LE)` + `height (u16 LE)` + `bpp (u32 LE, 24/32)` + `reserved (8B)`，
    /// 随后紧跟 `width * height * (bpp / 8)` 字节自上而下的 BGRA/BGR 像素流。
    pub fn decode_raw_bgi(data: &[u8]) -> Result<DecodedImage> {
        if data.len() < 16 {
            return Err(CoreError::InvalidFormat("BGI 原始位图长度不足 16 字节"));
        }
        let width = u16::from_le_bytes(data[0..2].try_into().unwrap()) as u32;
        let height = u16::from_le_bytes(data[2..4].try_into().unwrap()) as u32;
        let bpp = u32::from_le_bytes(data[4..8].try_into().unwrap());
        if width == 0 || height == 0 || (bpp != 24 && bpp != 32) {
            return Err(CoreError::InvalidFormat("BGI 原始位图几何或位深非法"));
        }
        let px_bytes = (bpp / 8) as usize;
        let expected_len = 16usize
            .checked_add((width as usize).saturating_mul(height as usize).saturating_mul(px_bytes))
            .ok_or(CoreError::OutOfBounds)?;
        if data.len() != expected_len {
            return Err(CoreError::InvalidFormat("BGI 原始位图字节长度与宽高不匹配"));
        }

        let pixels = &data[16..];
        let mut rgba_data = Vec::with_capacity((width * height * 4) as usize);
        if bpp == 32 {
            for chunk in pixels.chunks_exact(4) {
                rgba_data.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
            }
        } else {
            for chunk in pixels.chunks_exact(3) {
                rgba_data.extend_from_slice(&[chunk[2], chunk[1], chunk[0], 255]);
            }
        }

        Ok(DecodedImage {
            width,
            height,
            bpp,
            rgba_data,
        })
    }

    /// BGI LCG 密钥递推算式
    #[inline]
    fn update_key(key: &mut u32) -> u8 {
        let v0 = 20021_u32.wrapping_mul(*key & 0xFFFF);
        let v1 = 20021_u32.wrapping_mul(*key >> 16);
        let v2 = 346_u32
            .wrapping_mul(*key)
            .wrapping_add(v1)
            .wrapping_add(v0 >> 16);
        *key = (v2 << 16).wrapping_add(v0 & 0xFFFF).wrapping_add(1);
        (v2 & 0xFF) as u8
    }

    /// 解码 CompressedBG Version 2 (8x8 DCT + YCbCr + 独立 Alpha LZSS)
    fn decode_v2(
        width: u32,
        height: u32,
        bpp: u32,
        dec_data: &[u8],
        rest: &[u8],
    ) -> Result<DecodedImage> {
        if dec_data.len() < 128 {
            return Err(CoreError::InvalidFormat("CBG v2 量化表长度不足 128 字节"));
        }
        let y_quant = &dec_data[0..64];
        let c_quant = &dec_data[64..128];

        // 1. 从 rest 读取 Tree1 (0x10) 与 Tree2 (0xB0) LEB128 权重表
        let mut pos = 0usize;
        let mut t1_weights = [0u32; 0x10];
        for w in &mut t1_weights {
            *w = Self::read_leb128(rest, &mut pos)?;
        }
        let mut t2_weights = [0u32; 0xB0];
        for w in &mut t2_weights {
            *w = Self::read_leb128(rest, &mut pos)?;
        }

        let (tree1, root1) = Self::build_huffman_tree_v2(&t1_weights)?;
        let (tree2, root2) = Self::build_huffman_tree_v2(&t2_weights)?;

        // 2. 读取行块偏移表 (blocks_y + 1 个 u32 LE，相对于 rest 起始)
        let blocks_x = (width as usize + 7) / 8;
        let blocks_y = (height as usize + 7) / 8;
        let table_bytes = (blocks_y + 1) * 4;
        if pos + table_bytes > rest.len() {
            return Err(CoreError::InvalidFormat("CBG v2 行偏移表越界"));
        }

        let mut row_offsets = Vec::with_capacity(blocks_y + 1);
        for i in 0..=blocks_y {
            let mut off = u32::from_le_bytes(
                rest[pos + i * 4..pos + i * 4 + 4]
                    .try_into()
                    .unwrap(),
            ) as usize;
            // CBG v2 24bpp (无独立 Alpha 流) 的行偏移表末项使用 0xFFFFFFFF 哨兵，仅此时归一为 rest.len()
            if i == blocks_y && bpp == 24 && off == u32::MAX as usize {
                off = rest.len();
            }
            if off > rest.len() {
                return Err(CoreError::InvalidFormat("CBG v2 行偏移量超出数据范围"));
            }
            row_offsets.push(off);
        }

        // 3. 逐行块解码 8x8 DCT (Y, Cb, Cr) 并转换为 RGB
        let w = width as usize;
        let h = height as usize;
        let mut rgba_data = vec![0u8; w * h * 4];
        // 默认 Alpha 全 255 (24bpp 时直接使用；32bpp 时后续覆盖)
        for i in (3..rgba_data.len()).step_by(4) {
            rgba_data[i] = 255;
        }

        const ZIGZAG: [usize; 64] = [
            0, 1, 8, 16, 9, 2, 3, 10,
            17, 24, 32, 25, 18, 11, 4, 5,
            12, 19, 26, 33, 40, 48, 41, 34,
            27, 20, 13, 6, 7, 14, 21, 28,
            35, 42, 49, 56, 57, 50, 43, 36,
            29, 22, 15, 23, 30, 37, 44, 51,
            58, 59, 52, 45, 38, 31, 39, 46,
            53, 60, 61, 54, 47, 55, 62, 63,
        ];

        let pad_skip = (blocks_x + 7) / 8;
        let mut all_planes = vec![[[0i32; 64]; 3]; blocks_x];
        for by in 0..blocks_y {
            let start = row_offsets[by];
            let end = row_offsets[by + 1];
            if end < start || end > rest.len() || start + pad_skip > end {
                return Err(CoreError::InvalidFormat("CBG v2 行块范围无效"));
            }
            let mut row_pos = start + pad_skip;
            let block_size = Self::read_leb128(rest, &mut row_pos)? as usize;
            let mut reader = BitReader::new(&rest[row_pos..end]);

            for blk in &mut all_planes {
                *blk = [[0i32; 64]; 3];
            }
            let mut dc_prev = 0i32;
            let max_blocks = (block_size / 64).min(blocks_x * 3);

            // 先解码该行所有块的 DC 系数 (按平面 Y, Cb, Cr，共享 dc_prev 差分状态)
            let mut blk_idx = 0usize;
            'dc_loop: for c in 0..3 {
                let quant = if c == 0 { y_quant } else { c_quant };
                for bx in 0..blocks_x {
                    if blk_idx >= max_blocks {
                        break 'dc_loop;
                    }
                    let dc_len = Self::decode_huffman_symbol(&tree1, root1, &mut reader, 0x10)?;
                    let dc_diff = if dc_len > 0 {
                        reader.read_signed(dc_len)?
                    } else {
                        0
                    };
                    dc_prev = dc_prev.wrapping_add(dc_diff);
                    all_planes[bx][c][0] = dc_prev * (quant[0] as i32);
                    blk_idx += 1;
                }
            }
            reader.align_byte();

            // 再解码该行所有块的 AC 系数 (按平面 Y, Cb, Cr)
            blk_idx = 0;
            'ac_loop: for c in 0..3 {
                let quant = if c == 0 { y_quant } else { c_quant };
                for bx in 0..blocks_x {
                    if blk_idx >= max_blocks {
                        break 'ac_loop;
                    }
                    let mut k = 1usize;
                    while k < 64 {
                        let sym = match Self::decode_huffman_symbol(&tree2, root2, &mut reader, 0xB0) {
                            Ok(s) => s,
                            Err(_) => break 'ac_loop,
                        };
                        if sym == 0 {
                            break; // EOB
                        }
                        if sym == 0x0F {
                            k += 16;
                            continue;
                        }
                        let run = sym & 0x0F;
                        let size = sym >> 4;
                        k += run;
                        if k >= 64 {
                            break;
                        }
                        let val = if size > 0 {
                            match reader.read_signed(size) {
                                Ok(v) => v,
                                Err(_) => break 'ac_loop,
                            }
                        } else {
                            0
                        };
                        let zz = ZIGZAG[k];
                        all_planes[bx][c][zz] = val * (quant[zz] as i32);
                        k += 1;
                    }
                    blk_idx += 1;
                }
            }

            for bx in 0..blocks_x {
                // 执行 8x8 IDCT
                let y_blk = Self::idct_8x8(&all_planes[bx][0]);
                let cb_blk = Self::idct_8x8(&all_planes[bx][1]);
                let cr_blk = Self::idct_8x8(&all_planes[bx][2]);

                // YCbCr -> RGB 并写入输出缓冲
                let py_base = by * 8;
                let px_base = bx * 8;
                for dy in 0..8 {
                    let py = py_base + dy;
                    if py >= h {
                        break;
                    }
                    for dx in 0..8 {
                        let px = px_base + dx;
                        if px >= w {
                            break;
                        }
                        let idx8 = dy * 8 + dx;
                        let y_val = y_blk[idx8] + 128.0;
                        let cb_val = cb_blk[idx8];
                        let cr_val = cr_blk[idx8];

                        let r = Self::clamp_u8(y_val + 1.402 * cr_val);
                        let g = Self::clamp_u8(y_val - 0.344136 * cb_val - 0.714136 * cr_val);
                        let b = Self::clamp_u8(y_val + 1.772 * cb_val);

                        let out_idx = (py * w + px) * 4;
                        rgba_data[out_idx] = r;
                        rgba_data[out_idx + 1] = g;
                        rgba_data[out_idx + 2] = b;
                    }
                }
            }
        }

        // 4. 若为 32bpp，解压 Alpha 通道
        if bpp == 32 {
            let alpha_off = row_offsets[blocks_y];
            if alpha_off + 4 <= rest.len() {
                let alpha_stream = &rest[alpha_off..];
                let padded_w = blocks_x * 8;
                let padded_h = blocks_y * 8;
                Self::unpack_alpha_v2(alpha_stream, w, h, padded_w, padded_h, &mut rgba_data);
            }
        }

        Ok(DecodedImage {
            width,
            height,
            bpp,
            rgba_data,
        })
    }

    /// CBG v2 Alpha 通道解压 (支持 ctl == 0 原始流与 ctl == 1 2D LZSS)
    /// 对照 GARbro ImageCBG.cs::UnpackAlpha：
    /// 16 位回溯令牌 v -> x = i6(v & 0x3F), y = -8..0((v >> 6) & 7), count = ((v >> 9) & 0x7F) + 3
    fn unpack_alpha_v2(
        alpha_stream: &[u8],
        w: usize,
        h: usize,
        padded_w: usize,
        padded_h: usize,
        rgba_data: &mut [u8],
    ) {
        if alpha_stream.len() < 4 {
            return;
        }
        let ctl = u32::from_le_bytes(alpha_stream[0..4].try_into().unwrap());
        let total_px = w * h;
        if ctl == 0 {
            let copy_len = total_px.min(alpha_stream.len() - 4);
            for i in 0..copy_len {
                rgba_data[i * 4 + 3] = alpha_stream[4 + i];
            }
        } else if ctl == 1 {
            let padded_total_px = padded_w * padded_h;
            let mut alpha_buf = vec![0u8; padded_total_px];

            let mut src = 4usize;
            let mut dst = 0usize;
            while dst < padded_total_px && src < alpha_stream.len() {
                let flags = alpha_stream[src];
                src += 1;
                for bit in 0..8 {
                    if dst >= padded_total_px || src >= alpha_stream.len() {
                        break;
                    }
                    if (flags & (1 << bit)) == 0 {
                        alpha_buf[dst] = alpha_stream[src];
                        src += 1;
                        dst += 1;
                    } else {
                        if src + 1 >= alpha_stream.len() {
                            break;
                        }
                        let v = u16::from_le_bytes([alpha_stream[src], alpha_stream[src + 1]]);
                        src += 2;

                        let mut x = (v & 0x3F) as isize;
                        if x > 0x1F {
                            x -= 0x40;
                        }
                        let mut y = ((v >> 6) & 7) as isize;
                        if y != 0 {
                            y -= 8;
                        }
                        let count = ((v >> 9) & 0x7F) as usize + 3;

                        let mut copy_src = (dst as isize) + x + y * (padded_w as isize);
                        if copy_src < 0 || (copy_src as usize) >= dst {
                            dst = padded_total_px;
                            break;
                        }

                        for _ in 0..count {
                            if dst >= padded_total_px {
                                break;
                            }
                            alpha_buf[dst] = alpha_buf[copy_src as usize];
                            copy_src += 1;
                            dst += 1;
                        }
                    }
                }
            }

            for py in 0..h {
                let row_src = py * padded_w;
                let row_dst = py * w;
                for px in 0..w {
                    rgba_data[(row_dst + px) * 4 + 3] = alpha_buf[row_src + px];
                }
            }
        }
    }

    fn read_leb128(buf: &[u8], pos: &mut usize) -> Result<u32> {
        let mut v = 0u32;
        let mut shift = 0;
        loop {
            if *pos >= buf.len() || shift >= 32 {
                return Err(CoreError::InvalidFormat("CBG LEB128 解码越界"));
            }
            let code = buf[*pos];
            *pos += 1;
            v |= ((code & 0x7F) as u32) << shift;
            shift += 7;
            if (code & 0x80) == 0 {
                break;
            }
        }
        Ok(v)
    }

    /// 构建 CBG v2 规范 Huffman 树 (严格对齐 GARbro ImageCBG.cs::HuffmanTree v2=true 节点扫描规则)
    fn build_huffman_tree_v2(leaf_weights: &[u32]) -> Result<(Vec<HuffmanNode>, usize)> {
        let mut node_list = Vec::with_capacity(leaf_weights.len() * 2);
        let mut root_weight = 0u32;
        let mut valid_count = 0usize;
        let mut single_valid = 0usize;
        for (idx, &w) in leaf_weights.iter().enumerate() {
            let valid = w != 0;
            if valid {
                valid_count += 1;
                single_valid = idx;
            }
            node_list.push(HuffmanNode {
                weight: w,
                left: 0,
                right: 0,
                valid,
            });
            root_weight = root_weight.wrapping_add(w);
        }

        if valid_count == 0 {
            return Ok((node_list, 0));
        }
        if valid_count == 1 {
            return Ok((node_list, single_valid));
        }

        loop {
            let mut child = [usize::MAX, usize::MAX];
            let mut weight = 0u32;

            for i in 0..2 {
                let mut min_weight = u32::MAX;
                let mut n = 0usize;
                while n < node_list.len() {
                    if node_list[n].valid {
                        min_weight = node_list[n].weight;
                        child[i] = n;
                        n += 1;
                        break;
                    }
                    n += 1;
                }
                n = n.max(i + 1);
                while n < node_list.len() {
                    if node_list[n].valid && node_list[n].weight < min_weight {
                        min_weight = node_list[n].weight;
                        child[i] = n;
                    }
                    n += 1;
                }
                if child[i] == usize::MAX {
                    continue;
                }
                node_list[child[i]].valid = false;
                weight = weight.wrapping_add(min_weight);
            }

            if child[0] == usize::MAX || child[1] == usize::MAX {
                break;
            }

            let parent_node = HuffmanNode {
                weight,
                left: child[0] as u16,
                right: child[1] as u16,
                valid: true,
            };
            node_list.push(parent_node);
            if weight >= root_weight {
                break;
            }
        }

        let root_node = node_list.len() - 1;
        Ok((node_list, root_node))
    }

    fn decode_huffman_symbol(
        tree: &[HuffmanNode],
        root: usize,
        reader: &mut BitReader<'_>,
        leaf_count: usize,
    ) -> Result<usize> {
        let mut curr = root;
        while curr >= leaf_count {
            let bit = reader.read_bit()?;
            curr = if bit == 0 {
                tree[curr].left as usize
            } else {
                tree[curr].right as usize
            };
        }
        Ok(curr)
    }

    #[inline(always)]
    fn clamp_u8(v: f32) -> u8 {
        if v <= 0.0 {
            0
        } else if v >= 255.0 {
            255
        } else {
            (v + 0.5) as u8
        }
    }

    fn idct_8x8(coeff: &[i32; 64]) -> [f32; 64] {
        // 快速通道：纯 DC 或全零块无需执行 8x8 行列乘加循环（透明留白与色度平坦块占绝大多数）
        if coeff[1..].iter().all(|&v| v == 0) {
            if coeff[0] == 0 {
                return [0.0f32; 64];
            }
            return [(coeff[0] as f32) * 0.125; 64];
        }

        // 预计算 8x8 1D IDCT 基矩阵 (含 0.5 * cu 系数)，消除运行时三角函数 cos() 调用
        const IDCT_BASIS: [[f32; 8]; 8] = [
            [0.353553391, 0.353553391, 0.353553391, 0.353553391, 0.353553391, 0.353553391, 0.353553391, 0.353553391],
            [0.490392640, 0.415734806, 0.277785117, 0.097545161, -0.097545161, -0.277785117, -0.415734806, -0.490392640],
            [0.461939766, 0.191341716, -0.191341716, -0.461939766, -0.461939766, -0.191341716, 0.191341716, 0.461939766],
            [0.415734806, -0.097545161, -0.490392640, -0.277785117, 0.277785117, 0.490392640, 0.097545161, -0.415734806],
            [0.353553391, -0.353553391, -0.353553391, 0.353553391, 0.353553391, -0.353553391, -0.353553391, 0.353553391],
            [0.277785117, -0.490392640, 0.097545161, 0.415734806, -0.415734806, -0.097545161, 0.490392640, -0.277785117],
            [0.191341716, -0.461939766, 0.461939766, -0.191341716, -0.191341716, 0.461939766, -0.461939766, 0.191341716],
            [0.097545161, -0.277785117, 0.415734806, -0.490392640, 0.490392640, -0.415734806, 0.277785117, -0.097545161],
        ];

        let mut tmp = [0.0f32; 64];
        // 行变换：跳过全零行或零系数，快速处理纯 DC 块
        for y in 0..8 {
            let row = y * 8;
            let mut has_ac = false;
            for u in 1..8 {
                if coeff[row + u] != 0 {
                    has_ac = true;
                    break;
                }
            }
            let c0 = coeff[row];
            if !has_ac {
                if c0 != 0 {
                    let dc = (c0 as f32) * IDCT_BASIS[0][0];
                    tmp[row..row + 8].fill(dc);
                }
                continue;
            }
            for x in 0..8 {
                let mut sum = 0.0f32;
                for u in 0..8 {
                    let cu_val = coeff[row + u];
                    if cu_val != 0 {
                        sum += (cu_val as f32) * IDCT_BASIS[u][x];
                    }
                }
                tmp[row + x] = sum;
            }
        }

        let mut out = [0.0f32; 64];
        // 列变换
        for x in 0..8 {
            for y in 0..8 {
                let mut sum = 0.0f32;
                for v in 0..8 {
                    let tv = tmp[v * 8 + x];
                    if tv != 0.0 {
                        sum += tv * IDCT_BASIS[v][y];
                    }
                }
                out[y * 8 + x] = sum;
            }
        }
        out
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bit_pos: 0,
        }
    }

    #[inline]
    fn align_byte(&mut self) {
        if self.bit_pos != 0 {
            self.bit_pos = 0;
            self.pos += 1;
        }
    }

    #[inline]
    fn read_bit(&mut self) -> Result<u32> {
        if self.pos >= self.data.len() {
            return Err(CoreError::InvalidFormat("CBG v2 比特流意外终止"));
        }
        let bit = (self.data[self.pos] >> (7 - self.bit_pos)) & 1;
        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.bit_pos = 0;
            self.pos += 1;
        }
        Ok(bit as u32)
    }

    #[inline]
    fn read_bits(&mut self, count: usize) -> Result<u32> {
        let mut val = 0u32;
        for _ in 0..count {
            val = (val << 1) | self.read_bit()?;
        }
        Ok(val)
    }

    #[inline]
    fn read_signed(&mut self, count: usize) -> Result<i32> {
        let bits = self.read_bits(count)?;
        let half = 1u32 << (count - 1);
        if bits < half {
            Ok((bits as i32) - ((1i32 << count) - 1))
        } else {
            Ok(bits as i32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::VirtualFileSystem;
    use std::path::Path;

    #[test]
    fn test_cbg_v2_alpha_2d_lzss_and_huffman_v2() {
        // 1. 验证 2D LZSS Alpha 解压 (8x8 对齐缓冲，x 有正负偏移，y 有负偏移与重叠 RLE)
        let w = 6usize;
        let h = 4usize;
        let padded_w = 8usize;
        let padded_h = 8usize;
        let mut rgba = vec![0u8; w * h * 4];

        // 构造 ctl = 1 流：单字节 flags = 0b0000_1110
        // bit0=0: lit 255 @ dst=0 (dst->1)
        // bit1=1: match x=-1, y=0, count=7 (dst 1..8 全填 255，测试 RLE 重叠复制)
        //   x=-1 => 0x3F, y=0 => 0, count=7 => (7-3)<<9 = 0x0800 -> v = 0x083F
        // bit2=1: match x=0, y=-1, count=24 (dst 8..32 复制自第0..2行)
        //   x=0 => 0, y=-1 => 7<<6 = 0x01C0, count=24 => (24-3)<<9 = 0x2A00 -> v = 0x2BC0
        // bit3=1: match x=0, y=-4, count=32 (dst 32..64 填满剩余 8x8 缓冲，触发 dst >= padded_total_px 结束)
        //   x=0 => 0, y=-4 => 4<<6 = 0x0100, count=32 => (32-3)<<9 = 0x3A00 -> v = 0x3B00
        let mut stream = vec![1, 0, 0, 0];
        stream.push(0b0000_1110);
        stream.push(255);
        stream.extend_from_slice(&0x083Fu16.to_le_bytes());
        stream.extend_from_slice(&0x2BC0u16.to_le_bytes());
        stream.extend_from_slice(&0x3B00u16.to_le_bytes());

        CompressedBg::unpack_alpha_v2(&stream, w, h, padded_w, padded_h, &mut rgba);
        for py in 0..h {
            for px in 0..w {
                assert_eq!(rgba[(py * w + px) * 4 + 3], 255, "({},{}) 必须全为 255", px, py);
            }
        }

        // 2. 边界防越界测试：非法负偏移不得 panic
        let mut bad_rgba = vec![0u8; 4 * 4 * 4];
        let bad_stream = [1, 0, 0, 0, 0x01, 0x3F, 0x00]; // dst=0 时 x=-1 越界
        CompressedBg::unpack_alpha_v2(&bad_stream, 4, 4, 8, 8, &mut bad_rgba);
        assert_eq!(bad_rgba[3], 0);

        // 3. 验证 build_huffman_tree_v2 空表/单节点安全返回
        let empty_weights = [0u32; 16];
        let (_, r_empty) = CompressedBg::build_huffman_tree_v2(&empty_weights).unwrap();
        assert_eq!(r_empty, 0);

        let mut single_weights = [0u32; 16];
        single_weights[5] = 42;
        let (_, r_single) = CompressedBg::build_huffman_tree_v2(&single_weights).unwrap();
        assert_eq!(r_single, 5);
    }

    #[test]
    fn test_idct_8x8_matches_reference_cosine_formula() {
        let mut coeff = [0i32; 64];
        coeff[0] = 512;
        coeff[1] = -120;
        coeff[8] = 85;
        coeff[9] = 44;
        coeff[19] = -33;
        coeff[63] = 17;

        let fast = CompressedBg::idct_8x8(&coeff);
        const C_NORM: f32 = 0.70710678;
        const PI_16: f32 = std::f32::consts::PI / 16.0;
        let mut tmp = [0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                let mut sum = 0.0f32;
                for u in 0..8 {
                    let cu = if u == 0 { C_NORM } else { 1.0 };
                    sum += cu * (coeff[y * 8 + u] as f32) * (((2 * x + 1) * u) as f32 * PI_16).cos();
                }
                tmp[y * 8 + x] = sum * 0.5;
            }
        }
        let mut reference = [0.0f32; 64];
        for x in 0..8 {
            for y in 0..8 {
                let mut sum = 0.0f32;
                for v in 0..8 {
                    let cv = if v == 0 { C_NORM } else { 1.0 };
                    sum += cv * tmp[v * 8 + x] * (((2 * y + 1) * v) as f32 * PI_16).cos();
                }
                reference[y * 8 + x] = sum * 0.5;
            }
        }
        for i in 0..64 {
            assert!(
                (fast[i] - reference[i]).abs() < 1e-4,
                "IDCT idx {} mismatch: fast={} ref={}",
                i,
                fast[i],
                reference[i]
            );
        }
    }

    #[test]
    fn test_decode_cbg_v2_real_assets() {
        let arc_2300 = Path::new("U:/anti/projects/PURExCONNECT/Archive/data02300.arc");
        let arc_2530 = Path::new("U:/anti/projects/PURExCONNECT/Archive/data02530.arc");
        if !arc_2300.exists() || !arc_2530.exists() {
            return;
        }
        let mut vfs = VirtualFileSystem::new();
        vfs.mount_archive(arc_2300).unwrap();
        vfs.mount_archive(arc_2530).unwrap();

        std::fs::create_dir_all("U:/anti/scratchpad/261121").unwrap();

        for name in ["ar01aa_n", "ar01aa_l", "so01aa_n", "so01aa_l", "si01aa_n", "si01aa_l", "ay01aa_n", "ay01aa_l", "mo01aa_m", "mo01aa_l", "ch1_arisa0a"] {
            let raw = vfs.read(name).unwrap();
            let img = CompressedBg::decode(&raw)
                .unwrap_or_else(|e| panic!("CBG v2 解码 {} 失败: {:?}", name, e));
            println!("Decoded {}: {}x{}, bpp={}", name, img.width, img.height, img.bpp);

            if name == "ar01aa_n" {
                // 外围背景区域必须全部为 0 透明，0 噪点
                for y in 0..100usize {
                    for x in 0..100usize {
                        let a_left = img.rgba_data[(y * img.width as usize + x) * 4 + 3];
                        let a_right = img.rgba_data[(y * img.width as usize + (img.width as usize - 1 - x)) * 4 + 3];
                        assert_eq!(a_left, 0, "ar01aa_n 左上角 ({},{}) Alpha 必须为 0", x, y);
                        assert_eq!(a_right, 0, "ar01aa_n 右上角 Alpha 必须为 0");
                    }
                }
            }

            if name == "ch1_arisa0a" {
                // 外围背景区域必须全部为 0 透明
                for y in 0..100usize {
                    for x in 0..100usize {
                        let a = img.rgba_data[(y * img.width as usize + x) * 4 + 3];
                        assert_eq!(a, 0, "ch1_arisa0a 左上角 ({},{}) Alpha 必须为 0", x, y);
                    }
                }
                // 手机屏幕内部区域必须全 255 不透明且 0 孔洞噪点
                for y in 100..500usize {
                    for x in 550..700usize {
                        let a = img.rgba_data[(y * img.width as usize + x) * 4 + 3];
                        assert_eq!(a, 255, "ch1_arisa0a 手机内部 ({},{}) Alpha 必须为 255", x, y);
                    }
                }
            }

            let out_path = format!("U:/anti/scratchpad/261121/{}.png", name);
            image::save_buffer(
                &out_path,
                &img.rgba_data,
                img.width,
                img.height,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
    }

    #[test]
    fn test_decode_cbg_v2_24bpp_arev_and_sentinel_guard() {
        let arc_2400 = Path::new("U:/anti/projects/PURExCONNECT/Archive/data02400.arc");
        if !arc_2400.exists() {
            return;
        }
        let arc = crate::archive::Arc20Archive::open(arc_2400).unwrap();
        let mut arev_names: Vec<String> = arc
            .iter_entries()
            .map(|e| e.name.as_str())
            .filter(|n| {
                let lo = n.to_ascii_lowercase();
                lo.starts_with("arev07") || lo.starts_with("arev08")
            })
            .map(|s| s.to_string())
            .collect();
        arev_names.sort();
        assert!(!arev_names.is_empty(), "data02400.arc 中必须存在 arev07*/arev08*");

        std::fs::create_dir_all("U:/anti/temp/verify_shots").unwrap();
        let mut exported = 0usize;
        for name in &arev_names {
            let raw = arc.read(name).unwrap();
            let img = CompressedBg::decode(raw)
                .unwrap_or_else(|e| panic!("CBG v2 24bpp 解码 {} 失败: {:?}", name, e));
            assert_eq!(img.bpp, 24, "{} 应为 24bpp", name);
            assert!(img.width > 0 && img.height > 0);
            assert_eq!(img.rgba_data.len(), (img.width * img.height * 4) as usize);
            assert!(
                img.rgba_data.chunks_exact(4).all(|px| px[3] == 255),
                "{} 24bpp 图像 Alpha 通道必须全为 255",
                name
            );
            if exported < 3 && (name.eq_ignore_ascii_case("arev07a") || name.eq_ignore_ascii_case("arev07b") || name.to_ascii_lowercase().starts_with("arev08")) {
                let out_path = format!("U:/anti/temp/verify_shots/batch1_{}.png", name.to_ascii_lowercase());
                image::save_buffer(
                    &out_path,
                    &img.rgba_data,
                    img.width,
                    img.height,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                exported += 1;
            }
        }
        assert!(exported >= 2, "必须导出至少 2 张 arev07*/arev08* 核验 PNG");

        // 严格防放宽校验：构造中间行被篡改为 0xFFFFFFFF 的 24bpp v2 数据，必须返回 Err
        let mut corrupted = arc.read("arev07a").unwrap().to_vec();
        let enc_len = u32::from_le_bytes(corrupted[0x28..0x2C].try_into().unwrap()) as usize;
        let rest_start = 0x30 + enc_len;
        let mut pos = 0usize;
        for _ in 0..0x10 {
            let _ = CompressedBg::read_leb128(&corrupted[rest_start..], &mut pos).unwrap();
        }
        for _ in 0..0xB0 {
            let _ = CompressedBg::read_leb128(&corrupted[rest_start..], &mut pos).unwrap();
        }
        // 篡改第 1 行偏移（非末项）为 0xFFFFFFFF
        let mid_off_pos = rest_start + pos + 4;
        corrupted[mid_off_pos..mid_off_pos + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            CompressedBg::decode(&corrupted).is_err(),
            "中间行出现 0xFFFFFFFF 偏移必须严格报错，绝不能被误放行"
        );
    }

    #[test]
    fn test_decode_cbg_v1_8bpp_effect_masks() {
        let arc_2120 = Path::new("U:/anti/projects/PURExCONNECT/Archive/data02120.arc");
        if !arc_2120.exists() {
            return;
        }
        let arc = crate::archive::Arc20Archive::open(arc_2120).unwrap();
        let mut effect_names: Vec<String> = arc
            .iter_entries()
            .map(|e| e.name.as_str())
            .filter(|n| n.to_ascii_lowercase().starts_with("effect"))
            .map(|s| s.to_string())
            .collect();
        effect_names.sort();
        assert_eq!(effect_names.len(), 16, "data02120.arc 应包含 16 张 effect* 遮罩");

        for name in &effect_names {
            let raw = arc.read(name).unwrap();
            let img = CompressedBg::decode(raw)
                .unwrap_or_else(|e| panic!("CBG v1 8bpp 解码 {} 失败: {:?}", name, e));
            assert_eq!(img.bpp, 8, "{} 应为 8bpp 灰度遮罩", name);
            assert!(img.width > 0 && img.height > 0);
            assert_eq!(img.rgba_data.len(), (img.width * img.height * 4) as usize);
            for px in img.rgba_data.chunks_exact(4) {
                assert_eq!(px[0], px[1], "{} R==G", name);
                assert_eq!(px[1], px[2], "{} G==B", name);
                assert_eq!(px[3], 255, "{} A==255", name);
            }
        }
    }
}
