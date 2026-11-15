//! 文件编码探测与写回的测试。

use crate::encoding::{self, FileEncoding};

#[test]
fn plain_utf8_is_detected_without_bom() {
    let decoded = encoding::decode("Dim x As Long".as_bytes());
    assert_eq!(decoded.encoding, FileEncoding::Utf8);
    assert_eq!(decoded.text, "Dim x As Long");
}

#[test]
fn utf8_bom_is_removed_and_restored_on_save() {
    let bytes = encoding::encode("中文脚本", FileEncoding::Utf8Bom);
    assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);

    let decoded = encoding::decode(&bytes);
    assert_eq!(decoded.encoding, FileEncoding::Utf8Bom);
    assert_eq!(decoded.text, "中文脚本");
    assert_eq!(encoding::encode(&decoded.text, decoded.encoding), bytes);
}

#[test]
fn utf16_variants_round_trip() {
    for kind in [FileEncoding::Utf16Le, FileEncoding::Utf16Be] {
        let bytes = encoding::encode("Dim 名称 As String", kind);
        let decoded = encoding::decode(&bytes);
        assert_eq!(decoded.encoding, kind);
        assert_eq!(decoded.text, "Dim 名称 As String");
        assert_eq!(encoding::encode(&decoded.text, decoded.encoding), bytes);
    }
}

#[test]
fn gbk_bytes_are_decoded_as_chinese() {
    // 「中文」的 GBK 字节是 D6 D0 CE C4（不是合法 UTF-8）。
    let bytes = [0xD6, 0xD0, 0xCE, 0xC4];
    let decoded = encoding::decode(&bytes);
    assert_eq!(decoded.encoding, FileEncoding::Gbk);
    assert_eq!(decoded.text, "中文");
    assert_eq!(encoding::encode(&decoded.text, FileEncoding::Gbk), bytes);
}

#[test]
fn gbk_source_file_round_trips() {
    let source = "MsgBox \"你好\"";
    let bytes = encoding::encode(source, FileEncoding::Gbk);
    let decoded = encoding::decode(&bytes);
    assert_eq!(decoded.encoding, FileEncoding::Gbk);
    assert_eq!(decoded.text, source);
    assert_eq!(encoding::encode(&decoded.text, decoded.encoding), bytes);
}

#[test]
fn encoding_labels_are_readable() {
    assert_eq!(FileEncoding::Utf8.label(), "UTF-8");
    assert_eq!(FileEncoding::Utf8Bom.label(), "UTF-8 BOM");
    assert_eq!(FileEncoding::Utf16Le.label(), "UTF-16 LE");
    assert_eq!(FileEncoding::Utf16Be.label(), "UTF-16 BE");
    assert_eq!(FileEncoding::Gbk.label(), "GBK");
}

#[test]
fn decode_with_forces_the_requested_encoding() {
    // 同一段中文：GBK 字节按 GBK 读回来，按 UTF-8 读只会得到替换字符。
    let gbk = encoding::encode("你好", FileEncoding::Gbk);
    assert_eq!(encoding::decode_with(&gbk, FileEncoding::Gbk), "你好");
    assert_ne!(encoding::decode_with(&gbk, FileEncoding::Utf8), "你好");

    // 每种编码都能按自己的方式往返。
    for kind in FileEncoding::ALL {
        let bytes = encoding::encode("Dim 名称 As String", *kind);
        assert_eq!(
            encoding::decode_with(&bytes, *kind),
            "Dim 名称 As String",
            "{kind:?}"
        );
    }
}

#[test]
fn decode_with_strips_a_matching_bom() {
    let utf8_bom = encoding::encode("中文", FileEncoding::Utf8Bom);
    assert_eq!(encoding::decode_with(&utf8_bom, FileEncoding::Utf8), "中文");
    assert_eq!(
        encoding::decode_with(&utf8_bom, FileEncoding::Utf8Bom),
        "中文"
    );

    for kind in [FileEncoding::Utf16Le, FileEncoding::Utf16Be] {
        let bytes = encoding::encode("中文", kind);
        assert_eq!(encoding::decode_with(&bytes, kind), "中文", "{kind:?}");
    }
}

#[test]
fn newlines_can_be_normalized() {
    use crate::encoding::LineEnding;

    let mixed = "a\r\nb\nc\rd";
    assert_eq!(encoding::normalize_newlines(mixed, LineEnding::Keep), mixed);
    assert_eq!(
        encoding::normalize_newlines(mixed, LineEnding::Crlf),
        "a\r\nb\r\nc\r\nd"
    );
    assert_eq!(
        encoding::normalize_newlines(mixed, LineEnding::Lf),
        "a\nb\nc\nd"
    );
    // 没有结尾换行时不能凭空补一个。
    assert_eq!(
        encoding::normalize_newlines("a\nb", LineEnding::Crlf),
        "a\r\nb"
    );
    assert_eq!(encoding::normalize_newlines("", LineEnding::Crlf), "");
}

#[test]
fn line_ending_labels_are_readable() {
    use crate::encoding::LineEnding;

    assert_eq!(LineEnding::Keep.label(), "保持原样");
    assert_eq!(LineEnding::Crlf.label(), "CRLF（Windows）");
    assert_eq!(LineEnding::Lf.label(), "LF（Unix）");
    assert_eq!(LineEnding::ALL.len(), 3);
}
