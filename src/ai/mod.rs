//! AI 编程功能。
//!
//! 按「界面 / 网络 / 协议」分块，方便以后整体搬到别的程序里：
//!
//! - [`client`]：与界面无关的网络层。地址解析、请求体拼装、WinHTTP 调用，
//!   以及把流式响应一段段吐出来的回调接口；
//! - [`stream`]：与界面无关的 SSE 增量解析（纯逻辑，可单测）；
//! - [`tools`]：与界面无关的工具层：工具清单 + 把一次调用算成只读结果或待写入文件；
//! - [`markdown`]：与界面无关的极简 Markdown 分段（说明文字 / 围栏代码块）；
//! - [`panel`]：Q-IDE 的 GPUI 面板。换成别的界面框架时，只需要重写这一层，
//!   前面几块不用动。
//!
//! 隐私约定：API Key 只在 [`client`] 里拼进 `Authorization` 头，不写日志、
//! 不进错误信息，缓冲区用完立刻清零。

pub mod client;
pub mod markdown;
pub mod panel;
pub mod stream;
pub mod tools;
