//! VB 块级缩进正则的测试：关键字开块、普通语句不误判、结束关键字识别。

use std::sync::OnceLock;

use crate::indent;

/// 与编辑器一致：用「光标之前的内容」判断是否多缩进一级。
fn opens(line: &str) -> bool {
    static REGEX: OnceLock<regex::Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| regex::Regex::new(indent::VB_INCREASE).expect("VB_INCREASE 应当合法"))
        .is_match(line.trim_end())
}

/// 与编辑器一致：用「光标之后的内容」判断是否少缩进一级。
fn closes(rest: &str) -> bool {
    static REGEX: OnceLock<regex::Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| regex::Regex::new(indent::VB_DECREASE).expect("VB_DECREASE 应当合法"))
        .is_match(rest)
}

#[test]
fn block_openers_increase_indent() {
    for line in [
        "Sub Main()",
        "Private Sub Handle(ByVal x As Long)",
        "Public Function Add(a As Long) As Long",
        "Static Function Counter() As Long",
        "Public Property Get Total() As Long",
        "Private Property Let Total(ByVal value As Long)",
        "Property Set Item(ByVal v As Object)",
        "Public Type Point",
        "Private Class Square",
        "Type Colors",
        "Class Person",
        "Do",
        "Do While n > 0",
        "Do Until n = 3",
        "For i = 1 To 10",
        "For Each item In items",
        "While n < 3",
        "With obj",
        "Select Case grade",
        "Case 1, 2",
        "Case Else",
        "Else",
        "If a > 0 Then",
        "ElseIf a = 0 Then",
    ] {
        assert!(opens(line), "应当增加缩进：{line:?}");
    }
}

#[test]
fn ordinary_statements_keep_indent() {
    for line in [
        "x = 1",
        "Subtract = 1",
        "FunctionName = \"x\"",
        "Format = 1",
        "Double = 2",
        "TypeName = 3",
        "CaseSensitive = True",
        "DoSomething",
        "ForEach = 1",
        "Window = 1",
        "If a > 0 Then x = 1",
        "If a > 0 Then: x = 1",
        // 行尾注释：编辑器只在「代码」上下文里应用缩进规则，光标落在注释里时不会触发。
        "If a > 0 Then ' 注释",
        "' Sub Main",
        "Debug.Print \"If x Then\"",
        "End Sub",
        "Next i",
    ] {
        assert!(!opens(line), "不应增加缩进：{line:?}");
    }
}

#[test]
fn closers_are_recognized() {
    for line in [
        "End Sub",
        "End Function",
        "End If",
        "End With",
        "End Select",
        "End Type",
        "End Class",
        "End Property",
        "End",
        "Next",
        "Next i",
        "Loop",
        "Wend",
        "    End If",
    ] {
        assert!(closes(line), "应当识别为块结束：{line:?}");
    }
}

#[test]
fn closer_words_inside_identifiers_do_not_match() {
    for line in [
        "EndTime = 1",
        "NextStep = 2",
        "LoopCount = 3",
        "Wendy = 4",
        "x = 1",
    ] {
        assert!(!closes(line), "不应识别为块结束：{line:?}");
    }
}

#[test]
fn closes_block_matches_whole_lines() {
    // `End Sub` 行尾回车时用来补一次反缩进：整行（含行尾空白）判断。
    for line in [
        "End Sub",
        "    End Sub  ",
        "Next i",
        "Loop",
        "End If",
        "End With",
    ] {
        assert!(indent::closes_block(line), "应当识别为块结束行：{line:?}");
    }
    for line in [
        "",
        "    ",
        "EndTime = 1",
        "NextStep = 2",
        "x = 1",
        "' End Sub",
    ] {
        assert!(!indent::closes_block(line), "不应识别为块结束行：{line:?}");
    }
}
