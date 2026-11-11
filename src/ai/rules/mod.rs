//! Q-IDE 内置 AI 的固定规则。
//!
//! 规则放在程序目录下的 [`DIR_NAME`] 文件夹里，按当前文件语言选择：
//! `common.md` 总是加载，然后加载 `<语言>.md`（没有就 `default.md`）。
//! 首次启动会写出内置默认值；文件缺失时也会退回 [`DEFAULT_*`] 常量，
//! 所以删除整个文件夹也不会影响 AI 正常工作。

use std::path::{Path, PathBuf};

/// 程序目录下的规则文件夹名。
pub const DIR_NAME: &str = "ai-rules";

const COMMON: &str = include_str!("common.md");
const VB: &str = include_str!("vb.md");
const DEFAULT: &str = include_str!("default.md");
const README: &str = include_str!("README.md");

/// 规则文件夹的位置：优先用环境变量 `QIDE_AI_RULES_DIR`，否则是 exe 同级目录。
pub fn rules_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("QIDE_AI_RULES_DIR")
        && !dir.trim().is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|dir| dir.join(DIR_NAME))
}

/// 程序启动时调用：创建规则文件夹，并把缺失的默认规则写进去。
/// 已存在的文件不会覆盖；失败（例如程序目录只读）时静默忽略。
pub fn ensure_defaults() {
    if let Some(dir) = rules_dir() {
        let _ = ensure_defaults_in(&dir);
    }
}

pub(crate) fn ensure_defaults_in(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, content) in [
        ("README.md", README),
        ("common.md", COMMON),
        ("vb.md", VB),
        ("default.md", DEFAULT),
    ] {
        let path = dir.join(name);
        if !path.exists() {
            std::fs::write(path, content)?;
        }
    }
    Ok(())
}

/// 当前语言对应的固定规则：`common.md` + 语言文件（或 `default.md`）。
///
/// `language` 直接来自编辑器识别出的语言名（`vb` / `rust` / `json` / `text` …）。
pub fn rules_for_language(language: Option<&str>) -> String {
    let dir = rules_dir();
    rules_for_language_in(dir.as_deref(), language)
}

pub(crate) fn rules_for_language_in(dir: Option<&Path>, language: Option<&str>) -> String {
    let common = dir
        .and_then(|dir| read_rule(dir, "common.md"))
        .unwrap_or_else(|| COMMON.to_string());
    let language = language
        .map(str::to_ascii_lowercase)
        .filter(|lang| is_safe_language(lang));
    let specific = match language.as_deref() {
        // VB 的内置规则始终可用；即使外部 vb.md 被删掉也不会误用 default.md。
        Some("vb") => dir
            .and_then(|dir| read_rule(dir, "vb.md"))
            .unwrap_or_else(|| VB.to_string()),
        Some(lang) => dir
            .and_then(|dir| read_rule(dir, &format!("{lang}.md")))
            .or_else(|| dir.and_then(|dir| read_rule(dir, "default.md")))
            .unwrap_or_else(|| DEFAULT.to_string()),
        None => dir
            .and_then(|dir| read_rule(dir, "default.md"))
            .unwrap_or_else(|| DEFAULT.to_string()),
    };
    format!("{}\n\n{}", common.trim_end(), specific.trim_end())
}

fn read_rule(dir: &Path, name: &str) -> Option<String> {
    let bytes = std::fs::read(dir.join(name)).ok()?;
    // 文件存在但内容为空时也算「用户关掉了这份规则」，不要退回内置默认值。
    Some(crate::encoding::decode(&bytes).text)
}

/// 语言名会拼进文件名，只允许字母数字、`_` 和 `-`，避免路径穿越。
fn is_safe_language(language: &str) -> bool {
    !language.is_empty()
        && language.len() <= 32
        && language
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}
