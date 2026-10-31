//! Agent2API 官方提供商插件包（库形态）。
//!
//! ── 两种运行形态 ────────────────────────────────────────────
//! 1. **独立进程**（`main.rs`，`bin` feature）：网关 spawn 它，走 stdin/stdout
//!    的 JSON 行 RPC —— 桌面端 / 服务端用，能隔离 provider 崩溃。
//! 2. **进程内嵌入**（本 lib，`staticlib`）：iOS 沙箱禁止 spawn 子进程，把
//!    provider 实现链进主二进制，网关通过注入的内存管道直接调 `serve`。
//!
//! ── 进程内嵌入的入口 ────────────────────────────────────────
//! [`serve_inprocess`] 接收一对已就绪的 tokio 句柄（读端 + 写端），在当前
//! 进程里跑 `serve` 主循环。网关侧用 `tokio::io::duplex` 建内存管道，把两端
//! 分别交给 `serve` 与自己已有的 `Host::read_loop`，即可**不 spawn** 地复用
//! 整套 RPC 协议。
//!
//! 因为是**同进程内两个 Rust crate 直接互相调用**（都编进同一个静态库/二进制），
//! 不需要跨 FFI 边界 —— 直接传 tokio 的 trait object 即可，比 `extern "C"` +
//! 裸 fd 更简洁、更类型安全。

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod beijing;
mod content_block;
mod crypto;
mod dpop;
mod errors;
mod flight;
mod headers;
mod http;
mod ids;
mod json;
mod limits;
mod model_rules;
mod paths;
mod proxy_util;
mod queue;
mod quota;
mod logging;
mod onboarding;
mod providers;
mod readiness;
mod reasoning;
mod response_stream;
mod sse;
mod sse_events;
mod stall;
mod telemetry;
mod store;
mod timeouts;
mod timeutil;
mod workbuddy_shared;

use tokio::io::{AsyncRead, AsyncWrite};

pub use providers::registry;

/// 独立进程形态的入口：建 tokio 运行时并跑 `serve`（stdin/stdout）。
///
/// 这是 `main.rs` 唯一调用的东西，把「建运行时 + 数据目录初始化」收拢在
/// 一处，让 bin 与 lib 共用同一份逻辑。
pub fn run_standalone() {
    agent2api_provider_sdk::serve_blocking_with(
        providers::registry(),
        agent2api_provider_sdk::ServeOptions {
            on_init: Some(|env| paths::init(&env.data_dir)),
            // 独立进程：reader/writer 缺省走 stdin/stdout，shutdown 时 exit。
            ..Default::default()
        },
    );
}

/// 进程内嵌入形态的入口：在**当前进程**里跑 `serve` 主循环。
///
/// ── 参数 ────────────────────────────────────────────────────
/// 接收一对已就绪的 tokio 句柄（读端 + 写端），直接交给 `serve_with`。网关侧
/// 用 `tokio::io::duplex` 建两条内存管道：一端给本函数、另一端接进自己已有的
/// `Host::read_loop`。
///
/// ── 阻塞语义 ────────────────────────────────────────────────
/// 本函数**阻塞**直到主循环退出（管道 EOF 或 `shutdown`）。网关必须在一个
/// 独立 task / 线程里驱动它，不能占住自己的事件循环。`shutdown` 时**不会**
/// `process::exit`（那会杀死宿主 App），只正常返回。
pub async fn serve_inprocess(
    reader: Box<dyn AsyncRead + Unpin + Send + Sync>,
    writer: Box<dyn AsyncWrite + Unpin + Send + Sync>,
) -> std::io::Result<()> {
    let providers = providers::registry();
    let options = agent2api_provider_sdk::ServeOptions {
        on_init: Some(|env| paths::init(&env.data_dir)),
        reader: Some(reader),
        writer: Some(writer),
        in_process: true,
    };
    agent2api_provider_sdk::serve_with(providers, options).await
}
