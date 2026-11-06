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
