//! Agent2API 官方提供商插件包 —— 库目标（FFI 入口）。
//!
//! 阶段 1.1：从纯二进制拆出库目标，供 iOS / Android 静态链接。
//! 桌面端仍走 `main.rs`（薄包装，调用本库的 [`run`]）。
//!
//! 模块声明与二进制入口共用这一份 —— 库与二进制是同一套代码，
//! 只是目标类型不同（staticlib / cdylib / rlib vs bin）。

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// ── 关于上面这条 deny ───────────────────────────────────────
// 插件进程崩溃不会带走网关，但会打断所有在途请求。所以这里比网关更严：
// 连 `unwrap` 都不允许。确实需要的地方用 `guard()` 显式兜住并记日志。

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
mod sse;
mod sse_events;
mod stall;
mod telemetry;
mod store;
mod timeouts;
mod timeutil;
mod workbuddy_shared;

/// 桌面端入口：起 stdio 主循环（`main.rs` 调用）。
pub fn run() {
    agent2api_provider_sdk::serve_blocking(providers::registry());
}

/// FFI 探活（阶段 1.1 最小骨架：验证能编成 staticlib / cdylib）。
///
/// 后续阶段会扩展为完整的 `agent2api_providers_*` 系列
/// （init / provider_infos / login / chat / conversation / models / usage /
/// checkin / account_* 等）。
#[no_mangle]
pub extern "C" fn agent2api_providers_ping() -> i32 {
    0
}
