//! `SaveSetting` / `GetSetting` / `GetAllSettings` / `DeleteSetting`（写注册表）。
//!
//! 从 `builtins.rs` 按职责拆出来；`call()` 仍在 `super` 里按名字分派。

use super::*;

/// `SaveSetting` / `GetSetting` / `DeleteSetting` 的存储文件。
///
/// VB6 写注册表 `HKCU\Software\VB and VBA Program Settings`；QIDE 不碰注册表，
/// 写 `%APPDATA%\gpui-first\vb-settings.txt`（每行 `应用 节 键 值`，用制表符分隔、反斜杠转义）。
pub(super) fn settings_path() -> std::path::PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("LOCALAPPDATA"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("gpui-first").join("vb-settings.txt")
}

/// 转义/还原设置文件里的字段（`\`、制表符、换行）。
pub(super) fn escape_setting(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "")
}

pub(super) fn unescape_setting(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('t') => output.push('\t'),
            Some('n') => output.push('\n'),
            Some(other) => output.push(other),
            None => output.push('\\'),
        }
    }
    output
}

/// 设置文件是「读整份 → 改 → 写回整份」，多个线程同时写会互相丢数据（脚本线程与单元测试都会碰它），
/// 所以读改写整段串行化。
pub(super) static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 读改写设置文件的互斥体； poisoned 时也照常使用（数据只是文件，不会内存不安全）。
pub(super) fn lock_settings() -> std::sync::MutexGuard<'static, ()> {
    SETTINGS_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// 读出全部设置项（读不到文件就是空）。
pub(super) fn read_settings() -> Vec<(String, String, String, String)> {
    let Ok(text) = std::fs::read_to_string(settings_path()) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 3 {
                return None;
            }
            Some((
                unescape_setting(parts[0]),
                unescape_setting(parts[1]),
                unescape_setting(parts[2]),
                unescape_setting(parts.get(3).copied().unwrap_or("")),
            ))
        })
        .collect()
}

pub(super) fn write_settings(entries: &[(String, String, String, String)]) {
    let text: String = entries
        .iter()
        .map(|(app, section, key, value)| {
            format!(
                "{}\t{}\t{}\t{}\n",
                escape_setting(app),
                escape_setting(section),
                escape_setting(key),
                escape_setting(value)
            )
        })
        .collect();
    let path = settings_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, text);
}

/// 键名比较不分大小写（与 VB6 的注册表语义一致）。
pub(super) fn same_setting(
    entry: &(String, String, String, String),
    app: &str,
    section: &str,
    key: &str,
) -> bool {
    entry.0.eq_ignore_ascii_case(app)
        && entry.1.eq_ignore_ascii_case(section)
        && entry.2.eq_ignore_ascii_case(key)
}

pub(super) fn get_setting(app: &str, section: &str, key: &str) -> Option<String> {
    read_settings()
        .into_iter()
        .find(|entry| same_setting(entry, app, section, key))
        .map(|entry| entry.3)
}

pub(super) fn save_setting(app: &str, section: &str, key: &str, value: &str) {
    let _guard = lock_settings();
    let mut entries = read_settings();
    match entries
        .iter_mut()
        .find(|entry| same_setting(entry, app, section, key))
    {
        Some(entry) => entry.3 = value.to_string(),
        None => entries.push((
            app.to_string(),
            section.to_string(),
            key.to_string(),
            value.to_string(),
        )),
    }
    write_settings(&entries);
}

/// `LoadPicture` 支持的文件头：BMP / PNG / JPEG / GIF / ICO / WMF / EMF。
///
/// QIDE 不解码图像，只按幻数判断「像不像图片」——VB6 对内容无效的文件报错误号 481。
pub(super) fn looks_like_picture(bytes: &[u8]) -> bool {
    bytes.starts_with(b"BM")
        || bytes.starts_with(b"\x89PNG")
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF8")
        || bytes.starts_with(&[0x00, 0x00, 0x01, 0x00])
        || bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A])
        || bytes.starts_with(&[0x01, 0x00, 0x00, 0x00, 0x6C, 0x00, 0x00, 0x00])
}

/// `GetAllSettings`：某一节里的全部键值对，保持写入顺序。
pub(super) fn settings_in_section(app: &str, section: &str) -> Vec<(String, String)> {
    read_settings()
        .into_iter()
        .filter(|entry| entry.0.eq_ignore_ascii_case(app) && entry.1.eq_ignore_ascii_case(section))
        .map(|entry| (entry.2, entry.3))
        .collect()
}

/// `GetAllSettings` 的返回值：0 基 `(行, 列)` 二维 `String` 数组。
pub(super) fn settings_array(entries: &[(String, String)]) -> VbArray {
    let mut array = VbArray::new(vec![(0, entries.len()), (0, 2)], VbType::Str, true);
    for (row, (key, value)) in entries.iter().enumerate() {
        let row = row as i32;
        let _ = array.set(&[row, 0], Value::Str(key.clone()), None);
        let _ = array.set(&[row, 1], Value::Str(value.clone()), None);
    }
    array
}

/// `DeleteSetting 应用[, 节[, 键]]`：省略越靠后的参数，删除范围越大。
pub(super) fn delete_setting(app: &str, section: Option<&str>, key: Option<&str>) {
    let _guard = lock_settings();
    let mut entries = read_settings();
    entries.retain(|entry| {
        if !entry.0.eq_ignore_ascii_case(app) {
            return true;
        }
        if let Some(section) = section
            && !entry.1.eq_ignore_ascii_case(section)
        {
            return true;
        }
        if let Some(key) = key
            && !entry.2.eq_ignore_ascii_case(key)
        {
            return true;
        }
        false
    });
    write_settings(&entries);
}
