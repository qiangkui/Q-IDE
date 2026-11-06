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
