//! 控制面分发（一问一答），`serve`（stdio）与 `EmbeddedHost`（进程内）共用。
//!
//! 返回 `Result<Value, String>`，不负责回复 —— 回复方式由调用方决定：
//! `serve` 把它映射成 WireError 写回 stdout，`EmbeddedHost` 把它包进 FFI JSON。

use agent2api_provider_protocol as proto;
use serde_json::Value;

use crate::ctx::PluginCtx;
use crate::provider::Provider;

/// 控制面分发：按方法名路由到 provider 的对应操作，返回结果。
pub(crate) async fn dispatch_control(
    provider: &dyn Provider,
    ctx: &PluginCtx,
    account: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    match method {
        crate::provider::SCHEDULES_LIST_METHOD => {
            serde_json::to_value(provider.schedules())
                .map_err(|error| format!("排程申报无法序列化: {error}"))
        }
        proto::methods::gateway::CHAT => {
            match serde_json::from_value::<proto::ChatRequest>(params.clone()) {
                Ok(req) => match provider.build_chat(ctx, req).await {
                    Ok(plan) => Ok(serde_json::json!({
                        "url": plan.url,
                        "headers": plan.headers,
                        "body": plan.body,
                        "upstream": plan.upstream,
                    })),
                    Err(e) => Err(e),
                },
                Err(e) => Err(format!("chat 参数无法解析: {e}")),
            }
        }
        crate::images::CAPABILITIES_METHOD => {
            Ok(serde_json::json!({
                "generation": provider.supports_image_generation(),
                "editing": provider.supports_image_editing(),
            }))
        }
        crate::images::GENERATE_METHOD | crate::images::EDIT_METHOD => {
            match serde_json::from_value::<crate::images::ImageRequest>(params.clone()) {
                Ok(req) => {
                    let result = if method == crate::images::EDIT_METHOD {
                        provider.build_image_edit(ctx, req).await
                    } else {
                        provider.build_image(ctx, req).await
                    };
                    result.and_then(|plan| serde_json::to_value(plan)
                        .map_err(|error| format!("图片请求计划无法序列化: {error}")))
                }
                Err(error) => Err(crate::control_error(
                    400, format!("图片请求参数无法解析: {error}"), None, None,
                )),
            }
        }
        proto::methods::gateway::CLASSIFY => {
            let status = params.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
            let body = params.get("body").cloned().unwrap_or(Value::Null);
            serde_json::to_value(provider.classify(status, &body))
                .map_err(|e| format!("分类结果无法序列化: {e}"))
        }
        proto::methods::gateway::IS_ERROR_RESPONSE => {
            let status = params.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
            let headers: std::collections::BTreeMap<String, String> = params
                .get("headers")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            Ok(Value::Bool(provider.is_error_response(status, &headers)))
        }
        proto::methods::gateway::REFUSAL_MARKERS => {
            let (finishes, markers) = provider.refusal_markers();
            Ok(serde_json::json!({ "finishes": finishes, "markers": markers }))
        }
        crate::retry::RETRY_DECISION_METHOD => {
            let status = params.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
            let body = params.get("body").cloned().unwrap_or(Value::Null);
            let attempt = params.get("attempt").and_then(Value::as_u64).unwrap_or(0) as usize;
            let budget = params.get("budget").and_then(Value::as_u64).unwrap_or(0) as usize;
            serde_json::to_value(provider.retry_decision(status, &body, attempt, budget))
                .map_err(|error| format!("重试决定无法序列化: {error}"))
        }
        proto::methods::gateway::RETRY_ADVICE => {
            let body = params.get("body").cloned().unwrap_or(Value::Null);
            let attempt = params.get("attempt").and_then(Value::as_u64).unwrap_or(0) as usize;
            let budget = params.get("budget").and_then(Value::as_u64).unwrap_or(0) as usize;
            match provider.retry_advice(&body, attempt, budget) {
                Some(advice) => serde_json::to_value(advice)
                    .map_err(|e| format!("退避建议无法序列化: {e}")),
                None => Ok(Value::Null),
            }
        }
        proto::methods::gateway::REASONING_PATCH => {
            let level = params.get("level").and_then(Value::as_str).unwrap_or("");
            let model = params.get("model").and_then(Value::as_str).unwrap_or("");
            let body = params.get("body").cloned().unwrap_or(Value::Null);
            serde_json::to_value(provider.reasoning_patch(level, model, &body))
                .map_err(|e| format!("档位结果无法序列化: {e}"))
        }
        proto::methods::gateway::COOLDOWN_MODELS => {
            let wire = params.get("wire_model").and_then(Value::as_str).unwrap_or("");
            serde_json::to_value(provider.cooldown_models(account, wire))
                .map_err(|e| format!("冷却结果无法序列化: {e}"))
        }
        proto::methods::gateway::TOKEN_ENSURE => {
            provider.ensure_token(ctx, account).await.map(Value::String)
        }
        proto::methods::gateway::TOKEN_REFRESH => {
            provider.refresh_token(ctx, account).await.map(Value::String)
        }
        proto::methods::gateway::TOKEN_EXPIRING => {
            provider.token_expiring(ctx, account).await.map(Value::Bool)
        }
        proto::methods::gateway::MODELS_LIST => provider.models(ctx, account).await.and_then(
            |list| serde_json::to_value(list).map_err(|e| format!("清单无法序列化: {e}")),
        ),
        proto::methods::gateway::MODELS_REFRESH => {
            let force = params.get("force").and_then(Value::as_bool).unwrap_or(false);
            provider.refresh_models(ctx, account, force).await.and_then(|outcome| {
                serde_json::to_value(outcome).map_err(|e| format!("刷新结果无法序列化: {e}"))
            })
        }
        proto::methods::gateway::USAGE_QUERY => provider.usage(ctx, account).await.map(|mut report| {
            let sources = provider.quota_sources(&report);
            if let Some(object) = report.as_object_mut() {
                object.insert("quotaSources".to_string(), Value::Array(sources));
            }
            report
        }),
        proto::methods::gateway::CHECKIN_CLAIM => {
            let manual = params.get("manual").and_then(Value::as_bool).unwrap_or(false);
            crate::checkin::run(provider, ctx, account, manual).await.and_then(
                |report| serde_json::to_value(report).map_err(|e| format!("签到结果无法序列化: {e}")),
            )
        }
        proto::methods::gateway::CLAIM => {
            let claim = params.get("claim").and_then(Value::as_str).unwrap_or("");
            provider.claim(ctx, account, claim).await
        }
        proto::methods::gateway::CLAIM_STATUS => {
            let refresh = params.get("refresh").and_then(Value::as_bool).unwrap_or(false);
            provider.claim_status(ctx, account, refresh).await
        }
        proto::methods::gateway::ADMIN_ACTION => {
            let action = params.get("action").and_then(Value::as_str).unwrap_or("");
            if action.trim().is_empty() {
                Err("admin.action 缺少 action".to_string())
            } else {
                let payload = params.get("payload").cloned().unwrap_or(Value::Null);
                provider.admin_action(ctx, account, action, &payload).await
            }
        }
        proto::methods::gateway::ACCOUNT_NORMALIZE => {
            match serde_json::from_value::<proto::NormalizeInput>(params.clone()) {
                Ok(input) => provider.normalize_account(ctx, input).await.and_then(|account| {
                    serde_json::to_value(account).map_err(|e| format!("账号无法序列化: {e}"))
                }),
                Err(e) => Err(format!("normalize 参数无法解析: {e}")),
            }
        }
        proto::methods::gateway::ACCOUNT_VIEW => {
            let secret = params.get("secret").cloned().unwrap_or(Value::Null);
            provider.account_view(&secret)
        }
        proto::methods::gateway::ACCOUNT_IDENTITY => {
            let secret = params.get("secret").cloned().unwrap_or(Value::Null);
            provider.account_identity(&secret).map(Value::String)
        }
        proto::methods::gateway::LOGIN_START => {
            let method_name = params.get("method").and_then(Value::as_str).unwrap_or("");
            let payload = params.get("payload").cloned().unwrap_or(Value::Null);
            provider.login_start(ctx, method_name, &payload).await.and_then(|start| {
                serde_json::to_value(start).map_err(|e| format!("登录启动无法序列化: {e}"))
            })
        }
        proto::methods::gateway::LOGIN_POLL => {
            let flow = params.get("flow").and_then(Value::as_str).unwrap_or("");
            provider.login_poll(ctx, flow).await.and_then(|poll| {
                serde_json::to_value(poll).map_err(|e| format!("登录轮询无法序列化: {e}"))
            })
        }
        proto::methods::gateway::LOGIN_CALLBACK => {
            match serde_json::from_value::<proto::LoginCallback>(params.clone()) {
                Ok(callback) => provider.login_callback(ctx, callback).await.and_then(|poll| {
                    serde_json::to_value(poll).map_err(|e| format!("登录回调无法序列化: {e}"))
                }),
                Err(e) => Err(format!("callback 参数无法解析: {e}")),
            }
        }
        proto::methods::gateway::LOGIN_CANCEL => {
            let flow = params.get("flow").and_then(Value::as_str).unwrap_or("");
            provider.login_cancel(ctx, flow).await.map(|()| Value::Null)
        }
        _ => Err(format!("不认识的方法 {method}")),
    }
}
