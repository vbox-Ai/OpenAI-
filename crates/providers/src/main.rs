//! Agent2API 官方提供商插件包（独立进程入口）。
//!
//! 真正的逻辑在 `lib.rs`：`run_standalone()` 建运行时并跑 `serve`。
//! 本文件只剩一个 `main`，把 bin 与 lib 解耦 —— lib 同时服务「独立进程」
//! 与「进程内嵌入（iOS 静态库）」两种形态。

fn main() {
    agent2api_providers::run_standalone();
}
