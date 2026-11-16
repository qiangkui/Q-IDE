//! VB 常量表：`vbBlack` 这类颜色与 `vbCrLf` 这类字符串常量。
//!
//! 从 `builtins.rs` 按职责拆出来；`call()` 仍在 `super` 里按名字分派。

use super::*;

/// `QBColor` 的 16 个经典颜色（Long，`r + g*256 + b*65536`；本机 VB6 实测 0/1/7/15）。
pub(super) const QB_COLORS: [i64; 16] = [
    0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xC0C0C0, 0x808080,
    0xFF0000, 0x00FF00, 0xFFFF00, 0x0000FF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
];

/// VB 标准常量（VB6 里来自 VBA 库）：名字一律小写，比较不分大小写。
///
/// 只收脚本里常用的那些：布尔/默认值、控制字符、比较方式、日期格式、一周第一天与首周规则、
/// `MsgBox` 的标志与返回值、`VarType` 的类型码。
pub(super) const VB_CONSTANTS: &[(&str, i64)] = &[
    ("vbtrue", -1),
    ("vbfalse", 0),
    ("vbusdefault", -2),
    ("vbobjecterror", -2147221504),
    ("vbbinarycompare", 0),
    ("vbtextcompare", 1),
    ("vbdatabasecompare", 2),
    ("vbgeneraldate", 0),
    ("vblongdate", 1),
    ("vbshortdate", 2),
    ("vblongtime", 3),
    ("vbshorttime", 4),
    ("vbusesystem", 0),
    ("vbsunday", 1),
    ("vbmonday", 2),
    ("vbtuesday", 3),
    ("vbwednesday", 4),
    ("vbthursday", 5),
    ("vbfriday", 6),
    ("vbsaturday", 7),
    ("vbsystemdayofweek", 0),
    ("vbfirstjan1", 1),
    ("vbfirstfourdays", 2),
    ("vbfirstfullweek", 3),
    ("vbokonly", 0),
    ("vbokcancel", 1),
    ("vbabortretryignore", 2),
    ("vbyesnocancel", 3),
    ("vbyesno", 4),
    ("vbretrycancel", 5),
    ("vbcritical", 16),
    ("vbquestion", 32),
    ("vbexclamation", 48),
    ("vbinformation", 64),
    ("vbdefaultbutton1", 0),
    ("vbdefaultbutton2", 256),
    ("vbdefaultbutton3", 512),
    ("vbdefaultbutton4", 768),
    ("vbapplicationmodal", 0),
    ("vbsystemmodal", 4096),
    ("vbok", 1),
    ("vbcancel", 2),
    ("vbabort", 3),
    ("vbretry", 4),
    ("vbignore", 5),
    ("vbyes", 6),
    ("vbno", 7),
    ("vbempty", 0),
    ("vbnull", 1),
    ("vbinteger", 2),
    ("vblong", 3),
    ("vbsingle", 4),
    ("vbdouble", 5),
    ("vbcurrency", 6),
    ("vbdate", 7),
    ("vbstring", 8),
    ("vbobject", 9),
    ("vberror", 10),
    ("vbboolean", 11),
    ("vbvariant", 12),
    ("vbdataobject", 13),
    ("vbdecimal", 14),
    ("vbbyte", 17),
    ("vblonglong", 20),
    ("vbuserdefinedtype", 36),
    ("vbnormal", 0),
    ("vbreadonly", 1),
    ("vbhidden", 2),
    ("vbsystem", 4),
    ("vbvolume", 8),
    ("vbdirectory", 16),
    ("vbarchive", 32),
    ("vbalias", 64),
    ("vbarray", 8192),
    ("vbmethod", 1),
    ("vbget", 2),
    ("vblet", 4),
    ("vbset", 8),
];

/// 字符串型 VB 常量（控制字符）。
pub(super) const VB_STRING_CONSTANTS: &[(&str, &str)] = &[
    ("vbcr", "\r"),
    ("vblf", "\n"),
    ("vbcrlf", "\r\n"),
    ("vbtab", "\t"),
    ("vbnullchar", "\0"),
    ("vbnullstring", ""),
    ("vbback", "\u{8}"),
    ("vbformfeed", "\u{c}"),
    ("vbverticaltab", "\u{b}"),
];

/// 查一个 VB 标准常量（名字不分大小写），没查到时返回 `None`。
pub fn vb_constant(name: &str) -> Option<Value> {
    let key = name.to_ascii_lowercase();
    if let Some((_, number)) = VB_CONSTANTS.iter().find(|(name, _)| *name == key) {
        return Some(Value::Int(*number));
    }
    VB_STRING_CONSTANTS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, text)| Value::Str((*text).to_string()))
}

/// VB 标准常量的类型（本机 VB6 实测：`VarType(vbTrue)` 是 3（Long）、`VarType(vbCrLf)` 是 8）。
pub fn vb_constant_type(name: &str) -> Option<VbType> {
    let key = name.to_ascii_lowercase();
    if VB_STRING_CONSTANTS.iter().any(|(name, _)| *name == key) {
        return Some(VbType::Str);
    }
    VB_CONSTANTS
        .iter()
        .any(|(name, _)| *name == key)
        .then_some(VbType::Long)
}
