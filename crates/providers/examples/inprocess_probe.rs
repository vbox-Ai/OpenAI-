//! 进程内嵌入验证探针：证明 `serve_inprocess` 能不经 spawn、在内存管道上完成握手。
//!
//! 运行方式（在插件仓库根目录）：
//! ```bash
//! cargo run --example inprocess_probe --no-default-features --features workbuddy,raccoon
//! ```
//!
//! 它做的事：
//! 1. 用 `tokio::io::duplex` 建两对内存管道（模拟网关 ↔ 插件的 stdio）；
//! 2. 在一个 task 里跑 `agent2api_providers::serve_inprocess(plugin_reader, plugin_writer)`；
//! 3. 主 task 用管道另一端发 `INIT` 握手，等 `serve` 回 provider 列表；
//! 4. 校验列表里包含 `workbuddy`，打印结果后发 `shutdown` 收尾。
//!
//! 通过即证明：进程内跑 provider 的整条 RPC 链路（帧编解码 + 握手 + provider 注册）
//! 在**不 spawn 子进程**的前提下完全可用 —— 这正是 iOS 沙箱场景要的。

use std::time::Duration;

use agent2api_provider_protocol as proto;
use serde_json::json;

#[tokio::main]
async fn main() {
    // 两对内存管道：一对给「网关→插件」（读端给 serve，写端留网关），
    // 一对给「插件→网关」（写端给 serve，读端留网关）。
    // 注意 duplex 的语义：`duplex(n)` 返回 `(a, b)`，a 写的数据 b 能读到，反之亦然。
    let (gw_to_plugin, plugin_from_gw) = tokio::io::duplex(1 << 20);
    let (plugin_to_gw, gw_from_plugin) = tokio::io::duplex(1 << 20);

    // serve 需要「读端」和「写端」：
    //   读端 = plugin_from_gw（从网关读请求）
    //   写端 = plugin_to_gw（写应答给网关）
    let serve_task = tokio::spawn(async move {
        let result = agent2api_providers::serve_inprocess(
            Box::new(plugin_from_gw),
            Box::new(plugin_to_gw),
        )
        .await;
        eprintln!("[serve_inprocess 退出] {result:?}");
        result
    });

    // 网关侧：写端 = gw_to_plugin，读端 = gw_from_plugin
    let mut writer = gw_to_plugin;
    let mut reader = proto::frame::buffered(gw_from_plugin);

    // 发 INIT 握手
    let init_env = proto::HostEnv {
        data_dir: std::env::temp_dir().to_string_lossy().into_owned(),
        host_version: "poc".into(),
        target: "aarch64-apple-ios".into(),
        proxy: String::new(),
        debug_traffic: false,
        loopback_port: 0,
    };
    let init_frame = proto::Frame {
        id: 1,
        method: Some(proto::methods::gateway::INIT.to_string()),
        params: Some(json!({ "env": init_env })),
        ..Default::default()
    };
    proto::write_frame(&mut writer, &init_frame).await.expect("写 INIT 失败");

    // 等 INIT 应答
    let reply = tokio::time::timeout(Duration::from_secs(10), proto::read_frame(&mut reader))
        .await
        .expect("等 INIT 应答超时")
        .expect("读 INIT 应答失败")
        .expect("INIT 应答是 EOF");

    assert_eq!(reply.id, 1, "INIT 应答 id 不匹配");
    assert!(reply.error.is_none(), "INIT 握手失败: {:?}", reply.error);

    let infos: Vec<proto::ProviderInfo> =
        serde_json::from_value(reply.result.expect("INIT 应答缺 result"))
            .expect("provider 列表无法解析");

    println!("=== 握手成功，收到 {} 家 provider ===", infos.len());
    for info in &infos {
        println!("  - id={} label={}", info.id.as_str(), info.label);
    }

    let has_workbuddy = infos.iter().any(|i| i.id.as_str() == "workbuddy");
    assert!(has_workbuddy, "provider 列表里没有 workbuddy！");

    // 发 PING 再收一次，验证双向通信稳定
    let ping = proto::Frame {
        id: 2,
        method: Some(proto::methods::gateway::PING.to_string()),
        params: Some(json!({})),
        ..Default::default()
    };
    proto::write_frame(&mut writer, &ping).await.expect("写 PING 失败");
    let pong = tokio::time::timeout(Duration::from_secs(5), proto::read_frame(&mut reader))
        .await
        .expect("等 PONG 超时")
        .expect("读 PONG 失败")
        .expect("PONG 是 EOF");
    assert_eq!(pong.id, 2);
    assert!(pong.error.is_none(), "PING 失败: {:?}", pong.error);

    // 发 shutdown（进程内模式：serve 不 exit，只返回；靠写端关闭触发 EOF 退出）
    let shutdown = proto::Frame {
        id: 3,
        method: Some(proto::methods::gateway::SHUTDOWN.to_string()),
        params: Some(json!({})),
        ..Default::default()
    };
    proto::write_frame(&mut writer, &shutdown).await.expect("写 shutdown 失败");

    // 关键：drop 写端，让 serve 的读端读到 EOF 从而退出主循环。
    // 进程内模式没有 process::exit，退出靠「网关关闭写端 → serve 读 EOF」。
    drop(writer);

    // 等 serve task 正常退出（进程内模式 shutdown 后返回 Ok）
    let serve_result = tokio::time::timeout(Duration::from_secs(10), serve_task)
        .await
        .expect("serve task 未在 shutdown 后退出");
    match serve_result {
        Ok(Ok(())) => println!("✅ serve_inprocess 正常退出"),
        Ok(Err(e)) => panic!("serve_inprocess 异常退出: {e}"),
        Err(e) => panic!("serve task join 失败: {e}"),
    }

    println!("✅ POC 验证通过：进程内 serve 握手 + PING + shutdown 全链路可用");
}
