use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("非法的 ARC20 魔数: {0:?}")]
    InvalidArcMagic(Vec<u8>),

    #[error("封包切片越界: 索引表或数据区超出文件大小")]
    OutOfBounds,

    #[error("文件未找到: {0}")]
    FileNotFound(String),

    #[error("数据格式损坏或不支持: {0}")]
    InvalidFormat(&'static str),

    #[error("未知或损坏的音频头: {0:?}")]
    InvalidAudioHeader(Vec<u8>),

    #[error("非法的 BGI 剧本魔数: {0:?}")]
    InvalidScriptMagic(Vec<u8>),

    #[error("VM 虚拟机执行错误: {0}")]
    VmError(String),

    #[error("路径解析失败: {0:?}")]
    PathError(PathBuf),
}

pub type Result<T> = std::result::Result<T, CoreError>;
