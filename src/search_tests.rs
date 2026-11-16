//! 项目内搜索的匹配与扫描测试。

use crate::search::{
    MAX_SEARCH_HITS, SearchHit, SearchQuery, SearchResults, file_hit_indices, find_in_text,
    replace_in_files, replace_in_text, replace_selected, search_files, snapshot_files,
    toggle_file_selection, undo_replace,
};

fn hits_of(text: &str, query: &SearchQuery) -> Vec<(usize, usize, String)> {
    let mut hits = Vec::new();
    find_in_text(text, query, "C:\\p\\a.vb", &mut hits);
    hits.into_iter()
        .map(|hit| (hit.line, hit.column, hit.preview))
        .collect()
}

#[test]
fn matches_are_case_insensitive_by_default() {
    let query = SearchQuery::new("hello");
    assert_eq!(
        hits_of("Hello world\nhello again\n", &query),
        vec![
            (1, 1, "Hello world".to_string()),
            (2, 1, "hello again".to_string()),
        ]
    );
}

#[test]
fn case_sensitive_query_skips_wrong_case() {
    let mut query = SearchQuery::new("hello");
    query.case_sensitive = true;
    assert_eq!(
        hits_of("Hello world\nhello again\n", &query),
        vec![(2, 1, "hello again".to_string())]
    );
}

#[test]
fn whole_word_ignores_hits_inside_longer_identifiers() {
    let mut query = SearchQuery::new("cat");
    query.whole_word = true;
    assert_eq!(
        hits_of("concat(\"cat\")\ncategory\n", &query),
        vec![(1, 9, "concat(\"cat\")".to_string())]
    );

    query.whole_word = false;
    let lines: Vec<usize> = hits_of("concat(\"cat\")\ncategory\n", &query)
        .into_iter()
        .map(|(line, _, _)| line)
        .collect();
    assert_eq!(lines, vec![1, 2]);
}

#[test]
fn columns_are_one_based_character_positions() {
    let query = SearchQuery::new("变量");
    assert_eq!(
        hits_of("Dim 变量 As Long\n", &query),
        vec![(1, 5, "Dim 变量 As Long".to_string())]
    );
}

#[test]
fn empty_query_finds_nothing() {
    assert!(hits_of("anything", &SearchQuery::new("   ")).is_empty());
    let mut hits = Vec::new();
    assert!(!find_in_text(
        "anything",
        &SearchQuery::new(""),
        "a",
        &mut hits
    ));
    assert!(hits.is_empty());
}

#[test]
fn carriage_returns_are_stripped_from_previews() {
    let query = SearchQuery::new("b");
    assert_eq!(hits_of("a\r\nb\r\n", &query), vec![(2, 1, "b".to_string())]);
}

#[test]
fn results_count_files_and_stop_at_the_hit_limit() {
    let mut results = SearchResults::default();
    let mut hits = Vec::new();
    let line = "needle\n";
    let text = line.repeat(MAX_SEARCH_HITS + 5);
    assert!(find_in_text(
        &text,
        &SearchQuery::new("needle"),
        "a",
        &mut hits
    ));
    results.hits = hits;
    assert_eq!(results.hits.len(), MAX_SEARCH_HITS);
    assert_eq!(results.file_count(), 1);
}

#[test]
fn search_files_reads_text_and_skips_binaries() {
    let dir = std::env::temp_dir().join(format!("gpui-first-search-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let text = dir.join("a.vb");
    let binary = dir.join("b.bin");
    std::fs::write(&text, "MsgBox \"hi\"\n").expect("写文本文件");
    std::fs::write(&binary, [0x4d, 0x5a, 0x00, 0x01, 0x02]).expect("写二进制文件");

    let paths = vec![
        text.to_string_lossy().into_owned(),
        binary.to_string_lossy().into_owned(),
        dir.join("missing.vb").to_string_lossy().into_owned(),
    ];
    let results = search_files(&paths, &SearchQuery::new("msgbox"));

    assert_eq!(results.hits.len(), 1, "{:?}", results.hits);
    assert_eq!(results.hits[0].line, 1);
    assert_eq!(results.files, 1);
    assert_eq!(results.skipped, 2, "二进制与读不了的文件都要计入跳过");
    assert!(!results.truncated);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn blank_query_skips_every_file() {
    let paths: Vec<String> = Vec::new();
    let results = search_files(&paths, &SearchQuery::new(""));
    assert!(results.is_empty());
    assert_eq!(results.files, 0);
}

#[test]
fn total_budget_stops_the_scan() {
    let dir = std::env::temp_dir().join(format!("gpui-first-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let first = dir.join("a.txt");
    let second = dir.join("b.txt");
    std::fs::write(&first, "needle\n").expect("写文件");
    std::fs::write(&second, "needle\n").expect("写文件");
    let paths = vec![
        first.to_string_lossy().into_owned(),
        second.to_string_lossy().into_owned(),
    ];

    // 预算只够读一个文件：第二个文件不会被打开。
    let results = crate::search::search_files_with_budget(&paths, &SearchQuery::new("needle"), 7);
    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.files, 1);
    assert!(results.truncated);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_keeps_original_line() {
    let query = SearchQuery::new("x");
    let found = hits_of("    x = 1\n", &query);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].2, "    x = 1");
}

#[test]
fn replacing_hits_every_occurrence_in_a_line() {
    let (text, count) = replace_in_text("cat cat catalog\n", &SearchQuery::new("cat"), "dog");
    assert_eq!(count, 3);
    assert_eq!(text, "dog dog dogalog\n");
}

#[test]
fn replacing_is_case_insensitive_by_default() {
    let (text, count) = replace_in_text("Cat CAT cat", &SearchQuery::new("cat"), "dog");
    assert_eq!(count, 3);
    assert_eq!(text, "dog dog dog");
}

#[test]
fn replacing_respects_case_sensitive_and_whole_word() {
    let mut query = SearchQuery::new("cat");
    query.case_sensitive = true;
    assert_eq!(
        replace_in_text("Cat cat", &query, "dog"),
        ("Cat dog".into(), 1)
    );

    let mut query = SearchQuery::new("cat");
    query.whole_word = true;
    assert_eq!(
        replace_in_text("concat cat", &query, "dog"),
        ("concat dog".into(), 1)
    );
}

#[test]
fn replacing_keeps_line_endings() {
    let (text, count) = replace_in_text("a\r\nb\na\r\n", &SearchQuery::new("a"), "z");
    assert_eq!(count, 2);
    assert_eq!(text, "z\r\nb\nz\r\n");
}

#[test]
fn replacing_accepts_an_empty_replacement_and_non_ascii() {
    let (text, count) = replace_in_text("变量 = 1\n", &SearchQuery::new("变量"), "");
    assert_eq!(count, 1);
    assert_eq!(text, " = 1\n");

    let (text, count) = replace_in_text("x = 1", &SearchQuery::new("x"), "结果");
    assert_eq!(count, 1);
    assert_eq!(text, "结果 = 1");
}

#[test]
fn an_empty_query_replaces_nothing() {
    let (text, count) = replace_in_text("abc", &SearchQuery::new("  "), "x");
    assert_eq!(count, 0);
    assert_eq!(text, "abc");
}

#[test]
fn replace_in_files_writes_back_and_reports_counts() {
    let dir = std::env::temp_dir().join(format!("gpui-first-replace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let text = dir.join("a.vb");
    let other = dir.join("b.vb");
    let binary = dir.join("c.bin");
    std::fs::write(&text, "MsgBox \"hi\"\r\nMsgBox \"hi\"\r\n").expect("写文件");
    std::fs::write(&other, "没有命中\n").expect("写文件");
    std::fs::write(&binary, [0x00, 0x01]).expect("写二进制文件");

    let paths = vec![
        text.to_string_lossy().into_owned(),
        other.to_string_lossy().into_owned(),
        binary.to_string_lossy().into_owned(),
    ];
    let outcome = replace_in_files(&paths, &SearchQuery::new("msgbox"), "Debug.Print");

    assert_eq!(outcome.replacements, 2);
    assert_eq!(outcome.changed.len(), 1, "{:?}", outcome.changed);
    assert_eq!(outcome.skipped, 1, "二进制文件应计入跳过");
    assert!(outcome.failures.is_empty());
    assert_eq!(
        std::fs::read_to_string(&text).expect("读回文件"),
        "Debug.Print \"hi\"\r\nDebug.Print \"hi\"\r\n",
        "行尾要保持 CRLF"
    );
    assert_eq!(
        std::fs::read_to_string(&other).expect("读回文件"),
        "没有命中\n"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 正则模式：`\d+` 命中数字、列号与命中长度都按字符算。
#[test]
fn regex_query_matches_patterns_and_reports_span() {
    let mut query = SearchQuery::new(r"\d+");
    query.regex = true;
    let mut hits = Vec::new();
    find_in_text("let x = 42\nno digits\n", &query, "a", &mut hits);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line, 1);
    assert_eq!(hits[0].column, 9);
    assert_eq!(hits[0].length, 2);
    // 列号按字符算：非 ASCII 前缀不会把列号推后。
    let mut hits = Vec::new();
    find_in_text("中文 7\n", &query, "a", &mut hits);
    assert_eq!((hits[0].column, hits[0].length), (4, 1));
}

#[test]
fn regex_query_honours_case_option() {
    let mut query = SearchQuery::new("hello");
    query.regex = true;
    assert_eq!(hits_of("Hello\nhello\n", &query).len(), 2);
    query.case_sensitive = true;
    assert_eq!(hits_of("Hello\nhello\n", &query).len(), 1);
}

/// 全词在正则模式下是「给整个模式加一层词边界」。
#[test]
fn regex_query_with_whole_word_wraps_pattern() {
    let mut query = SearchQuery::new(r"cat\d*");
    query.regex = true;
    query.whole_word = true;
    assert!(hits_of("cat1 and concat2\n", &query).len() == 1);
    assert!(hits_of("concat2\n", &query).is_empty());
}

/// 正则替换支持 `$1` 分组引用，并统计替换次数。
#[test]
fn replace_in_text_with_regex_uses_group_references() {
    let mut query = SearchQuery::new(r"(\w+) = (\d+)");
    query.regex = true;
    let (text, count) = replace_in_text("a = 1\r\nb = 2\r\nuntouched", &query, "$2 = $1");
    assert_eq!(text, "1 = a\r\n2 = b\r\nuntouched");
    assert_eq!(count, 2);
}

/// 正则写错：搜索不产生命中并标记无效，替换原样返回。
#[test]
fn invalid_regex_is_reported_and_changes_nothing() {
    let mut query = SearchQuery::new("(unclosed");
    query.regex = true;
    assert!(query.compile().is_err());
    let results = crate::search::search_files_with_budget(&[], &query, 1 << 20);
    assert!(results.invalid_pattern);
    assert!(results.hits.is_empty());
    let (text, count) = replace_in_text("keep (unclosed", &query, "x");
    assert_eq!((text.as_str(), count), ("keep (unclosed", 0));
}

/// 非正则模式完全不受影响（同一个关键字在两种模式下结果一致）。
#[test]
fn literal_query_is_unchanged_by_regex_support() {
    let query = SearchQuery::new("a.b");
    let expected = hits_of("a.b and axb\n", &query);
    assert_eq!(expected.len(), 1);
    let mut regex_query = SearchQuery::new("a.b");
    regex_query.regex = true;
    // 每行只记第一处命中，所以换行来体现「正则里 `.` 能匹配任意字符」。
    assert_eq!(hits_of("a.b\naxb\n", &regex_query).len(), 2);
    assert!(regex_query.compile().is_ok());
}
/// GBK 源码文件也要能搜到（中文 VB6 工程大量是 GBK）：按打开文件时的同一套探测解码。
#[test]
fn gbk_source_files_are_searchable() {
    use crate::encoding::{self, FileEncoding};

    let dir = std::env::temp_dir().join(format!("gpui-first-search-gbk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("gbk.vb");
    let body = "Attribute VB_Name = \"模块\"\r\nSub 测试()\r\nMsgBox \"中文提示\"\r\nEnd Sub\r\n";
    std::fs::write(&path, encoding::encode(body, FileEncoding::Gbk)).expect("写 GBK 文件");

    let paths = vec![path.to_string_lossy().into_owned()];
    let ascii = crate::search::search_files(&paths, &SearchQuery::new("msgbox"));
    assert_eq!(
        ascii.hits.len(),
        1,
        "ASCII 关键字应当能命中（大小写不敏感）"
    );
    assert_eq!(ascii.skipped, 0, "GBK 文件不该被当成二进制跳过");
    let chinese = crate::search::search_files(&paths, &SearchQuery::new("中文提示"));
    assert_eq!(chinese.hits.len(), 1);
    assert_eq!(chinese.hits[0].line, 3);
    assert!(chinese.hits[0].preview.contains("中文提示"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// 带 BOM 的 UTF-16 文件正文里有 0 字节，但它是文本，不该被「二进制」判断挡掉。
#[test]
fn utf16_bom_files_are_searchable() {
    use crate::encoding::{self, FileEncoding};

    let dir = std::env::temp_dir().join(format!("gpui-first-search-utf16-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("wide.vb");
    std::fs::write(
        &path,
        encoding::encode("Debug.Print \"宽字符\"\n", FileEncoding::Utf16Le),
    )
    .expect("写 UTF-16 文件");

    let paths = vec![path.to_string_lossy().into_owned()];
    let results = crate::search::search_files(&paths, &SearchQuery::new("宽字符"));
    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.skipped, 0);

    let _ = std::fs::remove_dir_all(&dir);
}
/// 跨文件替换要按**原编码**写回：GBK 文件替换后必须还是 GBK（写成 UTF-8 会毁掉中文）。
#[test]
fn replace_in_files_keeps_the_original_encoding() {
    use crate::encoding::{self, FileEncoding};

    let dir = std::env::temp_dir().join(format!("gpui-first-replace-gbk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("gbk.vb");
    let body = "Attribute VB_Name = \"模块\"\r\nMsgBox \"旧提示\"\r\n";
    std::fs::write(&path, encoding::encode(body, FileEncoding::Gbk)).expect("写 GBK 文件");

    let paths = vec![path.to_string_lossy().into_owned()];
    let outcome = replace_in_files(&paths, &SearchQuery::new("旧提示"), "新提示");
    assert_eq!(outcome.replacements, 1);
    assert_eq!(outcome.changed.len(), 1);

    let bytes = std::fs::read(&path).expect("读回文件");
    // 仍是 GBK：按 GBK 解码拿到新文本，且**不是**合法 UTF-8（中文部分）。
    let decoded = encoding::decode_with(&bytes, FileEncoding::Gbk);
    assert!(decoded.contains("新提示"), "GBK 内容应当保留：{decoded:?}");
    assert!(std::str::from_utf8(&bytes).is_err(), "不该被改写成 UTF-8");

    let _ = std::fs::remove_dir_all(&dir);
}
/// 带 BOM 的 UTF-8 文件替换后 BOM 还在（否则编辑器下次打开会按别的编码猜）。
#[test]
fn replace_in_files_keeps_the_utf8_bom() {
    use crate::encoding::{self, FileEncoding};

    let dir = std::env::temp_dir().join(format!("gpui-first-replace-bom-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("bom.vb");
    std::fs::write(
        &path,
        encoding::encode("MsgBox \"旧\"\n", FileEncoding::Utf8Bom),
    )
    .expect("写带 BOM 的 UTF-8 文件");

    let paths = vec![path.to_string_lossy().into_owned()];
    let outcome = replace_in_files(&paths, &SearchQuery::new("旧"), "新");
    assert_eq!(outcome.replacements, 1);

    let bytes = std::fs::read(&path).expect("读回文件");
    assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF], "BOM 应当保留");
    assert!(String::from_utf8(bytes).expect("仍是 UTF-8").contains("新"));

    let _ = std::fs::remove_dir_all(&dir);
}
/// 逐处勾选：只动勾上的行，没勾的行原样保留（列表一行一条命中）。
#[test]
fn replace_selected_only_touches_checked_lines() {
    let dir = std::env::temp_dir().join(format!("gpui-first-pick-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("a.vb");
    std::fs::write(&path, "old 1\nold 2\nold 3\n").expect("写文件");

    let query = SearchQuery::new("old");
    let mut hits = Vec::new();
    let text = std::fs::read_to_string(&path).expect("读文件");
    find_in_text(&text, &query, &path.to_string_lossy(), &mut hits);
    assert_eq!(hits.len(), 3);
    // 只勾第 2 行。
    let picked = vec![hits[1].clone()];
    let outcome = replace_selected(&picked, &query, "new");
    assert_eq!(outcome.replacements, 1);
    assert_eq!(outcome.changed.len(), 1);
    let after = std::fs::read_to_string(&path).expect("读回文件");
    assert_eq!(after, "old 1\nnew 2\nold 3\n");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 勾上的一行里有多个匹配时全部替换（命中粒度是行，次数按真实命中算）。
#[test]
fn replace_selected_replaces_every_match_on_checked_line() {
    let dir = std::env::temp_dir().join(format!("gpui-first-pick2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("a.vb");
    std::fs::write(&path, "old old\nkept\n").expect("写文件");

    let query = SearchQuery::new("old");
    let hit = SearchHit {
        path: path.to_string_lossy().into_owned(),
        line: 1,
        column: 1,
        length: 3,
        preview: "old old".to_string(),
    };
    let outcome = replace_selected(&[hit], &query, "new");
    assert_eq!(outcome.replacements, 2);
    let after = std::fs::read_to_string(&path).expect("读回文件");
    assert_eq!(after, "new new\nkept\n");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 逐处替换同样按原编码写回（GBK 文件不会被改成 UTF-8）。
#[test]
fn replace_selected_keeps_the_original_encoding() {
    use crate::encoding::{self, FileEncoding};

    let dir = std::env::temp_dir().join(format!("gpui-first-pick3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("gbk.vb");
    std::fs::write(&path, encoding::encode("提示旧\n保留\n", FileEncoding::Gbk))
        .expect("写 GBK 文件");

    let query = SearchQuery::new("旧");
    let hit = SearchHit {
        path: path.to_string_lossy().into_owned(),
        line: 1,
        column: 3,
        length: 1,
        preview: "提示旧".to_string(),
    };
    let outcome = replace_selected(&[hit], &query, "新");
    assert_eq!(outcome.replacements, 1);
    let bytes = std::fs::read(&path).expect("读回文件");
    assert!(encoding::decode_with(&bytes, FileEncoding::Gbk).contains("提示新"));
    assert!(std::str::from_utf8(&bytes).is_err(), "不该被改写成 UTF-8");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 撤销把文件逐字节写回替换前的内容（含 CRLF 与编码）。
#[test]
fn undo_restores_the_original_bytes() {
    use crate::encoding::{self, FileEncoding};

    let dir = std::env::temp_dir().join(format!("gpui-first-undo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let path = dir.join("a.vb");
    let original = encoding::encode("MsgBox \"旧\"\r\nMsgBox \"旧\"\r\n", FileEncoding::Gbk);
    std::fs::write(&path, &original).expect("写文件");

    let paths = vec![path.to_string_lossy().into_owned()];
    let undo = snapshot_files(&paths);
    assert!(!undo.is_empty());
    let outcome = replace_in_files(&paths, &SearchQuery::new("旧"), "新");
    assert_eq!(outcome.replacements, 2);
    assert_ne!(std::fs::read(&path).expect("读回"), original);

    let (restored, failures) = undo_replace(&undo);
    assert_eq!((restored, failures.len()), (1, 0));
    assert_eq!(std::fs::read(&path).expect("读回"), original);

    // 撤销之后就没有可撤销的内容了（面板据此把按钮置灰）。
    let (restored, _) = undo_replace(&snapshot_files(&[]));
    assert_eq!(restored, 0);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 没勾任何命中、空关键字、正则写错时都不动文件。
#[test]
fn replace_selected_is_a_no_op_without_hits() {
    let query = SearchQuery::new("old");
    let outcome = replace_selected(&[], &query, "new");
    assert_eq!(outcome, Default::default());

    let hit = SearchHit {
        path: "C:\\none\\x.vb".to_string(),
        line: 1,
        column: 1,
        length: 3,
        preview: "old".to_string(),
    };
    assert_eq!(
        replace_selected(std::slice::from_ref(&hit), &SearchQuery::new(""), "new"),
        Default::default()
    );
    let mut broken = SearchQuery::new("(");
    broken.regex = true;
    assert_eq!(
        replace_selected(std::slice::from_ref(&hit), &broken, "new"),
        Default::default()
    );
}

fn grouped_hits() -> Vec<SearchHit> {
    ["a.vb", "a.vb", "a.vb", "b.vb"]
        .iter()
        .enumerate()
        .map(|(index, path)| SearchHit {
            path: path.to_string(),
            line: index + 1,
            column: 1,
            length: 3,
            preview: "old".to_string(),
        })
        .collect()
}

/// 双击文件标题的语义：这一组全勾着就全不勾，否则全勾，别的文件不受影响。
#[test]
fn toggle_file_selection_flips_only_that_file() {
    let hits = grouped_hits();
    assert_eq!(file_hit_indices(&hits, "a.vb"), vec![0, 1, 2]);
    assert_eq!(file_hit_indices(&hits, "b.vb"), vec![3]);
    assert!(file_hit_indices(&hits, "没有这个文件").is_empty());

    // 只勾了 a.vb 的前两条：切换后 a.vb 变成全勾，b.vb 保持原样。
    let mut selected = vec![true, true, false, false];
    assert_eq!(toggle_file_selection(&hits, &mut selected, "a.vb"), 3);
    assert_eq!(selected, vec![true, true, true, false]);

    // a.vb 已全勾：切换后全部取消；b.vb 仍不动。
    assert_eq!(toggle_file_selection(&hits, &mut selected, "a.vb"), 0);
    assert_eq!(selected, vec![false, false, false, false]);

    // b.vb 只有一条：勾上。
    assert_eq!(toggle_file_selection(&hits, &mut selected, "b.vb"), 1);
    assert_eq!(selected, vec![false, false, false, true]);

    // 不存在的文件不改变任何状态。
    assert_eq!(toggle_file_selection(&hits, &mut selected, "zzz.vb"), 0);
    assert_eq!(selected, vec![false, false, false, true]);
}
