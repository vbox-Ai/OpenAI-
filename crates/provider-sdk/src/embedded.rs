//! 进程内宿主：`EmbeddedHost`（无 stdio 的 provider 调度基座）。
//!
//! ── 阶段 1.3 要解决什么 ─────────────────────────────────────
//! 桌面端 [`serve`] 是一个读 stdin / 写 stdout 的进程主循环。移动端单进程化后
//! 没有 stdio、没有网关进程，provider 方法要由 FFI 直接调用。这里给出一个
//! **不读 stdin、不写 stdout、不 `process::exit`** 的宿主骨架：
//!
//! - [`EmbeddedHost::new`]：装入 provider + 运行环境 + 直连回调（`CallGateway`）
//! - [`EmbeddedHost::ctx`]：构造 provider 的 [`PluginCtx`]（进程内直连）
//! - [`EmbeddedHost::provider_infos`]：收集握手信息（1.5 接到 FFI 给 Flutter）
//! - [`EmbeddedHost::shutdown`]：优雅停机，绝不 `exit`（否则杀掉 APP）
//!
//! 具体 provider 方法调度（chat / models / usage / checkin / login 等）
//! 在 1.5 / 1.6 接到 FFI 时补在这里。

use std::sync::Arc;

use agent2api_provider_protocol as proto;
use serde_json::Value;

use crate::ctx::{CallGateway, PluginCtx};
use crate::dispatch::dispatch_control;
use crate::provider::{BoxProvider, ConversationSink, Provider, SinkInner};

/// 进程内宿主：持有 provider + 运行环境 + 直连回调，供 FFI 调用。
pub struct EmbeddedHost {
    providers: Vec<Arc<dyn Provider>>,
    env: proto::HostEnv,
    call: Arc<dyn CallGateway>,
}

impl EmbeddedHost {
    /// 构造一个进程内宿主。
    pub fn new(
        providers: Vec<BoxProvider>,
        env: proto::HostEnv,
        call: Arc<dyn CallGateway>,
    ) -> Arc<Self> {
        Arc::new(Self {
            providers: providers.into_iter().map(Arc::from).collect(),
            env,
            call,
        })
    }

    /// 构造一个 provider 的 [`PluginCtx`]（进程内：env + 直连 call）。
    pub fn ctx(&self, _provider: &str) -> PluginCtx {
        PluginCtx {
            env: self.env.clone(),
            call: self.call.clone(),
        }
    }

    /// 收集所有 provider 的握手信息（与 `serve` 的 `handle_init` 同源）。
    pub fn provider_infos(&self) -> Vec<proto::ProviderInfo> {
        self.providers
            .iter()
            .map(|provider| {
                let meta = provider.meta();
                let (refusal_finishes, refusal_markers) = provider.refusal_markers();
                proto::ProviderInfo {
                    id: proto::ProviderId::new(meta.id),
                    label: meta.label,
                    order: meta.order,
                    capabilities: meta.capabilities,
                    login_methods: meta.login_methods,
                    default_models: meta.default_models,
                    needs_proxy: false,
                    refusal_finishes,
                    refusal_markers,
                    admin_panels: meta.admin_panels,
                    account_features: meta.account_features,
                    checkin: meta.checkin,
                }
            })
            .collect()
    }

    /// 分发一个控制面方法（一问一答），返回结果或错误消息。
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let provider_id = params.get("provider").and_then(Value::as_str).unwrap_or("");
        let Some(provider) = self
            .providers
            .iter()
            .find(|p| p.provider_id() == provider_id)
            .cloned()
        else {
            return Err("params.provider 缺失或未知".to_string());
        };
        let ctx = self.ctx(&provider.provider_id());
        let account = params
            .get("account")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        dispatch_control(provider.as_ref(), &ctx, &account, method, params).await
    }

    /// 流式对话：把每个 chunk 推给 `sink`（实现 [`SinkInner`]）。
    pub async fn conversation(&self, params: Value, sink: &mut dyn SinkInner) -> Result<(), String> {
        let provider_id = params.get("provider").and_then(Value::as_str).unwrap_or("");
        let Some(provider) = self
            .providers
            .iter()
            .find(|p| p.provider_id() == provider_id)
            .cloned()
        else {
            return Err("params.provider 缺失或未知".to_string());
        };
        let req: proto::ConversationRequest = serde_json::from_value(params)
            .map_err(|e| format!("conversation 参数无法解析: {e}"))?;
        let ctx = self.ctx(&provider.provider_id());
        let view = ConversationSink::new(sink);
        provider.conversation(&ctx, req, view).await
    }

    /// 优雅停机：进程内模式直接返回，绝不 `process::exit`。
    pub fn shutdown(&self) {
        // 1.5 / 1.6 接入在途流取消时在这里补。
    }
}
