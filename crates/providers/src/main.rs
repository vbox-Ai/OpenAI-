//! Agent2API 官方提供商插件包 —— 二进制入口。
//!
//! 阶段 1.1 起，这里只是库目标（`lib.rs`）的薄包装：真正的模块与主循环
//! 都在库里。桌面端 `cargo build` 产出这个可执行文件；iOS / Android
//! 静态链接走库目标（`libagent2api_providers`）。

fn main() {
    agent2api_providers::run();
}
