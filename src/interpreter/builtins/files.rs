//! 文件类内置函数：`Dir`、`FileLen`、`GetAttr`/`SetAttr`、`FileDateTime`、`Kill`。
//!
//! 从 `builtins.rs` 按职责拆出来；`call()` 仍在 `super` 里按名字分派。

use super::dates::local_offset_days;
use super::dates::serial_from_unix;
use super::*;

/// I/O 错误 → VB 错误号：文件不存在 53、路径不存在 76、权限 70、已存在 58，其余 75。
pub(super) fn io_number(error: &std::io::Error) -> i64 {
    match error.raw_os_error() {
        Some(2) => 53,
        Some(3) => 76,
        Some(5) => 70,
        Some(183) => 58,
        _ => match error.kind() {
            std::io::ErrorKind::NotFound => 53,
            std::io::ErrorKind::PermissionDenied => 70,
            std::io::ErrorKind::AlreadyExists => 58,
            _ => 75,
        },
    }
}

/// 把路径拆成 `(目录, 文件名掩码)`。
pub(super) fn split_pattern(pattern: &std::path::Path) -> (std::path::PathBuf, String) {
    let parent = match pattern.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let mask = pattern
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "*".into());
    (parent, mask)
}

/// VB 的通配符匹配：`*` 任意串、`?` 单个字符，大小写不敏感。
pub(super) fn wildcard_match(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some(p);
                resume = n;
                p += 1;
            }
            Some(&ch) if ch == '?' || ch == name[n] => {
                p += 1;
                n += 1;
            }
            _ => {
                let Some(position) = star else {
                    return false;
                };
                p = position + 1;
                resume += 1;
                n = resume;
            }
        }
    }
    while matches!(pattern.get(p), Some('*')) {
        p += 1;
    }
    p == pattern.len()
}

/// 带通配符的路径展开成实际文件列表；没有通配符时原样返回。
pub(super) fn expand_paths(cwd: &std::path::Path, pattern: &str) -> Vec<String> {
    if !pattern.contains(['*', '?']) {
        return vec![resolved(cwd, pattern).display().to_string()];
    }
    let (parent, mask) = split_pattern(&resolved(cwd, pattern));
    let Ok(reader) = std::fs::read_dir(&parent) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = reader
        .flatten()
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| !kind.is_dir())
                .unwrap_or(false)
        })
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| wildcard_match(&mask, name))
        .map(|name| parent.join(name).display().to_string())
        .collect();
    paths.sort();
    paths
}

/// `Dir([路径[, 属性]])`：带路径时重建列表，无参时取下一条。
pub(super) fn dir_value(args: &[Value], ctx: &mut Ctx) -> Result<Value, String> {
    let pattern = match args.first() {
        None | Some(Value::Empty) => None,
        Some(value) => Some(value.to_display()),
    };
    let Some(pattern) = pattern else {
        let Some(state) = ctx.dir.as_mut() else {
            return Err("`Dir` 需要先用路径调用一次".into());
        };
        return Ok(Value::Str(state.next_entry()));
    };
    let (parent, mask) = split_pattern(&resolved(ctx.cwd, &pattern));
    let attributes = args.get(1).map(Value::as_int).transpose()?.unwrap_or(0);
    let include_dirs = attributes & 16 != 0;
    let reader = std::fs::read_dir(&parent).map_err(|error| {
        ctx.failure = Some(io_number(&error));
        format!("`Dir` 无法打开 {}：{error}", parent.display())
    })?;
    let mut entries = Vec::new();
    for entry in reader.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        if is_dir && !include_dirs {
            continue;
        }
        if wildcard_match(&mask, &name) {
            entries.push(name);
        }
    }
    entries.sort();
    let mut state = DirState { entries, index: 0 };
    let first = state.next_entry();
    *ctx.dir = Some(state);
    Ok(Value::Str(first))
}

pub(super) fn file_len_value(args: &[Value], ctx: &mut Ctx) -> Result<Value, String> {
    let path = args.first().map(Value::to_display).unwrap_or_default();
    let target = resolved(ctx.cwd, &path);
    std::fs::metadata(&target)
        .map(|metadata| Value::Int(metadata.len() as i64))
        .map_err(|error| {
            ctx.failure = Some(io_number(&error));
            format!("无法读取 `{path}`：{error}")
        })
}

/// `SetAttr` 允许设置的属性位：`vbReadOnly` / `vbHidden` / `vbSystem` / `vbArchive`。
pub(super) const SET_ATTR_MASK: i64 = 1 | 2 | 4 | 32;

/// `GetAttr(path)`：返回文件的属性位。
///
/// 本机 VB6 实测返回的就是 Win32 属性位（新建文件常带 `0x2000`「未编入索引」，例如 8224），
/// `vbReadOnly` 1 / `vbHidden` 2 / `vbSystem` 4 / `vbDirectory` 16 / `vbArchive` 32 与系统位一一对应。
pub(super) fn get_attr_value(args: &[Value], ctx: &mut Ctx) -> Result<Value, String> {
    let path = args.first().map(Value::to_display).unwrap_or_default();
    let target = resolved(ctx.cwd, &path);
    let metadata = std::fs::metadata(&target).map_err(|error| {
        ctx.failure = Some(io_number(&error));
        format!("`GetAttr` 无法读取 `{path}`：{error}")
    })?;
    Ok(Value::Int(attribute_bits(&metadata) as i64))
}

#[cfg(windows)]
pub(super) fn attribute_bits(metadata: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt as _;
    // `FILE_ATTRIBUTE_NORMAL`（0x80）只在「没有其它属性」时由系统给出，VB6 的 `GetAttr`
    // 把它报成 `vbNormal`（0），所以要清掉这一位（本机 VB6 实测 `SetAttr vbNormal` 后是 0）。
    metadata.file_attributes() & !0x80
}

#[cfg(not(windows))]
pub(super) fn attribute_bits(metadata: &std::fs::Metadata) -> u32 {
    let mut bits = 0;
    if metadata.permissions().readonly() {
        bits |= 1;
    }
    if metadata.is_dir() {
        bits |= 16;
    }
    bits
}

/// `SetAttr path, attributes`：只接受 `vbReadOnly`/`vbHidden`/`vbSystem`/`vbArchive` 的组合，
/// 其它位（`vbDirectory`、`vbVolume` 等）按 VB6 报错误号 5；属性不是数值报 13。
pub(super) fn set_attr_value(args: &[Value], ctx: &mut Ctx) -> Result<(), String> {
    let path = args.first().map(Value::to_display).unwrap_or_default();
    let attributes = match args.get(1).map(Value::as_int) {
        Some(Ok(value)) => value,
        Some(Err(message)) => {
            ctx.failure = Some(13);
            return Err(format!("`SetAttr` 的属性参数无效：{message}"));
        }
        None => 0,
    };
    if attributes & !SET_ATTR_MASK != 0 {
        ctx.failure = Some(5);
        return Err(format!(
            "`SetAttr` 不支持属性位 {attributes}（只能组合 vbReadOnly/vbHidden/vbSystem/vbArchive）"
        ));
    }
    let target = resolved(ctx.cwd, &path);
    set_file_attributes(&target, attributes as u32).map_err(|error| {
        ctx.failure = Some(match error.raw_os_error() {
            Some(2 | 3) => 53,
            Some(5) => 70,
            _ => 5,
        });
        format!("`SetAttr` 无法修改 `{path}` 的属性：{error}")
    })
}

#[cfg(windows)]
pub(super) fn set_file_attributes(path: &std::path::Path, attributes: u32) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;

    unsafe extern "system" {
        fn SetFileAttributesW(name: *const u16, attributes: u32) -> i32;
    }
    // `SetFileAttributesW` 不接受 0：`vbNormal` 要换成 FILE_ATTRIBUTE_NORMAL（0x80）。
    let flags = if attributes == 0 { 0x80 } else { attributes };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if unsafe { SetFileAttributesW(wide.as_ptr(), flags) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn set_file_attributes(path: &std::path::Path, attributes: u32) -> std::io::Result<()> {
    // 其它平台只有只读位可写。
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_readonly(attributes & 1 != 0);
    std::fs::set_permissions(path, permissions)
}

/// `FileDateTime(path)`：本地时区的 VB 日期序列。
pub(super) fn file_datetime_value(args: &[Value], ctx: &mut Ctx) -> Result<Value, String> {
    let path = args.first().map(Value::to_display).unwrap_or_default();
    let target = resolved(ctx.cwd, &path);
    let metadata = std::fs::metadata(&target).map_err(|error| {
        ctx.failure = Some(io_number(&error));
        format!("无法读取 `{path}`：{error}")
    })?;
    let modified = metadata.modified().map_err(|error| {
        ctx.failure = Some(75);
        format!("无法读取 `{path}` 的时间：{error}")
    })?;
    let seconds = modified
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0);
    Ok(Value::Date(serial_from_unix(seconds) + local_offset_days()))
}

/// `Kill`：支持 `*`/`?` 通配符；一个都没匹配到时按 VB 报「找不到文件」。
pub(super) fn kill_value(args: &[Value], ctx: &mut Ctx) -> Result<(), String> {
    let pattern = args.first().map(Value::to_display).unwrap_or_default();
    let paths = expand_paths(ctx.cwd, &pattern);
    if paths.is_empty() {
        ctx.failure = Some(53);
        return Err(format!("`Kill` 找不到文件 `{pattern}`"));
    }
    for path in paths {
        std::fs::remove_file(&path).map_err(|error| {
            ctx.failure = Some(io_number(&error));
            format!("`Kill` 无法删除 `{path}`：{error}")
        })?;
    }
    Ok(())
}
