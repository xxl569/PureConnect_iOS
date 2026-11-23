//! BGI / Buriko 二进制剧本字节码解析器与字符串解码器

use std::borrow::Cow;
use crate::error::{CoreError, Result};
use super::opcode::OpCode;

/// BGI 编译脚本标准魔数 (28 字节)
pub const BGI_SCRIPT_MAGIC: &[u8; 28] = b"BurikoCompiledScriptVer1.00\0";

/// BGI 二进制剧本解析体
#[derive(Debug, Clone)]
pub struct BgiScript<'a> {
    raw_data: &'a [u8],
    header_size: u32,
    base_offset: usize,
    modules: Vec<String>,
}

/// 单条字节码反汇编指令单元
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawInstruction {
    /// 该指令在字节码中的绝对字节偏移
    pub pc: usize,
    /// 操作码
    pub opcode: OpCode,
    /// 操作数 (例如字符串偏移或立即数)
    pub operand: Option<u32>,
    /// 指令占用字节数 (4 或 8 字节)
    pub byte_size: usize,
}

impl<'a> BgiScript<'a> {
    /// 从原始字节流解析 BGI 编译剧本
    pub fn parse(raw_data: &'a [u8]) -> Result<Self> {
        if raw_data.len() < 32 {
            return Err(CoreError::OutOfBounds);
        }

        // 1. 验证魔数
        if !raw_data.starts_with(BGI_SCRIPT_MAGIC) {
            let magic_chunk = raw_data[..28.min(raw_data.len())].to_vec();
            return Err(CoreError::InvalidScriptMagic(magic_chunk));
        }

        // 2. 读取魔数后的附加头部大小 (DWORD)
        let header_size = u32::from_le_bytes(
            raw_data[28..32]
                .try_into()
                .map_err(|_| CoreError::OutOfBounds)?,
        );

        let base_offset = 28 + header_size as usize;
        if raw_data.len() < base_offset {
            return Err(CoreError::OutOfBounds);
        }

        // 3. 解析依赖模块列表 (头部元数据)
        let mut modules = Vec::new();
        if header_size >= 8 {
            let mod_data = &raw_data[32..base_offset];
            if mod_data.len() >= 4 {
                let mod_count =
                    u32::from_le_bytes(mod_data[..4].try_into().unwrap_or([0; 4])) as usize;
                let mut cursor = 4;
                for _ in 0..mod_count {
                    if cursor >= mod_data.len() {
                        break;
                    }
                    if let Some(null_pos) = mod_data[cursor..].iter().position(|&b| b == 0) {
                        let name_slice = &mod_data[cursor..cursor + null_pos];
                        if let Ok(name) = std::str::from_utf8(name_slice) {
                            if !name.is_empty() {
                                modules.push(name.to_string());
                            }
                        }
                        cursor += null_pos + 1;
                    } else {
                        break;
                    }
                }
            }
        }

        Ok(Self {
            raw_data,
            header_size,
            base_offset,
            modules,
        })
    }

    /// 获取脚本的代码段与数据段基址偏移 (字节)
    #[inline]
    pub fn base_offset(&self) -> usize {
        self.base_offset
    }

    /// 获取头部长度
    #[inline]
    pub fn header_size(&self) -> u32 {
        self.header_size
    }

    /// 依赖模块列表
    #[inline]
    pub fn modules(&self) -> &[String] {
        &self.modules
    }

    /// 原始数据总字节大小
    #[inline]
    pub fn len(&self) -> usize {
        self.raw_data.len()
    }

    /// 是否为空
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.raw_data.is_empty()
    }

    /// 原始数据切片
    #[inline]
    pub fn raw_data(&self) -> &'a [u8] {
        self.raw_data
    }

    /// 从指定绝对偏移读取一个 32 位小端整数
    #[inline]
    pub fn read_u32(&self, abs_offset: usize) -> Result<u32> {
        let end = abs_offset.checked_add(4).ok_or(CoreError::OutOfBounds)?;
        if end > self.raw_data.len() {
            return Err(CoreError::OutOfBounds);
        }
        let bytes: [u8; 4] = self.raw_data[abs_offset..end]
            .try_into()
            .map_err(|_| CoreError::OutOfBounds)?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// 从指定绝对偏移读取一个 32 位小端有符号整数
    #[inline]
    pub fn read_i32(&self, abs_offset: usize) -> Result<i32> {
        let end = abs_offset.checked_add(4).ok_or(CoreError::OutOfBounds)?;
        if end > self.raw_data.len() {
            return Err(CoreError::OutOfBounds);
        }
        let bytes: [u8; 4] = self.raw_data[abs_offset..end]
            .try_into()
            .map_err(|_| CoreError::OutOfBounds)?;
        Ok(i32::from_le_bytes(bytes))
    }

    /// 根据相对偏移 (相对于 base_offset) 读取原始未解码以空字符结尾的字符串切片 (零拷贝)
    pub fn get_string_raw(&self, rel_offset: u32) -> Result<&'a [u8]> {
        let abs_pos = self
            .base_offset
            .checked_add(rel_offset as usize)
            .ok_or(CoreError::OutOfBounds)?;
        if abs_pos >= self.raw_data.len() {
            return Err(CoreError::OutOfBounds);
        }

        let slice = &self.raw_data[abs_pos..];
        let len = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        Ok(&slice[..len])
    }

    /// 根据相对偏移读取并执行 GBK 简体中文转码 (返回 Cow<str>)
    pub fn get_string_gbk(&self, rel_offset: u32) -> Result<Cow<'a, str>> {
        let raw = self.get_string_raw(rel_offset)?;
        let (cow, _, _) = encoding_rs::GBK.decode(raw);
        Ok(cow)
    }

    /// 根据相对偏移读取 String
    pub fn get_string(&self, rel_offset: u32) -> Result<String> {
        Ok(self.get_string_gbk(rel_offset)?.into_owned())
    }

    /// 读取一条完整原始指令并返回其大小 (用于单步或反汇编遍历)
    pub fn read_instruction(&self, pc: usize) -> Result<RawInstruction> {
        let word = self.read_u32(pc)?;
        let opcode = OpCode(word);

        if opcode.is_8byte_instruction() {
            // 8 字节指令 (4B opcode + 4B inline arg): {0x0000, 0x0001, 0x0002, 0x0003, 0x0009, 0x0017, 0x0019}
            let operand_pos = pc.checked_add(4).ok_or(CoreError::OutOfBounds)?;
            let operand = self.read_u32(operand_pos)?;
            Ok(RawInstruction {
                pc,
                opcode,
                operand: Some(operand),
                byte_size: 8,
            })
        } else {
            // 其余为标准 4 字节长指令字 (含 0x0010, 0x0011, 0x0018 等)
            Ok(RawInstruction {
                pc,
                opcode,
                operand: None,
                byte_size: 4,
            })
        }
    }
}

/// 拥有底层字节所有权的 BGI 编译剧本（零 unsafe，支持跨脚本调用栈与廉价克隆）
#[derive(Debug, Clone)]
pub struct OwnedBgiScript {
    raw_data: std::sync::Arc<Vec<u8>>,
    header_size: u32,
    base_offset: usize,
    modules: Vec<String>,
}

impl OwnedBgiScript {
    /// 从 Arc<Vec<u8>> 解析 BGI 编译剧本
    pub fn parse(raw_data: std::sync::Arc<Vec<u8>>) -> Result<Self> {
        let parsed = BgiScript::parse(raw_data.as_slice())?;
        Ok(Self {
            header_size: parsed.header_size,
            base_offset: parsed.base_offset,
            modules: parsed.modules,
            raw_data,
        })
    }

    #[inline]
    pub fn as_script(&self) -> BgiScript<'_> {
        BgiScript {
            raw_data: self.raw_data.as_slice(),
            header_size: self.header_size,
            base_offset: self.base_offset,
            modules: Vec::new(),
        }
    }

    #[inline]
    pub fn base_offset(&self) -> usize {
        self.base_offset
    }

    #[inline]
    pub fn header_size(&self) -> u32 {
        self.header_size
    }

    #[inline]
    pub fn modules(&self) -> &[String] {
        &self.modules
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.raw_data.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.raw_data.is_empty()
    }

    #[inline]
    pub fn read_u32(&self, abs_offset: usize) -> Result<u32> {
        self.as_script().read_u32(abs_offset)
    }

    #[inline]
    pub fn read_i32(&self, abs_offset: usize) -> Result<i32> {
        self.as_script().read_i32(abs_offset)
    }

    #[inline]
    pub fn get_string(&self, rel_offset: u32) -> Result<String> {
        self.as_script().get_string(rel_offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_magic() {
        let fake_data = b"InvalidMagicHeaderForTestingScript\0\0\0\0";
        let res = BgiScript::parse(fake_data);
        assert!(res.is_err());
        match res.unwrap_err() {
            CoreError::InvalidScriptMagic(magic) => {
                assert!(magic.starts_with(b"InvalidMagic"));
            }
            other => panic!("预期 InvalidScriptMagic，得到 {:?}", other),
        }
    }

    #[test]
    fn test_synthetic_script_decode() {
        let mut data = Vec::new();
        // 1. 魔数 28 字节
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        // 2. 头部大小 4 字节 (8 字节: 包含模块数量 0)
        data.extend_from_slice(&8u32.to_le_bytes());
        // 3. 模块数量 0
        data.extend_from_slice(&0u32.to_le_bytes());

        // 现在处于 base_offset (28 + 8 = 36)
        // 构造一个 PushString 指令 (0x0003), 相对偏移指向后续字符串
        let str_rel_offset = 8u32; // 处于 base_offset + 8
        data.extend_from_slice(&3u32.to_le_bytes()); // PushString opcode
        data.extend_from_slice(&str_rel_offset.to_le_bytes()); // 相对偏移

        // 写入 GBK 编码的中文字符串
        let (gbk_bytes, _, _) = encoding_rs::GBK.encode("纯Rust跨平台Galgame引擎测试");
        data.extend_from_slice(&gbk_bytes);
        data.push(0); // null terminator

        let script = BgiScript::parse(&data).expect("解析合成剧本失败");
        assert_eq!(script.base_offset(), 36);

        // 验证读取指令
        let instr = script.read_instruction(36).expect("读取指令失败");
        assert_eq!(instr.opcode, OpCode::PUSH_STRING);
        assert_eq!(instr.operand, Some(8));
        assert_eq!(instr.byte_size, 8);

        // 验证字符串解码
        let decoded = script.get_string(8).expect("解码 GBK 字符串失败");
        assert_eq!(decoded, "纯Rust跨平台Galgame引擎测试");
    }

    #[test]
    fn test_too_short_data() {
        let short = b"BurikoCompiled";
        let res = BgiScript::parse(short);
        assert!(res.is_err());
        match res.unwrap_err() {
            CoreError::OutOfBounds => {}
            other => panic!("预期 OutOfBounds，得到 {:?}", other),
        }
    }

    #[test]
    fn test_out_of_bounds_string() {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let script = BgiScript::parse(&data).expect("解析失败");
        let res = script.get_string(99999);
        assert!(res.is_err());
        match res.unwrap_err() {
            CoreError::OutOfBounds => {}
            other => panic!("预期 OutOfBounds，得到 {:?}", other),
        }
    }

    #[test]
    fn test_push_string_truncated_operand() {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        // 仅写入 4 字节的 PushString (0x0003)，没有后随的 4 字节相对偏移
        data.extend_from_slice(&3u32.to_le_bytes());

        let script = BgiScript::parse(&data).expect("解析失败");
        let res = script.read_instruction(36);
        assert!(res.is_err());
        match res.unwrap_err() {
            CoreError::OutOfBounds => {}
            other => panic!("预期 OutOfBounds，得到 {:?}", other),
        }
    }

    #[test]
    fn test_8byte_and_4byte_instruction_lengths() {
        let mut data = Vec::new();
        data.extend_from_slice(BGI_SCRIPT_MAGIC);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        // 8 字节全集: {0x0000, 0x0001, 0x0002, 0x0003, 0x0009, 0x0017, 0x0019}
        for op in [0x0000u32, 0x0001, 0x0002, 0x0003, 0x0009, 0x0017, 0x0019] {
            data.extend_from_slice(&op.to_le_bytes());
            data.extend_from_slice(&42u32.to_le_bytes());
        }
        // 4 字节指令 (包含 0x0010, 0x0011, 0x0018, 0x0021, 0x0030, 0x0033, 0x0038, 0x001b, 0x00f4)
        for op in [0x0010u32, 0x0011, 0x0018, 0x0021, 0x0030, 0x0033, 0x0038, 0x001b, 0x00f4] {
            data.extend_from_slice(&op.to_le_bytes());
        }

        let script = BgiScript::parse(&data).expect("解析失败");
        let mut pc = script.base_offset();
        for expected_op in [0x0000u32, 0x0001, 0x0002, 0x0003, 0x0009, 0x0017, 0x0019] {
            let insn = script.read_instruction(pc).expect("读取 8B 指令失败");
            assert_eq!(insn.opcode.0, expected_op);
            assert_eq!(insn.operand, Some(42));
            assert_eq!(insn.byte_size, 8);
            pc += insn.byte_size;
        }
        for expected_op in [0x0010u32, 0x0011, 0x0018, 0x0021, 0x0030, 0x0033, 0x0038, 0x001b, 0x00f4] {
            let insn = script.read_instruction(pc).expect("读取 4B 指令失败");
            assert_eq!(insn.opcode.0, expected_op);
            assert_eq!(insn.operand, None);
            assert_eq!(insn.byte_size, 4);
            pc += insn.byte_size;
        }
    }
}
