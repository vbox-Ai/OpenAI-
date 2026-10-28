//! FFI 边界（阶段 1.4 / 1.5）：把进程内宿主暴露给 Flutter（经 flutter_rust_bridge）。
//!
//! ── 字符串契约 ──────────────────────────────────────────────
//! Rust 返回的字符串一律是 `CString::into_raw` 出来的 C 字符串，调用方读完
//! 必须调 [`agent2api_providers_string_free`] 释放。这是手写 FFI 的标准做法。

use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use agent2api_provider_protocol as proto;
use agent2api_provider_sdk::provider::SinkInner;
use agent2api_provider_sdk::serve::Cancel;
use agent2api_provider_sdk::{EmbeddedHost, SendFuture};
use serde_json::Value;

use crate::embedded::memory_gateway;
use crate::providers;

static HOST: OnceLock<Arc<EmbeddedHost>> = OnceLock::new();
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static NEXT_CONV_ID: AtomicU64 = AtomicU64::new(1);
static CONVERSATIONS: Mutex<HashMap<u64, Arc<Cancel>>> = Mutex::new(HashMap::new());

/// 初始化进程内宿主。返回 0 = 成功，非 0 = 失败。
///
/// - `data_dir`：数据目录（iOS 沙盒路径）
/// - `target`：目标三元组（如 `aarch64-apple-ios`）
/// - `loopback_port`：OAuth 回调端口（0 = 未知）
#[no_mangle]
pub extern "C" fn agent2api_providers_init(
    data_dir: *const c_char,
    target: *const c_char,
    loopback_port: u16,
) -> i32 {
    let data_dir = unsafe { cstr_to_string(data_dir) };
    let target = unsafe { cstr_to_string(target) };
    let (Some(data_dir), Some(target)) = (data_dir, target) else {
        return 1;
    };
    let env = proto::HostEnv {
        data_dir,
        host_version: env!("CARGO_PKG_VERSION").to_string(),
        target,
        proxy: String::new(),
        debug_traffic: false,
        loopback_port,
    };
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(_) => return 3,
    };
    if RUNTIME.set(runtime).is_err() {
        return 4;
    }
    let host = EmbeddedHost::new(providers::registry(), env, memory_gateway());
    if HOST.set(host).is_err() {
        return 2; // 已初始化过
    }
    0
}

/// 返回所有 provider 的握手信息（JSON 字符串），失败返回 null。
/// 调用方读完必须调 [`agent2api_providers_string_free`] 释放。
#[no_mangle]
pub extern "C" fn agent2api_providers_provider_infos() -> *mut c_char {
    let Some(host) = HOST.get() else {
        return std::ptr::null_mut();
    };
    let infos = host.provider_infos();
    match serde_json::to_string(&infos) {
        Ok(json) => CString::new(json)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(_) => std::ptr::null_mut(),
    }
}

/// 释放 [`agent2api_providers_provider_infos`] 等返回的 C 字符串。
#[no_mangle]
pub unsafe extern "C" fn agent2api_providers_string_free(s: *mut c_char) {
    if !s.is_null() {
        let _ = CString::from_raw(s);
    }
}

/// 通用控制面调用：`method` + `params_json`(JSON) → 结果信封(JSON)。
///
/// 返回 `{"ok":true,"result":<value>}` 或 `{"ok":false,"error":"<msg>"}`。
/// 调用方读完必须调 [`agent2api_providers_string_free`] 释放。
#[no_mangle]
pub extern "C" fn agent2api_providers_call(
    method: *const c_char,
    params_json: *const c_char,
) -> *mut c_char {
    let Some(host) = HOST.get() else {
        return json_result_to_c(Err("宿主未初始化".to_string()));
    };
    let method = unsafe { cstr_to_string(method) };
    let params_json = unsafe { cstr_to_string(params_json) };
    let (Some(method), Some(params_json)) = (method, params_json) else {
        return json_result_to_c(Err("参数非法（空指针）".to_string()));
    };
    let params: Value = match serde_json::from_str(&params_json) {
        Ok(v) => v,
        Err(e) => return json_result_to_c(Err(format!("params JSON 解析失败: {e}"))),
    };
    let Some(runtime) = RUNTIME.get() else {
        return json_result_to_c(Err("运行时未初始化".to_string()));
    };
    let result = runtime.block_on(host.call(&method, params));
    json_result_to_c(result)
}

/// 把 `Result<Value, String>` 包成 JSON 信封，再转 C 字符串。
fn json_result_to_c(result: Result<Value, String>) -> *mut c_char {
    let envelope = match result {
        Ok(v) => serde_json::json!({ "ok": true, "result": v }),
        Err(e) => serde_json::json!({ "ok": false, "error": e }),
    };
    match serde_json::to_string(&envelope) {
        Ok(s) => CString::new(s).map(|c| c.into_raw()).unwrap_or(std::ptr::null_mut()),
        Err(_) => std::ptr::null_mut(),
    }
}

/// 优雅停机（进程内模式绝不 `process::exit`）。
#[no_mangle]
pub extern "C" fn agent2api_providers_shutdown() {
    if let Some(host) = HOST.get() {
        host.shutdown();
    }
}

/// 流式 chunk 回调（长度前缀，防 NUL 字节）。
pub type ConversationChunkCallback = unsafe extern "C" fn(*const u8, usize, *mut c_void);
/// 流式完成 / 错误回调。
pub type ConversationDoneCallback = unsafe extern "C" fn(i32, *const c_char, *mut c_void);

/// 把 chunk 转发给 C 回调的 sink（进程内流式对话用）。
struct FfiSink {
    on_head: ConversationChunkCallback,
    on_chunk: ConversationChunkCallback,
    user_data: *mut c_void,
}

impl SinkInner for FfiSink {
    fn send_head<'a>(&'a mut self, head: proto::StreamHead) -> SendFuture<'a, ()> {
        let cb = self.on_head;
        let ud = self.user_data;
        Box::pin(async move {
            if let Ok(json) = serde_json::to_vec(&head) {
                unsafe { cb(json.as_ptr(), json.len(), ud) };
            }
            Ok(())
        })
    }
    fn send_chunk<'a>(&'a mut self, bytes: &'a [u8]) -> SendFuture<'a, ()> {
        let cb = self.on_chunk;
        let ud = self.user_data;
        Box::pin(async move {
            unsafe { cb(bytes.as_ptr(), bytes.len(), ud) };
            Ok(())
        })
    }
}

/// 发起流式对话，返回会话 id（0 = 失败）。
///
/// `params_json`：`proto::ConversationRequest` 的 JSON。
/// 每个 chunk 经 `on_head` / `on_chunk` 回调，结束经 `on_done` 回调。
#[no_mangle]
pub extern "C" fn agent2api_providers_conversation_start(
    params_json: *const c_char,
    on_head: Option<ConversationChunkCallback>,
    on_chunk: Option<ConversationChunkCallback>,
    on_done: Option<ConversationDoneCallback>,
    user_data: *mut c_void,
) -> u64 {
    let Some(host) = HOST.get().cloned() else {
        return 0;
    };
    let Some(params_json) = (unsafe { cstr_to_string(params_json) }) else {
        return 0;
    };
    let Ok(params) = serde_json::from_str::<Value>(&params_json) else {
        return 0;
    };
    let (Some(on_head), Some(on_chunk), Some(on_done)) = (on_head, on_chunk, on_done) else {
        return 0;
    };

    let id = NEXT_CONV_ID.fetch_add(1, Ordering::Relaxed);
    let cancel = Arc::new(Cancel::default());
    if let Ok(mut map) = CONVERSATIONS.lock() {
        map.insert(id, cancel);
    }

    let Some(runtime) = RUNTIME.get() else {
        return 0;
    };
    runtime.spawn(async move {
        let mut sink = FfiSink {
            on_head,
            on_chunk,
            user_data,
        };
        let result = host.conversation(params, &mut sink).await;
        if let Ok(mut map) = CONVERSATIONS.lock() {
            map.remove(&id);
        }
        match result {
            Ok(()) => unsafe { on_done(1, std::ptr::null(), user_data) },
            Err(e) => {
                let cmsg = CString::new(e).unwrap_or_default();
                unsafe { on_done(0, cmsg.as_ptr(), user_data) };
            }
        }
    });
    id
}

/// 取消一条在途流式对话。
#[no_mangle]
pub extern "C" fn agent2api_providers_conversation_cancel(id: u64) {
    if let Ok(map) = CONVERSATIONS.lock() {
        if let Some(cancel) = map.get(&id) {
            cancel.cancel();
        }
    }
}

/// 把 C 字符串转成 String（空指针 / 非法 UTF-8 → None）。
unsafe fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string())
}
