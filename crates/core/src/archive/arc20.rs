use crate::error::{CoreError, Result};
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

/// ARC20 单个文件索引条目
#[derive(Debug, Clone)]
pub struct ArcEntry {
    pub name: String,
    pub abs_offset: usize,
    pub size: usize,
}

/// BURIKO ARC20 封包解析器（基于内存映射零拷贝）
pub struct Arc20Archive {
    path: PathBuf,
    mmap: Mmap,
    entries: HashMap<String, ArcEntry>,
}

impl Arc20Archive {
    /// 打开并零拷贝内存映射一个 BURIKO ARC20 封包文件
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let file = File::open(&path_buf)?;
        let mmap = unsafe { Mmap::map(&file)? };

        if mmap.len() < 16 {
            return Err(CoreError::OutOfBounds);
        }

        // 验证魔数前 12 字节
        let magic = &mmap[0..12];
        if !magic.starts_with(b"BURIKO ARC20") {
            return Err(CoreError::InvalidArcMagic(magic.to_vec()));
        }

        // 读取文件总数
        let file_count = u32::from_le_bytes(
            mmap[12..16]
                .try_into()
                .map_err(|_| CoreError::OutOfBounds)?,
        ) as usize;

        let index_end = 16 + file_count * 128;
        if mmap.len() < index_end {
            return Err(CoreError::OutOfBounds);
        }

        let data_base_offset = index_end;
        let mut entries = HashMap::with_capacity(file_count);

        for i in 0..file_count {
            let entry_offset = 16 + i * 128;
            let entry_slice = &mmap[entry_offset..entry_offset + 128];

            // 0x00..0x5F (96 字节): 提取文件名（Null-terminated）
            let name_bytes = &entry_slice[0..96];
            let name_len = name_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(name_bytes.len());
            let raw_name = &name_bytes[..name_len];

            // 优先 Shift-JIS (CP932) 解码，失败回退为 Latin1
            let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.decode(raw_name);
            let name = if had_errors {
                raw_name.iter().map(|&b| b as char).collect::<String>()
            } else {
                cow.into_owned()
            };

            // 0x60..0x63: 相对偏移; 0x64..0x67: 数据大小
            let rel_offset = u32::from_le_bytes(
                entry_slice[96..100]
                    .try_into()
                    .map_err(|_| CoreError::OutOfBounds)?,
            ) as usize;
            let size = u32::from_le_bytes(
                entry_slice[100..104]
                    .try_into()
                    .map_err(|_| CoreError::OutOfBounds)?,
            ) as usize;

            let abs_offset = data_base_offset + rel_offset;
            if abs_offset + size > mmap.len() {
                return Err(CoreError::OutOfBounds);
            }

            let entry = ArcEntry {
                name: name.clone(),
                abs_offset,
                size,
            };

            // 统一存为小写键，实现大小写不敏感匹配
            entries.insert(name.to_ascii_lowercase(), entry);
        }

        Ok(Self {
            path: path_buf,
            mmap,
            entries,
        })
    }

    /// 获取封包内文件总数
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 封包是否为空
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 封包原始物理路径
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 检查是否存在指定文件（大小写不敏感）
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(&name.to_ascii_lowercase())
    }

    /// 零拷贝读取文件数据切片
    pub fn read(&self, name: &str) -> Option<&[u8]> {
        let entry = self.entries.get(&name.to_ascii_lowercase())?;
        Some(&self.mmap[entry.abs_offset..entry.abs_offset + entry.size])
    }

    /// 获取所有文件条目列表迭代器
    pub fn iter_entries(&self) -> impl Iterator<Item = &ArcEntry> {
        self.entries.values()
    }
}
