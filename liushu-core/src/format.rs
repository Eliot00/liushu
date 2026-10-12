//! 常驻数据的扁平容器：文件布局就是内存布局。
//!
//! 词库、语言模型这类每次按键都要查、极少重写的数据不该走「序列化 → 反序列化」：
//! 文件 = [`HEADER_LEN`] 字节头 + 分节表（4 字节标签 + 偏移 + 长度）+ 各分节正文
//! （按 [`SECTION_ALIGN`] 对齐），正文在内存里是什么布局，落盘就是什么布局。
//! 打开一个容器只是 `mmap` 再校验一遍头与分节边界，随后 [`Table`] / [`Text`] 直接
//! 借用映射里的字节，没有分配、没有拷贝。
//!
//! 数值一律小端、原生对齐（表里的结构体字段按本机字节序解释），所以大端机器上
//! 拒绝编译。数据文件只整体替换（写临时文件再改名），从不就地修改，这是映射内容
//! 在生命周期内不变的依据。
//!
//! ```text
//! 0        8      12     16        20                    32
//! +--------+------+------+---------+---------------------+
//! | MAGIC  | ver  | kind | n_sects | reserved            |   头 32 字节
//! +--------+------+------+---------+---------------------+
//! | tag[4] | rsv  | offset u64    | len u64             |   每节 24 字节
//! +--------+------+---------------+---------------------+
//! | 正文（每节起点 8 字节对齐，节内不放填充）                |
//! ```

#[cfg(target_endian = "big")]
compile_error!("liushu-core 的容器是小端布局 + 零拷贝读取，不支持大端机器");

use std::fs::File;
use std::io::{BufWriter, Write};
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

use memmap2::Mmap;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::error::LiushuError;

/// 文件魔数。
pub const MAGIC: [u8; 8] = *b"LIUSHU\x00\x00";

/// 容器格式版本。布局改动必须加一，旧文件随即失效。
pub const FORMAT_VERSION: u32 = 1;

/// 头长度。
pub const HEADER_LEN: usize = 32;

/// 分节表每项长度（标签 4 + 保留 4 + 偏移 8 + 长度 8）。
pub const SECTION_ENTRY_LEN: usize = 24;

/// 分节正文的对齐：够让 `u64` / `f64` 字段的结构体零拷贝读取。
pub const SECTION_ALIGN: usize = 8;

/// 向上取到 [`SECTION_ALIGN`] 的整数倍。
pub fn align_up(value: usize) -> usize {
    value.div_ceil(SECTION_ALIGN) * SECTION_ALIGN
}

/// 一个待写入的分节。
pub struct Section<'a> {
    /// 4 字节标签。
    pub tag: [u8; 4],

    /// 正文。
    pub bytes: &'a [u8],
}

/// 把若干分节写成一个容器文件。先写同目录的 `.tmp` 再改名，中途失败不会留下半截文件。
pub fn write(path: &Path, kind: [u8; 4], sections: &[Section<'_>]) -> Result<(), LiushuError> {
    let count = u32::try_from(sections.len())
        .map_err(|_| LiushuError::Other("too many sections".to_owned()))?;

    // 先算出每节的偏移，再按同样的顺序写；两遍算出的必须一致。
    let table_len = sections.len() * SECTION_ENTRY_LEN;
    let body_start = align_up(HEADER_LEN + table_len);
    let mut offsets = Vec::with_capacity(sections.len());
    let mut offset = body_start;
    for section in sections {
        offsets.push(offset);
        offset = align_up(offset + section.bytes.len());
    }

    let mut header = [0u8; HEADER_LEN];
    header[..MAGIC.len()].copy_from_slice(&MAGIC);
    header[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&kind);
    header[16..20].copy_from_slice(&count.to_le_bytes());

    let tmp = path.with_extension("tmp");
    let mut writer = BufWriter::new(File::create(&tmp)?);
    writer.write_all(&header)?;
    for (section, &offset) in sections.iter().zip(&offsets) {
        let len = u64::try_from(section.bytes.len())
            .map_err(|_| LiushuError::Other("section too large".to_owned()))?;
        let mut entry = [0u8; SECTION_ENTRY_LEN];
        entry[..4].copy_from_slice(&section.tag);
        entry[8..16].copy_from_slice(&(offset as u64).to_le_bytes());
        entry[16..24].copy_from_slice(&len.to_le_bytes());
        writer.write_all(&entry)?;
    }
    // 头 + 分节表到第一节正文之间的填充。
    writer.write_all(&vec![0u8; body_start - HEADER_LEN - table_len])?;
    for (section, &offset) in sections.iter().zip(&offsets) {
        writer.write_all(section.bytes)?;
        let written = offset + section.bytes.len();
        let pad = align_up(written) - written;
        writer.write_all(&vec![0u8; pad])?;
    }
    // 显式 flush：`BufWriter` 在 drop 时吞掉错误。
    writer.flush()?;
    drop(writer);

    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 所有分节共享的映射。
type Shared = Arc<Mmap>;

/// 一段定长结构体数组：构造时是自己的 `Vec`，从文件打开时是映射里的一段，对外都是 `&[T]`。
#[derive(Debug)]
pub enum Table<T> {
    /// 内存里自己的。
    Owned(Vec<T>),

    /// 映射文件里的一段，打开容器时已校验对齐与长度整除。
    Mapped {
        /// 整个文件的映射，多个分节共享。
        map: Shared,

        /// 正文起始偏移。
        offset: usize,

        /// 正文字节数。
        len: usize,
    },
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self::Owned(Vec::new())
    }
}

impl<T: FromBytes + Immutable + KnownLayout> Table<T> {
    /// 从映射里取一段，校验对齐与长度整除。
    fn mapped(map: Shared, offset: usize, len: usize) -> Result<Self, LiushuError> {
        let bytes = map.get(offset..offset + len).ok_or_else(|| {
            LiushuError::Other("section runs past the end of the file".to_owned())
        })?;
        <[T]>::ref_from_bytes(bytes).map_err(|_| {
            LiushuError::Other("section is misaligned or not a multiple of T".to_owned())
        })?;
        Ok(Self::Mapped { map, offset, len })
    }
}

impl<T: FromBytes + Immutable + KnownLayout> Deref for Table<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        match self {
            Self::Owned(items) => items,
            Self::Mapped { map, offset, len } => {
                <[T]>::ref_from_bytes(&map[*offset..*offset + *len])
                    .expect("mapped table was validated when the container was opened")
            }
        }
    }
}

impl<T: FromBytes + IntoBytes + Immutable + KnownLayout> Table<T> {
    /// 落盘用的字节形式。
    pub fn as_bytes(&self) -> &[u8] {
        let items: &[T] = self;
        items.as_bytes()
    }
}

/// 一段 UTF-8 文本 arena：构造时是自己的 `String`，从文件打开时是映射里的一段，对外都是 `&str`。
#[derive(Debug)]
pub enum Text {
    /// 内存里自己的。
    Owned(String),

    /// 映射文件里的一段，打开容器时已校验 UTF-8。
    Mapped {
        /// 整个文件的映射，多个分节共享。
        map: Shared,

        /// 正文起始偏移。
        offset: usize,

        /// 正文字节数。
        len: usize,
    },
}

impl Default for Text {
    fn default() -> Self {
        Self::Owned(String::new())
    }
}

impl Text {
    /// 从映射里取一段并校验 UTF-8。
    fn mapped(map: Shared, offset: usize, len: usize) -> Result<Self, LiushuError> {
        let bytes = map.get(offset..offset + len).ok_or_else(|| {
            LiushuError::Other("section runs past the end of the file".to_owned())
        })?;
        std::str::from_utf8(bytes)
            .map_err(|_| LiushuError::Other("text section is not UTF-8".to_owned()))?;
        Ok(Self::Mapped { map, offset, len })
    }
}

impl Deref for Text {
    type Target = str;

    fn deref(&self) -> &str {
        match self {
            Self::Owned(text) => text,
            Self::Mapped { map, offset, len } => {
                let bytes = &map[*offset..*offset + *len];
                // SAFETY: `Text::mapped` 构造时校验过这段字节是 UTF-8；数据文件只整体替换、
                // 从不就地修改，映射内容在生命周期内不变。这里不能再用带校验的版本：
                // 每次取 `&str` 都扫一遍几十 MB 的 arena 会让每次按键都重扫一遍。
                unsafe { std::str::from_utf8_unchecked(bytes) }
            }
        }
    }
}

/// 打开的容器：持有映射与分节表。
#[derive(Debug)]
pub struct Container {
    map: Shared,
    sections: Vec<([u8; 4], usize, usize)>,
    kind: [u8; 4],
}

impl Container {
    /// 打开并校验一个容器；`kind` 不符（拿词库当模型用）直接报错。
    pub fn open(path: impl AsRef<Path>, kind: [u8; 4]) -> Result<Self, LiushuError> {
        let file = File::open(path.as_ref())?;
        // SAFETY: 数据文件只整体替换、从不就地修改，映射期间内容不变。
        let map = Arc::new(unsafe { Mmap::map(&file)? });
        let header = map
            .get(..HEADER_LEN)
            .ok_or_else(|| LiushuError::Other("file is shorter than a header".to_owned()))?;
        if header[..8] != MAGIC {
            return Err(LiushuError::Other("not a liushu container".to_owned()));
        }
        let version = u32::from_le_bytes(header[8..12].try_into().expect("4 bytes"));
        if version != FORMAT_VERSION {
            return Err(LiushuError::Other(format!(
                "container version {version} is not {FORMAT_VERSION}"
            )));
        }
        if header[12..16] != kind {
            return Err(LiushuError::Other(format!(
                "container holds {:?}, not {:?}",
                String::from_utf8_lossy(&header[12..16]),
                String::from_utf8_lossy(&kind)
            )));
        }
        let count = u32::from_le_bytes(header[16..20].try_into().expect("4 bytes")) as usize;
        let table_end = HEADER_LEN
            .checked_add(count * SECTION_ENTRY_LEN)
            .ok_or_else(|| LiushuError::Other("section table is too large".to_owned()))?;
        let table = map
            .get(HEADER_LEN..table_end)
            .ok_or_else(|| LiushuError::Other("section table runs past the end".to_owned()))?;

        let mut sections = Vec::with_capacity(count);
        for entry in table.as_chunks::<SECTION_ENTRY_LEN>().0 {
            let tag: [u8; 4] = entry[..4].try_into().expect("4 bytes");
            let offset = u64::from_le_bytes(entry[8..16].try_into().expect("8 bytes")) as usize;
            let len = u64::from_le_bytes(entry[16..24].try_into().expect("8 bytes")) as usize;
            let end = offset
                .checked_add(len)
                .ok_or_else(|| LiushuError::Other("section length overflows".to_owned()))?;
            if !offset.is_multiple_of(SECTION_ALIGN) || end > map.len() {
                return Err(LiushuError::Other("section is out of bounds".to_owned()));
            }
            sections.push((tag, offset, len));
        }

        Ok(Self {
            map,
            sections,
            kind,
        })
    }

    /// 数据种类（词库 / 语言模型）。
    pub fn kind(&self) -> [u8; 4] {
        self.kind
    }

    /// 分节正文。
    pub fn section(&self, tag: [u8; 4]) -> Result<&[u8], LiushuError> {
        self.sections
            .iter()
            .find(|(t, _, _)| *t == tag)
            .map(|(_, offset, len)| &self.map[*offset..*offset + *len])
            .ok_or_else(|| LiushuError::Other(format!("missing section {}", tag_str(&tag))))
    }

    /// 定长结构体数组分节。
    pub fn table<T: FromBytes + Immutable + KnownLayout>(
        &self,
        tag: [u8; 4],
    ) -> Result<Table<T>, LiushuError> {
        let (_, offset, len) = self
            .sections
            .iter()
            .find(|(t, _, _)| *t == tag)
            .ok_or_else(|| LiushuError::Other(format!("missing section {}", tag_str(&tag))))?;
        Table::mapped(Arc::clone(&self.map), *offset, *len)
    }

    /// 文本分节。
    pub fn text(&self, tag: [u8; 4]) -> Result<Text, LiushuError> {
        let (_, offset, len) = self
            .sections
            .iter()
            .find(|(t, _, _)| *t == tag)
            .ok_or_else(|| LiushuError::Other(format!("missing section {}", tag_str(&tag))))?;
        Text::mapped(Arc::clone(&self.map), *offset, *len)
    }
}

/// 分节标签的可读形式（错误信息用）。
pub fn tag_str(tag: &[u8; 4]) -> String {
    tag.iter()
        .map(|&b| if b.is_ascii_graphic() { b as char } else { '?' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
    #[repr(C)]
    struct Row {
        value: u32,
        flag: u16,
        reserved: u16,
    }

    fn row(value: u32, flag: u16) -> Row {
        Row {
            value,
            flag,
            reserved: 0,
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("liushu-format-tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("{name}-{}.bin", std::process::id()))
    }

    #[test]
    fn round_trips_tables_and_text_through_a_file() {
        let path = sample_path("round-trip");
        let rows = [row(7, 1), row(9, 0), row(11, 5)];
        let text = "你好世界";
        write(
            &path,
            *b"DICT",
            &[
                Section {
                    tag: *b"TEXT",
                    bytes: text.as_bytes(),
                },
                Section {
                    tag: *b"ROWS",
                    bytes: rows.as_bytes(),
                },
            ],
        )
        .unwrap();

        let container = Container::open(&path, *b"DICT").unwrap();
        let mapped: Table<Row> = container.table(*b"ROWS").unwrap();
        assert_eq!(&mapped[..], &rows[..]);
        let mapped_text = container.text(*b"TEXT").unwrap();
        assert_eq!(&*mapped_text, text);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn writes_atomically_and_leaves_no_tmp_behind() {
        let path = sample_path("atomic");
        write(
            &path,
            *b"DICT",
            &[Section {
                tag: *b"TEXT",
                bytes: b"x",
            }],
        )
        .unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("tmp").exists());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn rejects_a_foreign_or_truncated_file() {
        let path = sample_path("corrupt");
        std::fs::write(&path, b"not a container at all").unwrap();
        assert!(Container::open(&path, *b"DICT").is_err());

        write(
            &path,
            *b"DICT",
            &[Section {
                tag: *b"TEXT",
                bytes: b"hello",
            }],
        )
        .unwrap();
        // 换个 kind 打开：拿词库当模型用要被拦住。
        assert!(Container::open(&path, *b"LMOD").is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn rejects_a_section_that_runs_past_the_end() {
        let path = sample_path("past-end");
        write(
            &path,
            *b"DICT",
            &[Section {
                tag: *b"TEXT",
                bytes: b"hello",
            }],
        )
        .unwrap();
        // 把分节表里第一节的长度改大，指向文件外。
        let mut bytes = std::fs::read(&path).unwrap();
        let len_at = HEADER_LEN + 16;
        bytes[len_at..len_at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(Container::open(&path, *b"DICT").is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn missing_section_is_an_error_not_a_panic() {
        let path = sample_path("missing-section");
        write(
            &path,
            *b"DICT",
            &[Section {
                tag: *b"TEXT",
                bytes: b"hello",
            }],
        )
        .unwrap();
        let container = Container::open(&path, *b"DICT").unwrap();
        assert!(container.section(*b"NOPE").is_err());
        assert!(container.table::<Row>(*b"NOPE").is_err());
        assert!(container.text(*b"NOPE").is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn empty_sections_are_allowed() {
        let path = sample_path("empty");
        write(
            &path,
            *b"DICT",
            &[
                Section {
                    tag: *b"TEXT",
                    bytes: b"",
                },
                Section {
                    tag: *b"ROWS",
                    bytes: &[],
                },
            ],
        )
        .unwrap();
        let container = Container::open(&path, *b"DICT").unwrap();
        let rows: Table<Row> = container.table(*b"ROWS").unwrap();
        assert!(rows.is_empty());
        assert_eq!(&*container.text(*b"TEXT").unwrap(), "");
        std::fs::remove_file(&path).unwrap();
    }
}
