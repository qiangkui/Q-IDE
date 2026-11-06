//! 编辑器里的 VB 脚本高亮。
//!
//! gpui-component 的高亮走 tree-sitter 语法表，而 VB 脚本是「VB6 风格 + 自定义子集」，
//! 没有合适的语法可用，因此这里给编辑器装一个自己的 [`InputHighlighter`]：
//! - `vb`：用解释器同一套关键字/类型/内置函数表做词法着色，和脚本语义完全一致；
//! - 其它语言：包装 `SyntaxHighlighter`，行为与默认实现保持一致。

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::base::input::{
    EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter,
    InputHighlighterFactory, Rope,
};
use gpui_kit::component::highlighter::{LanguageRegistry, SyntaxHighlighter};
use gpui_kit::{Context, HighlightStyle, SharedString, Window};

use crate::interpreter::builtins;
use crate::interpreter::lexer;
use crate::interpreter::value::VbType;

/// 脚本文件使用的语言名。
pub const VB_LANGUAGE: &str = "vb";

/// 创建编辑器的语言高亮工厂。
pub fn highlighter_factory() -> InputHighlighterFactory {
    Rc::new(|language: &str| -> Option<Box<dyn InputHighlighter>> {
        if language.eq_ignore_ascii_case(VB_LANGUAGE) {
            return Some(Box::new(VbHighlighter::default()));
        }
        let has_grammar = LanguageRegistry::singleton()
            .language(language)
            .map(|config| config.has_grammar())
            .unwrap_or(false);
        has_grammar
            .then(|| Box::new(TreeSitterHighlighter::new(language)) as Box<dyn InputHighlighter>)
    })
}

// ---------------------------------------------------------------- VB 高亮

/// 用一个 span 表示一段有语义颜色的字节区间。
#[derive(Default)]
struct VbHighlighter {
    spans: Vec<(Range<usize>, &'static str)>,
}

impl InputHighlighter for VbHighlighter {
    fn language(&self) -> SharedString {
        VB_LANGUAGE.into()
    }

    fn update(
        &mut self,
        _edit: Option<InputEdit>,
        text: &Rope,
        _folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        let source: String = text.chars().collect();
        self.spans = scan(&source);
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut styles = Vec::new();
        let mut cursor = range.start;
        for (span, name) in &self.spans {
            if span.end <= range.start {
                continue;
            }
            if span.start >= range.end {
                break;
            }
            let start = span.start.max(range.start);
            let end = span.end.min(range.end);
            if start > cursor {
                styles.push((cursor..start, HighlightStyle::default()));
            }
            let style = resolver.style(name).unwrap_or_default();
            styles.push((start..end, style));
            cursor = cursor.max(end);
        }
        if cursor < range.end {
            styles.push((cursor..range.end, HighlightStyle::default()));
        }
        styles
    }

    fn fold_ranges(&self, _text: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

/// 把源码扫成 `(字节区间, 语义名)` 列表；空白不产生 span。
fn scan(source: &str) -> Vec<(Range<usize>, &'static str)> {
    let bytes = source.as_bytes();
    let length = bytes.len();
    let mut spans = Vec::new();
    let mut index = 0usize;
    let mut at_line_start = true;

    while index < length {
        let rest = &source[index..];
        let ch = rest.chars().next().expect("index 在范围内");

        match ch {
            '\'' => {
                let end = rest.find('\n').map_or(length, |offset| index + offset);
                spans.push((index..end, "comment"));
                index = end;
            }
            '"' => {
                let start = index;
                let mut cursor = index + 1;
                while cursor < length {
                    let byte = bytes[cursor];
                    if byte == b'"' {
                        if bytes.get(cursor + 1) == Some(&b'"') {
                            cursor += 2;
                            continue;
                        }
                        cursor += 1;
                        break;
                    }
                    cursor += 1;
                }
                let end = cursor.min(length);
                spans.push((start..end, "string"));
                index = end;
                at_line_start = false;
            }
            c if c.is_ascii_digit()
                || (c == '&' && matches!(bytes.get(index + 1), Some(b'h' | b'H' | b'o' | b'O'))) =>
            {
                let start = index;
                if c == '&' {
                    index += 2;
                }
                while index < length {
                    let next = bytes[index];
                    if next.is_ascii_alphanumeric() || next == b'.' {
                        index += 1;
                    } else {
                        break;
                    }
                }
                spans.push((start..index, "number"));
                at_line_start = false;
            }
            c if c.is_alphanumeric() || c == '_' => {
                let start = index;
                while index < length {
                    let next = source[index..].chars().next().expect("index 在范围内");
                    if next.is_alphanumeric() || next == '_' {
                        index += next.len_utf8();
                    } else {
                        break;
                    }
                }
                let word = &source[start..index];
                let lower = word.to_ascii_lowercase();
                if lower == "rem" && at_line_start {
                    let end = source[index..]
                        .find('\n')
                        .map_or(length, |offset| index + offset);
                    spans.push((start..end, "comment"));
                    index = end;
                    continue;
                }
                let name = if lexer::is_keyword(&lower) {
                    "keyword"
                } else if VbType::parse(&lower).is_some() {
                    "type"
                } else if builtins::is_builtin(&lower) {
                    "function"
                } else {
                    "variable"
                };
                spans.push((start..index, name));
                at_line_start = false;
            }
            ' ' | '\t' | '\r' => {
                index += ch.len_utf8();
            }
            '\n' => {
                index += 1;
                at_line_start = true;
            }
            c => {
                let start = index;
                index += c.len_utf8();
                let name = if matches!(c, '(' | ')' | ',' | '.') {
                    "punctuation"
                } else {
                    "operator"
                };
                spans.push((start..index, name));
                at_line_start = false;
            }
        }
    }

    spans
}

// ---------------------------------------------------- 非 VB 语言：沿用树语法

struct TreeSitterHighlighter {
    inner: SyntaxHighlighter,
}

impl TreeSitterHighlighter {
    fn new(language: &str) -> Self {
        Self {
            inner: SyntaxHighlighter::new(language),
        }
    }
}

impl InputHighlighter for TreeSitterHighlighter {
    fn language(&self) -> SharedString {
        self.inner.language().clone()
    }

    fn update(
        &mut self,
        _edit: Option<InputEdit>,
        text: &Rope,
        _folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        self.inner.update(None, text, None);
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        self.inner.styles(range, resolver)
    }

    fn fold_ranges(&self, _text: &Rope) -> Vec<FoldRange> {
        fold_ranges_from_tree(self.inner.tree())
    }
}

/// 与默认高亮器一致：把跨 2 行以上的节点作为可折叠区域。
fn fold_ranges_from_tree(tree: Option<&tree_sitter::Tree>) -> Vec<FoldRange> {
    fn collect(node: tree_sitter::Node<'_>, ranges: &mut Vec<FoldRange>) {
        let start = node.start_position().row;
        let end = node.end_position().row;
        if end.saturating_sub(start) >= 2 {
            ranges.push(FoldRange::new(start, end));
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect(child, ranges);
        }
    }

    let Some(tree) = tree else {
        return Vec::new();
    };
    let mut ranges = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        collect(child, &mut ranges);
    }
    ranges.sort_by_key(|range| range.start_line);
    ranges.dedup_by_key(|range| range.start_line);
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: &str) -> Vec<&'static str> {
        scan(source).into_iter().map(|(_, name)| name).collect()
    }

    #[test]
    fn scans_vb_script_tokens() {
        let source = "' 注释\nDim total As Long\ntotal = Mid(\"abc\", 1, 2) & 0\nDebug.Print total\n";
        let kinds = names(source);
        assert!(kinds.contains(&"comment"));
        assert!(kinds.contains(&"string"));
        assert!(kinds.contains(&"number"));
        assert!(kinds.contains(&"operator"));
        // Dim / As / Debug / Print 都是关键字。
        assert!(kinds.iter().filter(|name| **name == "keyword").count() >= 4);
        // Mid 是内置函数。
        assert!(kinds.contains(&"function"));
    }

    #[test]
    fn styles_cover_requested_range() {
        let source = "Dim x As Long\nx = 1\n";
        let highlighter = VbHighlighter {
            spans: scan(source),
        };
        struct Always;
        impl HighlightStyleResolver for Always {
            fn style(&self, _: &str) -> Option<HighlightStyle> {
                Some(HighlightStyle::default())
            }
        }
        let styles = highlighter.styles(&(0..source.len()), &Always);
        let mut cursor = 0usize;
        for (range, _) in &styles {
            assert_eq!(range.start, cursor);
            cursor = range.end;
        }
        assert_eq!(cursor, source.len());
    }
}
