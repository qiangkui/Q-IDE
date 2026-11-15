//! 文本文件编码：打开时探测，保存时按探测到的编码写回。
//!
//! 探测顺序是 BOM → 严格 UTF-8 → GBK 兜底，覆盖 Windows 上常见的老脚本。
//! 新建文件使用不带 BOM 的 UTF-8。

/// 文件使用的编码。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FileEncoding {
    /// 不带 BOM 的 UTF-8，新建文件的默认值。
    #[default]
    Utf8,
    /// 带 BOM 的 UTF-8。
    Utf8Bom,
    /// 带 BOM 的 UTF-16 小端。
    Utf16Le,
    /// 带 BOM 的 UTF-16 大端。
    Utf16Be,
    /// 中文 Windows 常见的 GBK。
    Gbk,
}

impl FileEncoding {
    /// 界面上的选择顺序（状态栏「切换编码」与另存为对话框共用）。
    pub const ALL: &'static [Self] = &[
        Self::Utf8,
        Self::Utf8Bom,
        Self::Utf16Le,
        Self::Utf16Be,
        Self::Gbk,
    ];

    /// 界面上展示的名字。
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Gbk => "GBK",
        }
    }
}

/// 保存时使用的换行符。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineEnding {
    /// 保持文本里已有的换行（正常保存都用它）。
    #[default]
    Keep,
    /// `\r\n`。
    Crlf,
    /// `\n`。
    Lf,
}

impl LineEnding {
    /// 另存为对话框里的选项顺序。
    pub const ALL: &'static [Self] = &[Self::Keep, Self::Crlf, Self::Lf];

    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => "保持原样",
            Self::Crlf => "CRLF（Windows）",
            Self::Lf => "LF（Unix）",
        }
    }
}

/// 把文本里的换行统一成指定形式；`Keep` 原样返回。
///
/// 先把 `\r\n` 与单独的 `\r` 都归一成 `\n`，再按目标换行符展开，
/// 所以混合换行的文件也能一次改干净。
pub fn normalize_newlines(text: &str, ending: LineEnding) -> String {
    if ending == LineEnding::Keep {
        return text.to_string();
    }
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    match ending {
        LineEnding::Keep => unified,
        LineEnding::Crlf => unified.replace('\n', "\r\n"),
        LineEnding::Lf => unified,
    }
}

/// 解码结果：文本与探测到的编码。
pub struct DecodedText {
    pub text: String,
    pub encoding: FileEncoding,
}

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16_LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16_BE_BOM: &[u8] = &[0xFE, 0xFF];

/// 把文件字节解码成文本，并记下探测到的编码。
pub fn decode(bytes: &[u8]) -> DecodedText {
    if let Some(rest) = bytes.strip_prefix(UTF8_BOM) {
        return DecodedText {
            text: String::from_utf8_lossy(rest).into_owned(),
            encoding: FileEncoding::Utf8Bom,
        };
    }
    if let Some(rest) = bytes.strip_prefix(UTF16_LE_BOM) {
        let (text, ..) = encoding_rs::UTF_16LE.decode(rest);
        return DecodedText {
            text: text.into_owned(),
            encoding: FileEncoding::Utf16Le,
        };
    }
    if let Some(rest) = bytes.strip_prefix(UTF16_BE_BOM) {
        let (text, ..) = encoding_rs::UTF_16BE.decode(rest);
        return DecodedText {
            text: text.into_owned(),
            encoding: FileEncoding::Utf16Be,
        };
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => DecodedText {
            text: text.to_string(),
            encoding: FileEncoding::Utf8,
        },
        // 不是合法 UTF-8：按中文 Windows 的老习惯当成 GBK 读。
        Err(_) => {
            let (text, ..) = encoding_rs::GBK.decode(bytes);
            DecodedText {
                text: text.into_owned(),
                encoding: FileEncoding::Gbk,
            }
        }
    }
}

/// 按用户指定的编码解码（状态栏「切换编码」用）。
///
/// 与 [`decode`] 不同，这里不探测：选了什么就按什么读，`Utf8` / `GBK` 也会先去掉
/// 可能存在的同类 BOM，避免把 BOM 当成正文。
pub fn decode_with(bytes: &[u8], encoding: FileEncoding) -> String {
    match encoding {
        FileEncoding::Utf8 | FileEncoding::Utf8Bom => {
            String::from_utf8_lossy(bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes)).into_owned()
        }
        FileEncoding::Utf16Le => {
            let (text, ..) =
                encoding_rs::UTF_16LE.decode(bytes.strip_prefix(UTF16_LE_BOM).unwrap_or(bytes));
            text.into_owned()
        }
        FileEncoding::Utf16Be => {
            let (text, ..) =
                encoding_rs::UTF_16BE.decode(bytes.strip_prefix(UTF16_BE_BOM).unwrap_or(bytes));
            text.into_owned()
        }
        FileEncoding::Gbk => {
            let (text, ..) = encoding_rs::GBK.decode(bytes);
            text.into_owned()
        }
    }
}

/// 按指定编码把文本写回字节；带 BOM 的编码会补上 BOM。
pub fn encode(text: &str, encoding: FileEncoding) -> Vec<u8> {
    match encoding {
        FileEncoding::Utf8 => text.as_bytes().to_vec(),
        FileEncoding::Utf8Bom => {
            let mut bytes = UTF8_BOM.to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        FileEncoding::Utf16Le => {
            let mut bytes = UTF16_LE_BOM.to_vec();
            bytes.extend(utf16_units(text, true));
            bytes
        }
        FileEncoding::Utf16Be => {
            let mut bytes = UTF16_BE_BOM.to_vec();
            bytes.extend(utf16_units(text, false));
            bytes
        }
        FileEncoding::Gbk => {
            let (bytes, ..) = encoding_rs::GBK.encode(text);
            bytes.into_owned()
        }
    }
}

fn utf16_units(text: &str, little_endian: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&if little_endian {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        });
    }
    bytes
}
