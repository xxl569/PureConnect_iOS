use crate::archive::Arc20Archive;
use crate::error::{CoreError, Result};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// 挂载的资产源类型
enum MountSource {
    Directory {
        _root: PathBuf,
        files: HashMap<String, PathBuf>,
    },
    Archive(Arc20Archive),
}

/// 统一虚拟文件系统 (Virtual File System)
/// 支持多层挂载、优先级覆盖与大小写不敏感检索
pub struct VirtualFileSystem {
    sources: Vec<MountSource>,
}

impl VirtualFileSystem {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    /// 挂载一个散落文件目录（高优先级，常用于挂载解包出来的汉化补丁 patch/ 目录）
    pub fn mount_directory<P: AsRef<Path>>(&mut self, dir_path: P) -> Result<()> {
        let root = dir_path.as_ref().to_path_buf();
        if !root.is_dir() {
            return Err(CoreError::PathError(root));
        }

        let mut files = HashMap::new();
        Self::scan_dir_recursive(&root, &root, &mut files)?;

        self.sources.push(MountSource::Directory { _root: root, files });
        Ok(())
    }

    /// 挂载一个 BURIKO ARC20 归档封包
    pub fn mount_archive<P: AsRef<Path>>(&mut self, arc_path: P) -> Result<()> {
        let archive = Arc20Archive::open(arc_path)?;
        self.sources.push(MountSource::Archive(archive));
        Ok(())
    }

    /// 递归扫描目录下的所有文件并建立小写键索引
    fn scan_dir_recursive(
        base: &Path,
        current: &Path,
        map: &mut HashMap<String, PathBuf>,
    ) -> Result<()> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                Self::scan_dir_recursive(base, &path, map)?;
            } else if path.is_file() {
                if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                    map.insert(file_name.to_ascii_lowercase(), path);
                }
            }
        }
        Ok(())
    }

    /// 检查指定文件是否存在（大小写不敏感）
    pub fn contains(&self, name: &str) -> bool {
        let key = name.to_ascii_lowercase();
        for source in &self.sources {
            match source {
                MountSource::Directory { files, .. } => {
                    if files.contains_key(&key) {
                        return true;
                    }
                }
                MountSource::Archive(arc) => {
                    if arc.contains(&key) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// 按优先级读取文件字节流（封包内零拷贝 Borrowed，散落文件为 Owned）
    pub fn read<'a>(&'a self, name: &str) -> Result<Cow<'a, [u8]>> {
        let key = name.to_ascii_lowercase();

        for source in &self.sources {
            match source {
                MountSource::Directory { files, .. } => {
                    if let Some(file_path) = files.get(&key) {
                        let data = fs::read(file_path)?;
                        return Ok(Cow::Owned(data));
                    }
                }
                MountSource::Archive(arc) => {
                    if let Some(slice) = arc.read(&key) {
                        return Ok(Cow::Borrowed(slice));
                    }
                }
            }
        }

        Err(CoreError::FileNotFound(name.to_string()))
    }

    /// 按挂载源优先级依次尝试多个候选文件名（外层按源优先级，内层按候选名），
    /// 确保高优先级目录源（如 exe_unpacked 汉化脚本）绝不会被低优先级 .arc 封包抢先命中
    pub fn read_first_matching<'a>(&'a self, candidates: &[String]) -> Option<Cow<'a, [u8]>> {
        let lower_candidates: Vec<String> = candidates
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .collect();

        for source in &self.sources {
            match source {
                MountSource::Directory { files, .. } => {
                    for key in &lower_candidates {
                        if let Some(file_path) = files.get(key) {
                            if let Ok(data) = fs::read(file_path) {
                                return Some(Cow::Owned(data));
                            }
                        }
                    }
                }
                MountSource::Archive(arc) => {
                    for key in &lower_candidates {
                        if let Some(slice) = arc.read(key) {
                            return Some(Cow::Borrowed(slice));
                        }
                    }
                }
            }
        }
        None
    }

    /// 获取当前挂载源的总数量
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// 统计 VFS 中已索引的总文件数目（去重前）
    pub fn total_indexed_files(&self) -> usize {
        self.sources
            .iter()
            .map(|s| match s {
                MountSource::Directory { files, .. } => files.len(),
                MountSource::Archive(arc) => arc.len(),
            })
            .sum()
    }
}

impl Default for VirtualFileSystem {
    fn default() -> Self {
        Self::new()
    }
}
