//! 进程内模式：`EmbeddedGateway`（`CallGateway` 的直连实现）。
//!
//! ── 它解决什么问题 ─────────────────────────────────────────
//! 桌面端的 `CallGateway` 是 `serve.rs` 里的 `CallHandle`：把账号读写写成
//! JSON 帧发到 stdout、等网关回帧。移动端没有网关进程、没有 stdio，所有
//! 账号读写必须**直连本机存储**。
//!
//! `EmbeddedGateway` 实现同一个 [`CallGateway`] trait，但把 store / settings
//! 回调路由到 [`AccountStore`]（持久层抽象），`notify` 交给宿主事件通道。
//!
//! ── 持久层为什么抽象成 trait ────────────────────────────────
//! 真实落盘依赖平台：凭证进 Keychain（iOS 专属，走 FFI 到 Swift），
//! 账号 / 账本 / 设置进 SQLite。阶段 1.2 先给 [`MemoryStore`]（不落盘）占位，
//! 换真实现时只改一处。

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use agent2api_provider_protocol as proto;
use agent2api_provider_sdk::ctx::CallGateway;
use agent2api_provider_sdk::{StoreAccount, StoreError};
use serde_json::{json, Value};

/// 账号 / 账本 / 设置的持久层抽象（进程内模式的存储后端）。
pub trait AccountStore: Send + Sync {
    /// 读一条账号。`account` 为空串 = 该 provider 的队首账号。
    fn get(&self, provider: &str, account: &str) -> Result<Option<StoreAccount>, StoreError>;
    /// 列出一个 provider 的全部账号。
    fn list(&self, provider: &str) -> Result<Vec<StoreAccount>, StoreError>;
    /// 条件更新（比较再写）：当前 secret 等于 `expect` 时才写。
    fn update(
        &self,
        account: &str,
        expect: &Value,
        secret: &Value,
        public: &Value,
    ) -> Result<bool, StoreError>;
    /// 标记账号对某模型限额（网关据此冷却与轮换）。
    fn mark_limited(
        &self,
        account: &str,
        model: &str,
        reset_at: Option<i64>,
    ) -> Result<(), StoreError>;
    /// 读一份台账。
    fn ledger_get(&self, account: &str, kind: &str) -> Result<Value, StoreError>;
    /// 写一份台账。
    fn ledger_put(&self, account: &str, kind: &str, value: &Value) -> Result<(), StoreError>;
    /// 读一条按家配置。
    fn settings_get(&self, provider: &str, key: &str) -> Result<Value, StoreError>;
    /// 写一条按家配置。
    fn settings_set(&self, provider: &str, key: &str, value: &Value) -> Result<(), StoreError>;
}

/// 进程内网关：实现 `CallGateway`，把回调直连到 [`AccountStore`]。
pub struct EmbeddedGateway {
    store: Arc<dyn AccountStore>,
}

impl EmbeddedGateway {
    pub fn new(store: Arc<dyn AccountStore>) -> Self {
        Self { store }
    }
}

impl CallGateway for EmbeddedGateway {
    fn call<'a>(
        &'a self,
        method: &'a str,
        params: Value,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Value, StoreError>> + Send + 'a>> {
        let store = self.store.clone();
        let method = method.to_string();
        Box::pin(async move { route(store.as_ref(), &method, params) })
    }
}

/// 按方法名路由到 [`AccountStore`] 的对应操作。
fn route(store: &dyn AccountStore, method: &str, params: Value) -> Result<Value, StoreError> {
    use proto::methods::plugin as m;
    match method {
        m::STORE_GET => {
            let provider = str_of(&params, "provider");
            let account = str_of(&params, "account");
            match store.get(provider, account)? {
                Some(acct) => to_value(acct, "账号"),
                None => Err(StoreError::NotFound),
            }
        }
        m::STORE_LIST => {
            let provider = str_of(&params, "provider");
            to_value(store.list(provider)?, "账号列表")
        }
        m::STORE_UPDATE => {
            let account = str_of(&params, "account").to_string();
            let expect = params.get("expect").cloned().unwrap_or(Value::Null);
            let secret = params.get("secret").cloned().unwrap_or(Value::Null);
            let public = params.get("public").cloned().unwrap_or(Value::Null);
            let updated = store.update(&account, &expect, &secret, &public)?;
            Ok(json!({ "updated": updated }))
        }
        m::STORE_MARK_LIMITED => {
            let account = str_of(&params, "account").to_string();
            let model = str_of(&params, "model").to_string();
            let reset_at = params.get("reset_at").and_then(Value::as_i64);
            store.mark_limited(&account, &model, reset_at)?;
            Ok(Value::Null)
        }
        m::STORE_LEDGER_GET => {
            let account = str_of(&params, "account");
            let kind = str_of(&params, "kind");
            store.ledger_get(account, kind)
        }
        m::STORE_LEDGER_PUT => {
            let account = str_of(&params, "account");
            let kind = str_of(&params, "kind");
            let value = params.get("value").cloned().unwrap_or(Value::Null);
            store.ledger_put(account, kind, &value)?;
            Ok(Value::Null)
        }
        m::SETTINGS_GET => {
            let provider = str_of(&params, "provider");
            let key = str_of(&params, "key");
            store.settings_get(provider, key)
        }
        m::SETTINGS_SET => {
            let provider = str_of(&params, "provider");
            let key = str_of(&params, "key");
            let value = params.get("value").cloned().unwrap_or(Value::Null);
            store.settings_set(provider, key, &value)?;
            Ok(Value::Null)
        }
        m::NOTIFY => {
            // 进程内模式下，通知交给宿主事件通道（Flutter 刷新 UI）。
            // 阶段 1.2 占位：直接吞掉；1.6 接 FFI 事件通道。
            Ok(Value::Null)
        }
        other => Err(StoreError::Rejected(format!("未知回调方法: {other}"))),
    }
}

/// 从 params 里取一个字符串字段（缺省 = 空串）。
fn str_of<'a>(params: &'a Value, key: &str) -> &'a str {
    params.get(key).and_then(Value::as_str).unwrap_or("")
}

/// 序列化到 Value，失败包装成 StoreError。
fn to_value<T: serde::Serialize>(value: T, what: &str) -> Result<Value, StoreError> {
    serde_json::to_value(value)
        .map_err(|e| StoreError::Unavailable(format!("{what}序列化失败: {e}")))
}

/// 内存占位实现（阶段 1.2 用）：不落盘，进程重启即丢。
///
/// 真实实现替换为 SQLite（账号 / 账本 / 设置）+ Keychain（secret）。
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<MemoryState>,
}

#[derive(Default)]
struct MemoryState {
    /// key: `{provider}\0{id}`
    accounts: HashMap<String, StoreAccount>,
    /// key: `{account}\0{kind}`
    ledger: HashMap<String, Value>,
    /// key: `{provider}\0{key}`
    settings: HashMap<String, Value>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> Result<std::sync::MutexGuard<'_, MemoryState>, StoreError> {
        self.inner
            .lock()
            .map_err(|_| StoreError::Unavailable("存储锁中毒".to_string()))
    }
}

impl AccountStore for MemoryStore {
    fn get(&self, provider: &str, account: &str) -> Result<Option<StoreAccount>, StoreError> {
        let state = self.state()?;
        if account.is_empty() {
            // 队首账号：占位语义下取任意一条该 provider 的账号。
            Ok(state
                .accounts
                .values()
                .find(|a| a.provider == provider)
                .cloned())
        } else {
            Ok(state.accounts.get(&key(provider, account)).cloned())
        }
    }

    fn list(&self, provider: &str) -> Result<Vec<StoreAccount>, StoreError> {
        let state = self.state()?;
        let mut list: Vec<StoreAccount> = state
            .accounts
            .values()
            .filter(|a| a.provider == provider)
            .cloned()
            .collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(list)
    }

    fn update(
        &self,
        account: &str,
        expect: &Value,
        secret: &Value,
        public: &Value,
    ) -> Result<bool, StoreError> {
        let mut state = self.state()?;
        // 占位简化：按账号 id 全局找（真实现按 provider + id）。
        let Some(existing) = state.accounts.values_mut().find(|a| a.id == account) else {
            return Ok(false);
        };
        if existing.secret != *expect {
            return Ok(false); // 条件更新失败：当前值已变
        }
        existing.secret = secret.clone();
        existing.public = public.clone();
        Ok(true)
    }

    fn mark_limited(
        &self,
        _account: &str,
        _model: &str,
        _reset_at: Option<i64>,
    ) -> Result<(), StoreError> {
        // 占位：限额标记只影响冷却/轮换，内存实现不落盘，这里不做。
        Ok(())
    }

    fn ledger_get(&self, account: &str, kind: &str) -> Result<Value, StoreError> {
        let state = self.state()?;
        Ok(state
            .ledger
            .get(&key(account, kind))
            .cloned()
            .unwrap_or(Value::Null))
    }

    fn ledger_put(&self, account: &str, kind: &str, value: &Value) -> Result<(), StoreError> {
        let mut state = self.state()?;
        state.ledger.insert(key(account, kind), value.clone());
        Ok(())
    }

    fn settings_get(&self, provider: &str, key_name: &str) -> Result<Value, StoreError> {
        let state = self.state()?;
        Ok(state
            .settings
            .get(&key(provider, key_name))
            .cloned()
            .unwrap_or(Value::Null))
    }

    fn settings_set(
        &self,
        provider: &str,
        key_name: &str,
        value: &Value,
    ) -> Result<(), StoreError> {
        let mut state = self.state()?;
        state
            .settings
            .insert(key(provider, key_name), value.clone());
        Ok(())
    }
}

fn key(a: &str, b: &str) -> String {
    format!("{a}\0{b}")
}

/// 构造一个内存占位的进程内网关（阶段 1.2 用）。
pub fn memory_gateway() -> Arc<dyn CallGateway> {
    Arc::new(EmbeddedGateway::new(Arc::new(MemoryStore::new())))
}
