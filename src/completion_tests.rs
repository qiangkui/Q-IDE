//! VB 补全候选的测试。
//!
//! 与其它模块一致放在独立文件里：写在 `completion.rs` 的 `mod tests` 中会触发
//! rustc 的宏展开递归上限。

use gpui_kit::base::input::Rope;

use crate::completion::{
    is_code, member_candidates, member_context, suggestions, suggestions_for, word_prefix,
};
use crate::interpreter::lexer;

#[test]
fn prefix_filters_keywords_and_builtins() {
    let items = suggestions("msg");
    assert!(items.iter().any(|item| item.label == "msgbox"));
    assert!(
        items
            .iter()
            .all(|item| item.label.to_lowercase().starts_with("msg"))
    );
}

#[test]
fn keywords_use_the_vb_spelling() {
    let items = suggestions("elseif");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, "ElseIf");
    assert_eq!(items[0].detail.as_deref(), Some("关键字"));
}

#[test]
fn data_types_are_suggested_with_vb_spelling() {
    let items = suggestions("lon");
    assert!(
        items
            .iter()
            .any(|item| item.label == "Long" && item.detail.as_deref() == Some("数据类型")),
        "`Dim x As Lon` 应当提示 Long：{items:?}"
    );

    let items = suggestions("boo");
    assert!(items.iter().any(|item| item.label == "Boolean"));

    for name in ["Collection", "Dictionary"] {
        let items = suggestions(&name.to_lowercase());
        assert!(
            items
                .iter()
                .any(|item| item.label == name && item.detail.as_deref() == Some("数据类型")),
            "`As {name}` 应当有候选：{items:?}"
        );
    }
}

#[test]
fn string_is_offered_as_both_type_and_builtin() {
    let items = suggestions("string");
    let matches: Vec<&str> = items
        .iter()
        .filter(|item| item.label.eq_ignore_ascii_case("string"))
        .filter_map(|item| item.detail.as_deref())
        .collect();
    assert!(
        matches.contains(&"数据类型"),
        "String 应当有数据类型候选：{items:?}"
    );
    assert!(
        matches.contains(&"内置函数"),
        "String 的内置函数候选也应当保留：{items:?}"
    );
}

#[test]
fn builtins_are_marked_as_functions() {
    let items = suggestions("left");
    let item = items
        .iter()
        .find(|item| item.label.eq_ignore_ascii_case("left"))
        .expect("应当有 Left 候选");
    assert_eq!(item.detail.as_deref(), Some("内置函数"));
}

#[test]
fn empty_prefix_lists_every_candidate_in_order() {
    let items = suggestions("");
    assert!(items.len() > 100, "关键字与内置函数应全部给出");
    assert!(
        items
            .windows(2)
            .all(|pair| pair[0].label.to_lowercase() <= pair[1].label.to_lowercase()),
        "候选应按字母排序"
    );
}

#[test]
fn unknown_prefix_has_no_candidates() {
    assert!(suggestions("zzz").is_empty());
}

#[test]
fn keyword_table_matches_the_lexer() {
    assert!(!lexer::KEYWORDS.is_empty());
    for keyword in lexer::KEYWORDS {
        assert!(lexer::is_keyword(keyword), "`{keyword}` 应当被词法器识别");
        assert!(
            lexer::keyword_display(keyword).is_some(),
            "`{keyword}` 应当有显示名"
        );
    }
}

#[test]
fn module_variables_are_offered() {
    let source = "Dim total As Long\nSub AddOne(value As Long)\n    Dim result As Long\nEnd Sub\n";
    let items = suggestions_for(source, "to");
    let total = items
        .iter()
        .find(|item| item.label.eq_ignore_ascii_case("total"))
        .expect("应当有 total 候选");
    assert_eq!(total.detail.as_deref(), Some("模块变量"));
}

#[test]
fn local_variables_show_their_procedure() {
    let items = suggestions_for("Sub AddOne()\n    Dim result As Long\nEnd Sub\n", "res");
    let result = items
        .iter()
        .find(|item| item.label == "result")
        .expect("应当有 result 候选");
    let detail = result.detail.as_deref().unwrap_or_default();
    assert!(detail.contains("局部变量"), "{detail}");
    assert!(detail.contains("addone"), "{detail}");
}

#[test]
fn document_names_take_priority_over_builtins() {
    let items = suggestions_for("Dim left As String\n", "lef");
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0].label, "left");
    assert_eq!(items[0].detail.as_deref(), Some("模块变量"));
}

#[test]
fn procedures_and_types_are_offered() {
    let source = "Type Point\n    X As Long\nEnd Type\nSub Fill()\nEnd Sub\n";
    let types = suggestions_for(source, "po");
    assert!(
        types
            .iter()
            .any(|item| item.label.eq_ignore_ascii_case("point")),
        "{types:?}"
    );
    let procedures = suggestions_for(source, "fil");
    assert!(
        procedures
            .iter()
            .any(|item| item.label.eq_ignore_ascii_case("fill")),
        "{procedures:?}"
    );
}

#[test]
fn incomplete_source_still_offers_symbols() {
    let items = suggestions_for("Sub Foo()\n    Dim local As Long\n", "loc");
    assert!(
        items
            .iter()
            .any(|item| item.label.eq_ignore_ascii_case("local")),
        "{items:?}"
    );
}

#[test]
fn word_prefix_stops_at_non_identifier_characters() {
    let rope = Rope::from("Dim total = Lef");
    assert_eq!(word_prefix(&rope, rope.len()), "Lef");
}

#[test]
fn comments_and_strings_have_no_completions() {
    let comment = Rope::from("Dim x ' 注释");
    assert!(!is_code(&comment, comment.len()));

    let string = Rope::from("MsgBox \"abc");
    assert!(!is_code(&string, string.len()));

    let code = Rope::from("Dim x\nLef");
    assert!(is_code(&code, code.len()));
}

const MEMBER_SOURCE: &str = "\
Type Point
    X As Long
    Y As Long
End Type

Class Counter
    Public Step As Long
    Private Items() As Long
    Public Sub Bump()
        Dim inner As Long
    End Sub
    Public Property Get Total() As Long
    End Property
End Class

Sub Main()
    Dim p As Point
    Dim c As New Counter
    Dim list As New Collection
    Dim dict As New Dictionary
    Dim plain As Long
End Sub
";

fn labels(items: &[lsp_types::CompletionItem]) -> Vec<String> {
    items.iter().map(|item| item.label.clone()).collect()
}

#[test]
fn member_context_recognises_dot_access() {
    let rope = Rope::from("    p.");
    assert_eq!(
        member_context(&rope, rope.len()),
        Some(("p".to_string(), String::new()))
    );

    let rope = Rope::from("    p.X");
    assert_eq!(
        member_context(&rope, rope.len()),
        Some(("p".to_string(), "X".to_string()))
    );

    let rope = Rope::from("    p.X + 1");
    assert_eq!(member_context(&rope, rope.len()), None);

    let rope = Rope::from("    .x");
    assert_eq!(member_context(&rope, rope.len()), None);
}

#[test]
fn udt_fields_are_offered() {
    let items = member_candidates(MEMBER_SOURCE, "p", "", 0);
    assert_eq!(labels(&items), vec!["X".to_string(), "Y".to_string()]);
    assert_eq!(items[0].detail.as_deref(), Some("字段 · Long"));
}

#[test]
fn class_members_skip_method_bodies() {
    let items = member_candidates(MEMBER_SOURCE, "c", "", 0);
    let names = labels(&items);
    assert_eq!(
        names,
        vec![
            "Bump".to_string(),
            "Items".to_string(),
            "Step".to_string(),
            "Total".to_string(),
        ]
    );
    assert!(
        !names.iter().any(|name| name == "inner"),
        "方法体里的局部变量不是成员"
    );
}

#[test]
fn member_prefix_filters_candidates() {
    let items = member_candidates(MEMBER_SOURCE, "c", "ste", 0);
    assert_eq!(labels(&items), vec!["Step".to_string()]);
}

#[test]
fn me_resolves_the_enclosing_class() {
    let source = "\
Class Counter
    Public Step As Long
    Public Sub Bump()
        Me.
    End Sub
End Class
";
    let line = source
        .lines()
        .position(|line| line.contains("Me."))
        .expect("测试源码里应当有 Me.")
        + 1;
    let items = member_candidates(source, "me", "", line);
    assert_eq!(labels(&items), vec!["Bump".to_string(), "Step".to_string()]);
}

#[test]
fn new_collection_and_dictionary_offer_methods() {
    let collection = labels(&member_candidates(MEMBER_SOURCE, "list", "", 0));
    assert_eq!(
        collection,
        vec![
            "Add".to_string(),
            "Count".to_string(),
            "Item".to_string(),
            "Remove".to_string(),
        ]
    );

    let dictionary = labels(&member_candidates(MEMBER_SOURCE, "dict", "", 0));
    assert!(dictionary.contains(&"Exists".to_string()), "{dictionary:?}");
    assert!(dictionary.contains(&"Keys".to_string()), "{dictionary:?}");
}

#[test]
fn unknown_base_has_no_members() {
    assert!(member_candidates(MEMBER_SOURCE, "plain", "", 0).is_empty());
    assert!(member_candidates(MEMBER_SOURCE, "nothing", "", 0).is_empty());
}
