//! 「转到文件」的模糊匹配与目录扫描测试。

use std::collections::HashSet;
use std::path::PathBuf;

use crate::workspace::{best_match, collect_files, disk_stamp, fuzzy_score, reprefix_path, stamp_differs};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gpui-first-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    dir
}

#[test]
fn fuzzy_prefers_exact_then_prefix_then_contains() {
    let exact = fuzzy_score("main.rs", "C:\\p\\main.rs").expect("完全匹配");
    let prefix = fuzzy_score("main", "C:\\p\\main.rs").expect("前缀匹配");
    let contains = fuzzy_score("ain", "C:\\p\\main.rs").expect("包含匹配");

    assert_eq!(exact, 10_000);
    assert!(exact > prefix, "{exact} > {prefix}");
    assert!(prefix > contains, "{prefix} > {contains}");
}

#[test]
fn fuzzy_matches_subsequence_and_ignores_case() {
    assert!(fuzzy_score("mnrs", "C:\\p\\main.rs").is_some(), "子序列");
    assert_eq!(fuzzy_score("MAIN.RS", "c:\\p\\Main.Rs"), Some(10_000));
    assert!(fuzzy_score("zzz", "C:\\p\\main.rs").is_none());
    assert!(fuzzy_score("", "C:\\p\\main.rs").is_none());
}

#[test]
fn path_query_matches_the_full_path() {
    let by_path = fuzzy_score("src/main", "C:\\proj\\src\\main.rs").expect("路径匹配");
    let by_name = fuzzy_score("main", "C:\\proj\\src\\main.rs").expect("文件名匹配");

    assert!(by_path > 0);
    assert!(
        by_name > by_path,
        "只写文件名时应当比写完整路径更优先：{by_name} > {by_path}"
    );
    assert!(
        fuzzy_score("src\\main", "C:\\proj\\src\\main.rs").is_some(),
        "反斜杠路径也要能匹配"
    );
}

#[test]
fn best_match_picks_the_closest_name() {
    let candidates = vec![
        "C:\\p\\main.rs".to_string(),
        "C:\\p\\domain.rs".to_string(),
        "C:\\p\\maintenance.rs".to_string(),
    ];
    assert_eq!(
        best_match("main", &candidates).map(String::as_str),
        Some("C:\\p\\main.rs"),
        "前缀更短的 main.rs 应当胜出"
    );
    assert_eq!(best_match("nomatch", &candidates), None);
}

#[test]
fn collect_files_skips_build_and_hidden_directories() {
    let dir = temp_dir("goto-files");
    std::fs::create_dir_all(dir.join("src")).expect("创建 src");
    std::fs::create_dir_all(dir.join("target")).expect("创建 target");
    std::fs::create_dir_all(dir.join(".git")).expect("创建 .git");
    std::fs::write(dir.join("root.txt"), "x").expect("写文件");
    std::fs::write(dir.join("src").join("main.rs"), "x").expect("写文件");
    std::fs::write(dir.join("target").join("debug.rs"), "x").expect("写文件");

    let mut files = Vec::new();
    let mut seen = HashSet::new();
    collect_files(&dir, &mut files, &mut seen);

    let names: Vec<String> = files
        .iter()
        .map(|path| {
            PathBuf::from(path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();
    assert!(names.contains(&"root.txt".to_string()), "{names:?}");
    assert!(names.contains(&"main.rs".to_string()), "{names:?}");
    assert!(
        !names.contains(&"debug.rs".to_string()),
        "target 目录应当跳过：{names:?}"
    );

    // 第二次扫描共用 seen，不会重复收集。
    let before = files.len();
    collect_files(&dir, &mut files, &mut seen);
    assert_eq!(files.len(), before);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reprefix_path_rewrites_files_under_a_renamed_folder() {
    let file = "C:\\proj\\old\\a.vb";
    assert_eq!(
        reprefix_path(file, "C:\\proj\\old", "C:\\proj\\new").as_deref(),
        Some("C:\\proj\\new\\a.vb")
    );
    assert_eq!(
        reprefix_path("C:\\proj\\old", "C:\\proj\\old", "C:\\proj\\new").as_deref(),
        Some("C:\\proj\\new")
    );
    assert_eq!(
        reprefix_path("C:\\proj\\old\\sub\\b.vb", "C:\\proj\\old", "C:\\proj\\new").as_deref(),
        Some("C:\\proj\\new\\sub\\b.vb")
    );
}

#[test]
fn reprefix_path_ignores_similar_prefixes() {
    // 只差前缀的兄弟目录不能被当成子项。
    assert_eq!(reprefix_path("C:\\proj\\older\\a.vb", "C:\\proj\\old", ""), None);
    assert_eq!(reprefix_path("C:\\proj\\oldish", "C:\\proj\\old", ""), None);
    assert_eq!(
        reprefix_path("C:\\proj\\old\\a.vb", "C:\\proj\\old", "").as_deref(),
        Some("\\a.vb"),
        "删除模式下只看是否为 None，具体文本无所谓"
    );
}

#[test]
fn a_file_that_does_not_change_keeps_the_same_stamp() {
    let dir = temp_dir("stamp-same");
    let file = dir.join("a.vb");
    std::fs::write(&file, "x").expect("写文件");

    let first = disk_stamp(&file);
    assert!(first.is_some(), "刚写过的文件应当能读到时间戳");
    assert!(!stamp_differs(first, disk_stamp(&file)));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_stamp_never_counts_as_modified() {
    // 新打开还没记录时间戳、或者文件刚被删掉时都不该弹「外部修改」。
    assert!(!stamp_differs(None, None));
    assert!(!stamp_differs(None, Some(std::time::UNIX_EPOCH)));
    assert!(!stamp_differs(Some(std::time::UNIX_EPOCH), None));
}

#[test]
fn a_different_stamp_means_the_file_changed() {
    let known = std::time::UNIX_EPOCH;
    let current = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1);
    assert!(stamp_differs(Some(known), Some(current)));
}
