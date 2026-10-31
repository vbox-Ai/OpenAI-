use agent2api_provider_protocol as proto;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::provider::BoxProvider;

/// 可替换的读端（默认 stdin，进程内模式时换成内存管道读端）。
///
/// `+ Sync`：`serve` 把这些句柄包进 `Mutex` 跨 task 共享，而 `Mutex<T>` 的
/// `Send + Sync` 要求 `T: Send`。`tokio::io::Stdout` / `DuplexStream` 都是
/// `Send + Sync`，加这个约束让 trait object 与它们一致。
pub type InjectedReader = Box<dyn AsyncRead + Unpin + Send + Sync>;
/// 可替换的写端（默认 stdout，进程内模式时换成内存管道写端）。
pub type InjectedWriter = Box<dyn AsyncWrite + Unpin + Send + Sync>;

#[derive(Default)]
pub struct ServeOptions {
    /// 在握手应答前初始化机器级资源；失败时拒绝握手。
    pub on_init: Option<fn(&proto::HostEnv) -> std::io::Result<()>>,
    /// 读端：`None` 时用 `tokio::io::stdin()`（独立进程的默认形态）。
    /// 进程内嵌入时注入内存管道读端，从而**不 spawn 子进程**也能跑同一套主循环。
    pub reader: Option<InjectedReader>,
    /// 写端：`None` 时用 `tokio::io::stdout()`。
    pub writer: Option<InjectedWriter>,
    /// 进程内模式：`shutdown` 时不调用 `std::process::exit`（会杀死宿主 App），
    /// 只正常返回。独立进程模式保持 `true`（exit 让网关感知子进程退出）。
    pub in_process: bool,
}

pub fn serve_blocking(providers: Vec<BoxProvider>) {
    serve_blocking_with(providers, ServeOptions::default());
}

pub fn serve_blocking_with(providers: Vec<BoxProvider>, options: ServeOptions) {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("provider host: 建 tokio 运行时失败: {e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = runtime.block_on(crate::serve::serve_with(providers, options)) {
        eprintln!("provider host: 主循环异常退出: {e}");
        std::process::exit(1);
    }
}
