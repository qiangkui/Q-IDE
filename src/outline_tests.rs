use std::collections::HashMap;

use crate::outline::{
    GotoTarget, OutlineModel, breakpoint_entries, definition, filter_symbols, goto_target,
    group_by_scope, references, split_declarations,
};

#[test]
fn filter_matches_names_case_insensitively() {
    let model = OutlineModel::from_source(
        "Dim Total As Long\nDim Items(3) As Long\n\nSub AddOne()\nEnd Sub\n",
    );
    let names = |query: &str| -> Vec<String> {
        filter_symbols(&model.variables, query)
            .into_iter()
            .map(|symbol| symbol.name.clone())
            .collect()
    };

    assert_eq!(names("tot"), ["total"]);
    assert_eq!(names("TOTAL"), ["total"], "忽略大小写");
    assert_eq!(names("  ite "), ["items"], "查询串会去掉首尾空白");
    assert_eq!(names(""), ["total", "items"], "空查询返回全部");
    assert!(names("zzz").is_empty());
}

#[test]
fn breakpoint_entries_list_lines_in_order_with_conditions() {
    let source = "\
Dim i As Long
For i = 1 To 3
    Debug.Print i
Next
";
    let conditions = HashMap::from([(3, "i = 2".to_string())]);
    let entries = breakpoint_entries([4, 1, 3], &conditions, source);

    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries.iter().map(|entry| entry.line).collect::<Vec<_>>(),
        [1, 3, 4],
        "按行号升序排列"
    );
    assert_eq!(entries[0].text, "Dim i As Long");
    assert_eq!(entries[1].text, "Debug.Print i");
    assert_eq!(entries[1].condition.as_deref(), Some("i = 2"));
    assert_eq!(entries[2].condition, None);
}

#[test]
fn breakpoint_entries_tolerate_missing_lines_and_duplicates() {
    // 行号越界（例如删掉了最后一行）时给空文本，重复行号只留一条。
    let entries = breakpoint_entries([2, 2, 99], &HashMap::new(), "Dim i As Long\n");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].line, 2);
    assert!(entries[0].text.is_empty());
    assert_eq!(entries[1].line, 99);
    assert!(entries[1].text.is_empty());
}

#[test]
fn valid_vb_source_exposes_procedures_and_variables() {
    let model = OutlineModel::from_source(
        "Dim total As Long\n\
         Public Function AddOne(value As Long) As Long\n\
             Dim result As Long\n\
             result = value + 1\n\
             AddOne = result\n\
         End Function\n",
    );

    assert!(
        model
            .procedures
            .iter()
            .any(|symbol| symbol.name.eq_ignore_ascii_case("AddOne"))
    );
    assert!(
        model
            .variables
            .iter()
            .any(|symbol| symbol.name.eq_ignore_ascii_case("total"))
    );
    assert!(
        model
            .variables
            .iter()
            .any(|symbol| symbol.name.eq_ignore_ascii_case("value"))
    );
    assert!(
        model
            .variables
            .iter()
            .any(|symbol| symbol.name.eq_ignore_ascii_case("result"))
    );
}

#[test]
fn variables_are_grouped_by_scope() {
    let model = OutlineModel::from_source(
        "Dim total As Long\n\
         Sub Foo()\n\
             Dim local As Long\n\
         End Sub\n",
    );

    // 解析得到的标识符按 VB 规则统一成小写。
    assert_eq!(
        variable_groups(&model),
        vec![
            ("模块".to_string(), vec!["total".to_string()]),
            ("foo".to_string(), vec!["local".to_string()]),
        ],
        "模块变量与过程局部变量应分成两组"
    );
}

#[test]
fn class_members_are_grouped_under_their_class_scope() {
    let model = OutlineModel::from_source(
        "Class Box\n\
             Public Value As Long\n\
             Public Sub Bump(ByVal amount As Long)\n\
                 Dim local As Long\n\
             End Sub\n\
         End Class\n",
    );

    assert_eq!(
        variable_groups(&model),
        vec![
            ("Box".to_string(), vec!["value".to_string()]),
            (
                "Box.bump".to_string(),
                vec!["amount".to_string(), "local".to_string()]
            ),
        ],
        "字段归到类，过程里的参数与局部变量归到 `类名.过程名`"
    );
}

#[test]
fn incomplete_vb_source_groups_variables_by_enclosing_procedure() {
    let model = OutlineModel::from_source(
        "Dim total As Long\n\
         Sub Foo()\n\
             Dim local As Long\n",
    );

    assert_eq!(
        variable_groups(&model),
        vec![
            ("模块".to_string(), vec!["total".to_string()]),
            ("Foo".to_string(), vec!["local".to_string()]),
        ],
        "解析失败时也要按最近的 Sub/Function 归类"
    );
}

fn variable_groups(model: &OutlineModel) -> Vec<(String, Vec<String>)> {
    group_by_scope(&model.variables)
        .into_iter()
        .map(|(scope, symbols)| {
            (
                scope.to_string(),
                symbols
                    .into_iter()
                    .map(|symbol| symbol.name.clone())
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn definition_prefers_the_current_procedures_local_variable() {
    let model = OutlineModel::from_source(
        "Dim total As Long\n\
         Sub Foo()\n\
             Dim total As Long\n\
             total = 1\n\
         End Sub\n",
    );

    // 第 4 行的 `total` 属于 Foo，应当跳到第 3 行的局部声明。
    let found = definition(&model, "TOTAL", 4).expect("应当找到定义");
    assert_eq!(found.line, 3);

    // 过程外面的 `total` 走模块变量。
    let module = definition(&model, "total", 1).expect("应当找到定义");
    assert_eq!(module.line, 1);
}

#[test]
fn definition_finds_procedures_types_and_classes() {
    let model = OutlineModel::from_source(
        "Type Point\n\
             X As Long\n\
         End Type\n\
         Class Box\n\
             Public Sub Bump()\n\
             End Sub\n\
         End Class\n\
         Enum Color\n\
             Red\n\
         End Enum\n\
         Declare Function MessageBoxW Lib \"user32\" (ByVal hwnd As Long) As Long\n\
         Sub Main()\n\
             Dim p As Point\n\
             p.Bump\n\
         End Sub\n",
    );

    assert_eq!(definition(&model, "Main", 1).map(|s| s.line), Some(12));
    assert_eq!(definition(&model, "Point", 1).map(|s| s.line), Some(1));
    assert_eq!(definition(&model, "Color", 1).map(|s| s.line), Some(8));
    assert_eq!(
        definition(&model, "MessageBoxW", 1).map(|s| s.line),
        Some(11)
    );
    // `p.Bump` 里的方法名只有方法名本身，靠 `类名.方法名` 后缀匹配到类方法。
    assert_eq!(definition(&model, "Bump", 14).map(|s| s.line), Some(5));
}

#[test]
fn definition_returns_none_for_unknown_names() {
    let model = OutlineModel::from_source("Dim total As Long\n");
    assert!(definition(&model, "missing", 1).is_none());
}

#[test]
fn incomplete_vb_source_keeps_visible_declarations_in_the_outline() {
    let model = OutlineModel::from_source(
        "Public Function StillTyping(value As Long) As Long\n\
         Dim pending As String\n",
    );

    assert!(
        model
            .procedures
            .iter()
            .any(|symbol| symbol.name == "StillTyping")
    );
    assert!(
        model
            .variables
            .iter()
            .any(|symbol| symbol.name == "pending")
    );
}

#[test]
fn references_finds_every_whole_word_use() {
    let source = "Dim total As Long\n\
         total = 1\n\
         subtotal = total + 2   ' total 在注释里\n\
         Debug.Print TOTAL\n";
    let found = references(source, "total");
    let lines: Vec<usize> = found.iter().map(|(line, _)| *line).collect();
    assert_eq!(lines, [1, 2, 3, 4], "整词、忽略大小写、注释里的不算：{found:?}");
    assert_eq!(found[1].1, "total = 1", "应当带上该行文本");
}

#[test]
fn references_ignores_names_within_longer_identifiers() {
    let source = "Dim count As Long\ncounter = count + 1\ndiscount = 0\n";
    let found = references(source, "count");
    let lines: Vec<usize> = found.iter().map(|(line, _)| *line).collect();
    assert_eq!(lines, [1, 2], "`counter`/`discount` 不算");
}

#[test]
fn references_of_an_unknown_name_is_empty() {
    assert!(references("Dim x As Long\n", "nothing").is_empty());
    assert!(references("Dim x As Long\n", "").is_empty());
}

#[test]
fn declarations_split_only_on_top_level_commas() {
    assert_eq!(
        split_declarations("a As Long, b(1, 2) As Long"),
        ["a As Long", "b(1, 2) As Long"]
    );
    assert_eq!(
        split_declarations(r#"s As String = "x, y""#),
        [r#"s As String = "x, y""#]
    );
    assert_eq!(
        split_declarations(r#"s As String = "a""b, c""#),
        [r#"s As String = "a""b, c""#],
        "双引号转义不结束字符串"
    );
}
#[test]
fn goto_target_parses_line_numbers_and_names() {
    assert_eq!(goto_target("42"), Some(GotoTarget::Line(42)));
    assert_eq!(goto_target("  7 "), Some(GotoTarget::Line(7)));
    assert_eq!(
        goto_target("AddOne"),
        Some(GotoTarget::Symbol("AddOne".to_string()))
    );
    assert_eq!(goto_target("Box.Bump"), Some(GotoTarget::Symbol("Box.Bump".to_string())));
    assert_eq!(goto_target("0"), None, "行号从 1 开始");
    assert_eq!(goto_target("   "), None);
}