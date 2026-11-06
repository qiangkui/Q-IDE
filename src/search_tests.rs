//! 项目内搜索的匹配与扫描测试。

use crate::search::{
    find_in_text, replace_in_files, replace_in_text, search_files, SearchQuery, SearchResults,
    MAX_SEARCH_HITS,
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
    assert!(!find_in_text("anything", &SearchQuery::new(""), "a", &mut hits));
    assert!(hits.is_empty());
}

#[test]
fn carriage_returns_are_stripped_from_previews() {
    let query = SearchQuery::new("b");
    assert_eq!(
        hits_of("a\r\nb\r\n", &query),
        vec![(2, 1, "b".to_string())]
    );
}

#[test]
fn results_count_files_and_stop_at_the_hit_limit() {
    let mut results = SearchResults::default();
    let mut hits = Vec::new();
    let line = "needle\n";
    let text = line.repeat(MAX_SEARCH_HITS + 5);
    assert!(find_in_text(&text, &SearchQuery::new("needle"), "a", &mut hits));
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
    assert_eq!(replace_in_text("Cat cat", &query, "dog"), ("Cat dog".into(), 1));

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
