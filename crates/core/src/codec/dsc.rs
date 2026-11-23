use std::collections::HashMap;

use crate::error::{CoreError, Result};

/// BGI DSC (`DSC FORMAT 1.00`) 解压器
///
/// 用于解压 `data060*.arc` 中的 `animationset*` / `animation*` 等系统表与定义文件。
pub struct Dsc;

impl Dsc {
    /// 解压 `DSC FORMAT 1.00` 数据块
    pub fn decode(data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 0x220 {
            return Err(CoreError::InvalidFormat("DSC 数据长度不足 0x220 字节"));
        }
        if &data[0x00..0x10] != b"DSC FORMAT 1.00\0" {
            return Err(CoreError::InvalidFormat("魔数不是 DSC FORMAT 1.00"));
        }

        let mut key = u32::from_le_bytes(data[0x10..0x14].try_into().unwrap());
        let unpacked_size = u32::from_le_bytes(data[0x14..0x18].try_into().unwrap()) as usize;
        let dec_count = u32::from_le_bytes(data[0x18..0x1C].try_into().unwrap()) as usize;

        // 1. 解密 512 字节码长表并按 (code_len, symbol) 排序
        let mut nodes_info: Vec<(u8, u16)> = Vec::with_capacity(512);
        for sym in 0..512u16 {
            let k = Self::update_key(&mut key);
            let code_len = data[0x20 + sym as usize].wrapping_sub(k);
            if code_len != 0 {
                nodes_info.push((code_len, sym));
            }
        }
        nodes_info.sort_unstable();

        if nodes_info.is_empty() {
            return Ok(Vec::new());
        }

        // 2. 构建规范 Huffman 二叉树（正数表示内部节点索引，负数表示叶子 -(sym + 1)）
        let mut tree: Vec<[i32; 2]> = vec![[0, 0]];
        let mut cur_level: Vec<usize> = vec![0];
        let mut info_pos = 0usize;
        let mut bit_len = 1u8;

        while info_pos < nodes_info.len() {
            if cur_level.is_empty() || bit_len > 32 {
                return Err(CoreError::InvalidFormat("DSC Huffman 树构建失败或码长异常"));
            }
            let mut next_level = Vec::new();
            for parent in cur_level {
                for side in 0..2 {
                    if info_pos < nodes_info.len() && nodes_info[info_pos].0 == bit_len {
                        let sym = nodes_info[info_pos].1 as i32;
                        info_pos += 1;
                        tree[parent][side] = -(sym + 1);
                    } else {
                        let child_idx = tree.len();
                        tree.push([0, 0]);
                        tree[parent][side] = child_idx as i32;
                        next_level.push(child_idx);
                    }
                }
            }
            cur_level = next_level;
            bit_len += 1;
        }

        // 3. MSB-first 解码比特流并执行 LZSS 回溯复制
        let mut out = Vec::with_capacity(unpacked_size);
        let mut reader = DscBitReader::new(&data[0x220..]);

        for _ in 0..dec_count {
            let mut node = 0i32;
            while node >= 0 {
                let bit = reader.read_bit()?;
                let idx = node as usize;
                if idx >= tree.len() {
                    return Err(CoreError::InvalidFormat("DSC Huffman 节点索引越界"));
                }
                node = tree[idx][bit as usize];
            }
            let sym = (-(node + 1)) as u16;
            if sym >= 256 {
                let count = ((sym & 0xFF) as usize) + 2;
                let offset = (reader.read_bits(12)? as usize) + 2;
                if offset > out.len() {
                    return Err(CoreError::InvalidFormat("DSC LZSS 回溯偏移越界"));
                }
                let start = out.len() - offset;
                for i in 0..count {
                    let b = out[start + i];
                    out.push(b);
                }
            } else {
                out.push(sym as u8);
            }
        }

        Ok(out)
    }

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
}

struct DscBitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_pos: u8,
}

impl<'a> DscBitReader<'a> {
    #[inline]
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bit_pos: 0,
        }
    }

    #[inline]
    fn read_bit(&mut self) -> Result<u32> {
        if self.pos >= self.data.len() {
            return Err(CoreError::InvalidFormat("DSC 比特流意外终止"));
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
}

/// 解析 `BurikoAnimationDefinitionSet` (`animationset000..050`) 宏展开表
///
/// 每条记录由 1 个宏代号（如 `SI01_N013`）+ 16 个分层部件槽位组成，
/// 统一转为小写键以支持大小写不敏感检索。
pub struct AnimationSetTable;

impl AnimationSetTable {
    pub fn parse_into(data: &[u8], out: &mut HashMap<String, Vec<String>>) -> Result<usize> {
        if data.len() < 0x40 || !data.starts_with(b"BurikoAnimationDefinitionSet\0") {
            return Err(CoreError::InvalidFormat("非法的 BurikoAnimationDefinitionSet 头"));
        }
        let count = u32::from_le_bytes(data[0x28..0x2C].try_into().unwrap()) as usize;
        let mut pos = 0x40usize;
        let mut added = 0usize;

        for _ in 0..count {
            let mut strs = Vec::with_capacity(17);
            for _ in 0..17 {
                if pos + 2 > data.len() {
                    return Err(CoreError::OutOfBounds);
                }
                let len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
                pos += 2;
                if pos + len > data.len() {
                    return Err(CoreError::OutOfBounds);
                }
                let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(&data[pos..pos + len]);
                strs.push(cow.into_owned());
                pos += len;
            }
            let macro_name = strs[0].trim().to_ascii_lowercase();
            let layers: Vec<String> = strs[1..]
                .iter()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            if !macro_name.is_empty() && !layers.is_empty() {
                out.insert(macro_name, layers);
                added += 1;
            }
        }
        Ok(added)
    }
}

/// 解析 `BurikoAnimationDefinition` (`animation000..050`) 动画别名表
///
/// 将动画代号（如 `ay01_n_h_dd`）映射至实际底图资产名（如 `ay01_n_h_db`）。
pub struct AnimationDefTable;

impl AnimationDefTable {
    pub fn parse_into(data: &[u8], out: &mut HashMap<String, String>) -> Result<usize> {
        if data.len() < 0x40 || !data.starts_with(b"BurikoAnimationDefinition\0") {
            return Err(CoreError::InvalidFormat("非法的 BurikoAnimationDefinition 头"));
        }
        let count = u32::from_le_bytes(data[0x28..0x2C].try_into().unwrap()) as usize;
        let mut pos = 0x40usize;
        let mut added = 0usize;

        for _ in 0..count {
            if pos + 2 > data.len() {
                return Err(CoreError::OutOfBounds);
            }
            let l1 = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
            pos += 2;
            if pos + l1 + 2 > data.len() {
                return Err(CoreError::OutOfBounds);
            }
            let (cow1, _, _) = encoding_rs::SHIFT_JIS.decode(&data[pos..pos + l1]);
            let anim_name = cow1.trim().to_ascii_lowercase();
            pos += l1;

            let l2 = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
            pos += 2;
            if pos + l2 + 162 > data.len() {
                return Err(CoreError::OutOfBounds);
            }
            let (cow2, _, _) = encoding_rs::SHIFT_JIS.decode(&data[pos..pos + l2]);
            let asset_name = cow2.trim().to_ascii_lowercase();
            pos += l2 + 162;

            if !anim_name.is_empty() && !asset_name.is_empty() && anim_name != asset_name {
                out.insert(anim_name, asset_name);
                added += 1;
            }
        }
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::VirtualFileSystem;
    use std::path::Path;

    #[test]
    fn test_dsc_and_animationset_real_archive() {
        let arc_6020 = Path::new(r"U:\anti\projects\PURExCONNECT\Archive\data06020.arc");
        if !arc_6020.exists() {
            return;
        }
        let mut vfs = VirtualFileSystem::new();
        vfs.mount_archive(arc_6020).unwrap();

        let aset_raw = vfs.read("animationset020").unwrap();
        let aset_dec = Dsc::decode(&aset_raw).expect("DSC 解压 animationset020 失败");
        let mut macros = HashMap::new();
        let count = AnimationSetTable::parse_into(&aset_dec, &mut macros)
            .expect("解析 animationset020 失败");
        assert!(count > 600, "animationset020 应包含 600+ 有效宏");

        let si013 = macros.get("si01_n013").expect("必须包含 si01_n013");
        assert!(si013.contains(&"si01_n_e_a07".to_string()));
        assert!(si013.contains(&"si01_n_m_a07".to_string()));

        let anim_raw = vfs.read("animation020").unwrap();
        let anim_dec = Dsc::decode(&anim_raw).expect("DSC 解压 animation020 失败");
        let mut aliases = HashMap::new();
        let alias_cnt = AnimationDefTable::parse_into(&anim_dec, &mut aliases)
            .expect("解析 animation020 失败");
        assert!(alias_cnt > 0);
        assert_eq!(
            aliases.get("ay01_n_h_dd").map(String::as_str),
            Some("ay01_n_h_db")
        );
    }

    #[test]
    fn test_dsc_invalid_header() {
        let bad = vec![0u8; 0x220];
        assert!(Dsc::decode(&bad).is_err());
    }
}
