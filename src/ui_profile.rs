//! 界面帧耗时采样：排查「拖动分割条时卡顿」这类问题时用。
//!
//! gpui 是即时模式：每次重绘都会重新跑一遍所有可见面板的 `render`，所以拖动分割条
//! （每帧都在重绘）会把每个面板的每帧开销放大成卡顿。这里给各个 `render` 挂上轻量
//! 计时，只在设了 `QIDE_UI_PROFILE=1` 时记录，并把「超过阈值」的慢帧追加到
//! `%APPDATA%\gpui-first\ui-profile.log`（超过 512 KB 重开一个）。
//!
//! 用法：`$env:QIDE_UI_PROFILE=1; .\first.exe` → 拖一次分割条 → 看日志里哪一段最贵。

use std::cell::RefCell;
use std::io::Write as _;
use std::sync::OnceLock;
use std::time::Instant;

/// 单段 `render` 超过这个耗时就记一笔。
const SLOW_SPAN: f64 = 8.0;
/// 帧间隔（两次重绘之间）超过这个值就算「这一帧画得太慢」：60 Hz 是 16.7 ms，
/// 20 ms 相当于掉到 50 fps 以下，拖动时能明显感到跟手性变差。
const SLOW_PERIOD: f64 = 20.0;
/// 间隔超过这个值基本是空闲（没在连续重绘），不用记。
const IDLE_PERIOD: f64 = 200.0;
/// 日志文件的上限，超过就清空重写。
const MAX_LOG_BYTES: u64 = 512 * 1024;

/// 一段 `render` 的计时守卫：离开作用域时把耗时记进当前帧。
pub struct Span {
    name: &'static str,
    started: Option<Instant>,
}

struct Frame {
    started: Instant,
    spans: Vec<(&'static str, f64)>,
    /// 第一帧没有「上一帧结束时间」可比，只用来起算。
    primed: bool,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            spans: Vec::new(),
            primed: false,
        }
    }
}

thread_local! {
    static FRAME: RefCell<Frame> = RefCell::new(Frame::default());
}

/// 是否开着采样（`QIDE_UI_PROFILE=1`，或者开了拖动复现探针）。
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        probe_enabled()
            || std::env::var_os("QIDE_UI_PROFILE").is_some_and(|value| {
                let text = value.to_string_lossy();
                text == "1" || text.eq_ignore_ascii_case("true")
            })
    })
}

/// 给一段 `render` 计时；离开作用域时把耗时记进当前帧。
pub fn span(name: &'static str) -> Span {
    Span {
        name,
        started: enabled().then(Instant::now),
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(started) = self.started else {
            return;
        };
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        FRAME.with(|frame| frame.borrow_mut().spans.push((self.name, elapsed)));
    }
}

/// 复现探针的模式（`QIDE_RESIZE_PROBE`）：`1` 改尺寸、`notify` 只重绘、其它值不开。
pub fn probe_mode() -> Option<&'static str> {
    match std::env::var("QIDE_RESIZE_PROBE") {
        Ok(value) if value == "1" || value.eq_ignore_ascii_case("true") => Some("resize"),
        Ok(value) if value.eq_ignore_ascii_case("notify") => Some("notify"),
        _ => None,
    }
}

/// 是否开着「拖动分割条」的复现探针。
pub fn probe_enabled() -> bool {
    probe_mode().is_some()
}

/// 一帧的统计结果（纯数据，方便单测）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameReport {
    /// 距离上一次帧结束的间隔：连续重绘（拖动分割条）时约等于一帧的周期，
    /// 空闲时会是几百毫秒，只用来说明「当时是不是在连续重绘」。
    pub gap_ms: f64,
    pub spans: Vec<(&'static str, f64)>,
}

impl FrameReport {
    /// 是否值得写进日志：有单段 `render` 超阈值，或者这一帧本身画得慢
    /// （连续重绘时间隔超过 `SLOW_PERIOD`；空闲间隔不算）。
    pub fn is_slow(&self) -> bool {
        let busy = self.gap_ms >= SLOW_PERIOD && self.gap_ms <= IDLE_PERIOD;
        busy || self.spans.iter().any(|(_, ms)| *ms >= SLOW_SPAN)
    }

    /// 写进日志的一行：帧间隔 + 各段耗时（从大到小）。
    pub fn line(&self) -> String {
        let mut spans = self.spans.clone();
        spans.sort_by(|left, right| right.1.total_cmp(&left.1));
        let detail: Vec<String> = spans
            .iter()
            .map(|(name, ms)| format!("{name}={ms:.1}ms"))
            .collect();
        format!("间隔 {:.0}ms | {}", self.gap_ms, detail.join(" "))
    }
}

/// 帧结束：整帧耗时超过阈值时写一行日志，并开始统计下一帧。
pub fn end_frame() {
    if !enabled() {
        return;
    }
    let report = FRAME.with(|frame| {
        let mut frame = frame.borrow_mut();
        if !frame.primed {
            frame.primed = true;
            *frame = Frame {
                primed: true,
                ..Frame::default()
            };
            return None;
        }
        let report = FrameReport {
            gap_ms: frame.started.elapsed().as_secs_f64() * 1000.0,
            spans: std::mem::take(&mut frame.spans),
        };
        *frame = Frame {
            primed: true,
            ..Frame::default()
        };
        Some(report)
    });
    if let Some(report) = report
        && report.is_slow()
    {
        append_line(&report.line());
    }
}

fn log_path() -> Option<std::path::PathBuf> {
    // 测试可以指定别的位置；正常运行时写在崩溃日志旁边。
    if let Some(path) = std::env::var_os("QIDE_UI_PROFILE_LOG") {
        return Some(std::path::PathBuf::from(path));
    }
    let appdata = std::env::var_os("APPDATA")?;
    let mut path = std::path::PathBuf::from(appdata);
    path.push("gpui-first");
    std::fs::create_dir_all(&path).ok()?;
    path.push("ui-profile.log");
    Some(path)
}

fn append_line(line: &str) {
    if let Some(path) = log_path() {
        append_to(&path, line);
    }
}

/// 往日志里追加一行；超过上限就先清空再写，免得无限长。
fn append_to(path: &std::path::Path, line: &str) {
    if std::fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES) {
        std::fs::remove_file(path).ok();
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_frame_detection_covers_single_span() {
        let quick = FrameReport {
            gap_ms: 16.0,
            spans: vec![("output", 1.2)],
        };
        assert!(!quick.is_slow(), "各段都很快、间隔也正常时不该记录");

        let heavy_span = FrameReport {
            gap_ms: 16.0,
            spans: vec![("search", 9.0)],
        };
        assert!(heavy_span.is_slow(), "单段超阈值就要记录");

        let slow_period = FrameReport {
            gap_ms: 33.0,
            spans: vec![("frame", 4.0)],
        };
        assert!(
            slow_period.is_slow(),
            "各段都小但整帧 33ms（掉到 30fps）也要记录：开销在排版/绘制里"
        );

        let idle_gap = FrameReport {
            gap_ms: 800.0,
            spans: vec![("output", 0.5)],
        };
        assert!(!idle_gap.is_slow(), "空闲间隔不算慢帧");
    }

    #[test]
    fn report_line_sorts_spans_by_cost() {
        let report = FrameReport {
            gap_ms: 16.0,
            spans: vec![("status", 0.4), ("output", 7.0), ("search", 1.5)],
        };
        let line = report.line();
        assert!(line.starts_with("间隔 16ms | "), "{line}");
        assert!(
            line.contains("output=7.0ms") && line.ends_with("status=0.4ms"),
            "耗时的段排前面：{line}"
        );
    }

    #[test]
    fn spans_are_ignored_when_profiling_is_off() {
        // 没设环境变量时 `span` 是空操作，不会攒下数据。
        let guard = span("test");
        drop(guard);
        FRAME.with(|frame| {
            let frame = frame.borrow();
            assert!(frame.spans.is_empty() || enabled());
        });
    }

    #[test]
    fn slow_frames_are_appended_to_the_log_file() {
        let dir = std::env::temp_dir().join(format!("qide-ui-profile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let path = dir.join("ui-profile.log");
        std::fs::remove_file(&path).ok();

        append_to(&path, "16.0ms | output=12.0ms");
        append_to(&path, "9.0ms | search=8.5ms");
        let text = std::fs::read_to_string(&path).expect("日志应当写出来");
        assert_eq!(text.lines().count(), 2, "两次追加应当各占一行：{text}");
        assert!(text.contains("output=12.0ms"), "{text}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
