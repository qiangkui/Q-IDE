//! `#Include "文件.vb"`：把常用结构体/常量/声明拆到单独文件。
//!
//! VB6/VBA 没有源码级 include（它们靠工程里的多个模块）；这里做一个解释器扩展：
//! 文件开头写 `#Include "相对路径"`，预处理时把目标文件的文本插进来。
//! 每个文件只包含一次，循环引用会报错。
//!
//! 为了不打断主文件的断点与错误行号，主文件的行号保持 1..N 不变；
//! 被包含文件的行号加上一个很大的基数（[`MAIN_LINE_LIMIT`] 的倍数），
//! 报错时再映射回主文件的 `#Include` 行，并在消息前标明「文件 第 N 行」。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 主文件行号的上限；被包含文件从这个值之后开始编号。
pub const MAIN_LINE_LIMIT: usize = 1_000_000;
/// 最多嵌套多少层 include。
const MAX_INCLUDE_DEPTH: usize = 16;
/// 一次展开最多包含多少个文件，避免逻辑行号无限增长。
const MAX_INCLUDE_FILES: usize = 256;

/// 展开后的源码与行号映射。
#[derive(Clone, Debug, Default)]
pub struct Expanded {
    /// 已经插入被包含文件内容的完整源码。
    pub text: String,
    /// 与 `text` 的物理行一一对应的逻辑行号（1 基）。
    pub logical_lines: Vec<usize>,
    /// 逻辑行号 → 原始文件/行号。
    pub map: SourceMap,
}

/// 一次 include 展开发现的错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncludeError {
    pub message: String,
    /// 主文件里应该报错的行号（通常是 `#Include` 所在行）。
    pub line: usize,
}

impl IncludeError {
    fn new(message: impl Into<String>, line: usize) -> Self {
        Self {
            message: message.into(),
            line,
        }
    }
}

/// 逻辑行号与原始文件行号的对应关系。
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    files: Vec<IncludedFile>,
}

#[derive(Clone, Debug)]
struct IncludedFile {
    /// 错误消息里显示的文件名（按 `#Include` 里写的路径）。
    display: String,
    /// 这个文件的逻辑行号基数。
    base: usize,
    /// 展开后占用的行数。
    line_count: usize,
    /// 顶层主文件里 `#Include` 的行号。
    anchor: usize,
}

impl SourceMap {
    /// 是否包含过任何文件。
    pub fn has_includes(&self) -> bool {
        !self.files.is_empty()
    }

    /// 主文件的行号（< [`MAIN_LINE_LIMIT`]）在编辑器里仍然有效。
    pub fn is_main_line(&self, line: usize) -> bool {
        line < MAIN_LINE_LIMIT
    }

    /// 把逻辑行号映射回主文件行号；被包含文件的消息会加上「文件 第 N 行：」前缀。
    pub fn remap(&self, line: usize, message: &str) -> (usize, String) {
        if line < MAIN_LINE_LIMIT {
            return (line, message.to_string());
        }
        for file in &self.files {
            if line > file.base && line - file.base <= file.line_count {
                return (
                    file.anchor,
                    format!("{} 第 {} 行：{message}", file.display, line - file.base),
                );
            }
        }
        (line, message.to_string())
    }
}

/// 展开 `#Include`。`path` 是主脚本路径，相对路径先按它所在目录解析，
/// 找不到时再查程序目录下的 `assets/`（放程序自带的公共定义）。
pub fn expand(source: &str, path: Option<&Path>) -> Result<Expanded, IncludeError> {
    let assets = bundled_assets_dir();
    expand_with_assets(source, path, assets.as_deref())
}

/// 带显式 assets 目录的展开（测试用；正常入口是 [`expand`]）。
pub(crate) fn expand_with_assets(
    source: &str,
    path: Option<&Path>,
    assets: Option<&Path>,
) -> Result<Expanded, IncludeError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut builder = Builder {
        assets: assets.map(Path::to_path_buf),
        ..Builder::default()
    };
    if let Some(path) = path
        && let Ok(canonical) = std::fs::canonicalize(path)
    {
        builder.stack.push(canonical);
    }
    let dir = path
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    builder.expand_source(
        source,
        &dir,
        FileCtx {
            display: None,
            base: 0,
            anchor: 0,
            depth: 0,
        },
    )?;
    Ok(Expanded {
        text: builder.out.join("\n"),
        logical_lines: builder.logical,
        map: SourceMap {
            files: builder.files,
        },
    })
}

/// exe 同级的 `assets/`：程序自带的公共 include 放这里。
fn bundled_assets_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|dir| dir.join("assets"))
}

/// 解析一个相对 include：先找脚本所在目录，再找程序的 `assets/`。
fn resolve_include(dir: &Path, assets: Option<&Path>, name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    let local = dir.join(path);
    if local.is_file() {
        return Some(local);
    }
    let bundled = assets?.join(path);
    bundled.is_file().then_some(bundled)
}

#[derive(Default)]
struct Builder {
    out: Vec<String>,
    logical: Vec<usize>,
    files: Vec<IncludedFile>,
    /// 已经完整展开过的文件（include-once）。
    seen: HashSet<PathBuf>,
    /// 正在展开的文件链（循环引用检测）。
    stack: Vec<PathBuf>,
    /// exe 同级的 `assets/`：脚本目录找不到 include 时再查这里。
    assets: Option<PathBuf>,
    /// 下一个被包含文件的逻辑行号基数。
    next_base: usize,
}

/// 当前正在展开的文件信息。
struct FileCtx<'a> {
    /// 被包含文件的显示名；主文件为 `None`。
    display: Option<&'a str>,
    /// 逻辑行号基数；主文件为 0。
    base: usize,
    /// 顶层主文件的 `#Include` 行号。
    anchor: usize,
    depth: usize,
}

impl Builder {
    /// 返回这个文件的源码行数（逻辑行号范围是 `base + 1 ..= base + 行数`）。
    fn expand_source(
        &mut self,
        source: &str,
        dir: &Path,
        file: FileCtx<'_>,
    ) -> Result<usize, IncludeError> {
        let display = file.display;
        let lines: Vec<&str> = source.split('\n').collect();
        let source_lines = lines.len();
        for (index, raw_line) in lines.iter().enumerate() {
            let line_no = index + 1;
            let include_line = if display.is_some() {
                file.anchor
            } else {
                line_no
            };
            let suffix = display
                .map(|display| format!("（{display} 第 {line_no} 行）"))
                .unwrap_or_default();

            let names = parse_directive(raw_line)
                .map_err(|message| IncludeError::new(format!("#Include {message}{suffix}"), include_line))?;
            let Some(name) = names else {
                self.out.push((*raw_line).to_string());
                self.logical.push(if display.is_some() {
                    file.base + line_no
                } else {
                    line_no
                });
                continue;
            };

            let target = resolve_include(dir, self.assets.as_deref(), &name).ok_or_else(|| {
                let assets = self
                    .assets
                    .as_deref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "（无）".to_string());
                IncludeError::new(
                    format!(
                        "#Include 找不到文件 \"{name}\"（脚本目录：{}；assets：{assets}）{suffix}",
                        dir.display()
                    ),
                    include_line,
                )
            })?;
            let canonical = std::fs::canonicalize(&target).map_err(|error| {
                IncludeError::new(
                    format!("#Include 无法解析 \"{name}\" 的路径：{error}{suffix}"),
                    include_line,
                )
            })?;
            if self.stack.iter().any(|path| path == &canonical) {
                return Err(IncludeError::new(
                    format!("#Include 循环引用 \"{name}\"{suffix}"),
                    include_line,
                ));
            }
            if self.seen.contains(&canonical) {
                continue;
            }
            if file.depth + 1 > MAX_INCLUDE_DEPTH {
                return Err(IncludeError::new(
                    format!("#Include 嵌套超过 {MAX_INCLUDE_DEPTH} 层{suffix}"),
                    include_line,
                ));
            }
            if self.files.len() >= MAX_INCLUDE_FILES {
                return Err(IncludeError::new(
                    format!("#Include 文件数量超过 {MAX_INCLUDE_FILES} 个{suffix}"),
                    include_line,
                ));
            }

            let bytes = std::fs::read(&target).map_err(|error| {
                IncludeError::new(
                    format!("#Include 读取 \"{name}\" 失败：{error}{suffix}"),
                    include_line,
                )
            })?;
            let text = crate::encoding::decode(&bytes).text;
            let file_base = self.next_base + MAIN_LINE_LIMIT;
            self.next_base = file_base;
            let file_index = self.files.len();
            self.files.push(IncludedFile {
                display: name.clone(),
                base: file_base,
                line_count: 0,
                anchor: include_line,
            });
            let child_dir = target
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| dir.to_path_buf());
            self.stack.push(canonical.clone());
            let line_count = self.expand_source(
                &text,
                &child_dir,
                FileCtx {
                    display: Some(&name),
                    base: file_base,
                    anchor: include_line,
                    depth: file.depth + 1,
                },
            )?;
            self.stack.pop();
            if line_count >= MAIN_LINE_LIMIT {
                return Err(IncludeError::new(
                    format!("#Include \"{name}\" 超过 {MAIN_LINE_LIMIT} 行{suffix}"),
                    include_line,
                ));
            }
            self.files[file_index].line_count = line_count;
            self.seen.insert(canonical);
        }
        Ok(source_lines)
    }
}

/// 解析单独一行 `#Include "路径"`；不是指令时返回 `None`。
fn parse_directive(line: &str) -> Result<Option<String>, String> {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix('#') else {
        return Ok(None);
    };
    let rest = rest.trim_start();
    let mut parts = rest.splitn(2, [' ', '\t']);
    let head = parts.next().unwrap_or_default();
    if !head.eq_ignore_ascii_case("include") {
        return Ok(None);
    }
    let after = parts.next().unwrap_or_default().trim_start();
    if after.is_empty() {
        return Err("后面需要文件名，例如 `#Include \"common.vb\"`".into());
    }
    let chars: Vec<char> = after.chars().collect();
    if chars.first() != Some(&'"') {
        return Err("文件名需要用双引号括起来".into());
    }
    let mut index = 1usize;
    let mut name = String::new();
    let mut closed = false;
    while index < chars.len() {
        match chars[index] {
            '"' => {
                if chars.get(index + 1) == Some(&'"') {
                    name.push('"');
                    index += 2;
                } else {
                    index += 1;
                    closed = true;
                    break;
                }
            }
            ch => {
                name.push(ch);
                index += 1;
            }
        }
    }
    if !closed {
        return Err("文件名缺少结束引号".into());
    }
    let tail: String = chars[index..].iter().collect();
    let tail = tail.trim();
    if !tail.is_empty() && !tail.starts_with('\'') {
        return Err("文件名后面只能跟注释".into());
    }
    if name.is_empty() {
        return Err("文件名不能为空".into());
    }
    Ok(Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_directive_accepts_spacing_and_comments() {
        assert_eq!(
            parse_directive("  #Include \"a.vb\" ' 注释").unwrap(),
            Some("a.vb".to_string())
        );
        assert_eq!(parse_directive("#include\t\"a b.vb\"").unwrap(), Some("a b.vb".to_string()));
        assert_eq!(parse_directive("' #Include \"a.vb\"").unwrap(), None);
        assert_eq!(parse_directive("Debug.Print 1").unwrap(), None);
        assert!(parse_directive("#Include").is_err());
        assert!(parse_directive("#Include a.vb").is_err());
        assert!(parse_directive("#Include \"a.vb").is_err());
        assert!(parse_directive("#Include \"a.vb\" x").is_err());
    }
}
