# Agent2API iOS 端开发方案

> 目标：把 Agent2API 的网关 + 提供商插件体系，以 Flutter APP 形态独立运行在越狱 iPhone 上，后期可平滑扩展 Android。
> 基线源码：`ProvidersPlugin/Agent2Providers`（已迁移至 `vbox-Ai/OpenAI-`）。
> 网页插件：`ProvidersPlugin/Agent2WebProviders`（已迁移至 `vbox-Ai/OpenAI-web-`）。

---

## 0. 开发边界要求（约定，必须遵守）

1. **开发优先级**：Flutter iOS 端优先 → 后期逐步 Android → 最后实现网页端。
2. **Flutter 端独立目录**：Flutter 项目在仓库内**新建独立目录**（如 `flutter-app/`），
   **不修改、不影响原有项目任何代码**。源项目（Rust 侧）日后更新/同步到仓库时，
   不会因为更新了源项目而改变 Flutter 这边的项目。
3. **记忆点机制**：每个步骤开发完毕后，必须**补充一份「记忆点」**（该步骤做了什么、
   关键决策、遗留问题、下一步入口），确认后再继续下一个步骤开发。
4. **分支策略**：
   - `main` = 最新代码（`3.3.0-beta.4` 基线），后续所有开发以此为准
   - `legacy-0.2.1` = 旧的 0.2.1 版本，仅存档，不再开发
5. **仓库划分**：
   - `vbox-Ai/OpenAI-` = 原生提供商插件（主仓库，含 Flutter 端）
   - `vbox-Ai/OpenAI-web-` = 网页提供商插件（单独仓库）

---

## 1. 背景与目标

### 1.1 现状
- 网关是 Tauri 桌面应用（本地 server + webview 壳 + 系统托盘）
- 插件是「独立子进程」模型：网关 `spawn` 插件，走 stdin/stdout JSON 行协议
- 五平台矩阵：win x64 / mac x64 / mac arm64 / linux x64 / linux arm64，**无 iOS**

### 1.2 目标
- 手机上独立运行，不依赖电脑
- Flutter 做 UI 壳（iOS 先上，Android 后上）
- Rust 核心内嵌进 APP（FFI），单进程，规避 iOS 禁止 spawn 子进程的限制

---

## 2. 总体架构

```
Flutter APP（单进程，装进手机）
│
├── Flutter UI（Dart）
│     账号管理 / 余额 / 签到 / 模型选择 / 设置
│
├── FFI 桥接层（flutter_rust_bridge 或手写 dart:ffi）
│     Dart ⇄ Rust 函数调用
│
└── Rust 核心（编译成静态库，链接进 APP）
      ├── 网关逻辑（从 Tauri 剥离出来的纯逻辑）
      └── 29 家 provider（全部静态编译进同一二进制）
```

**关键决策**：单进程。所有 provider 不再作为独立进程 spawn，而是编译进 APP 主进程直接函数调用。

---

## 3. 技术选型

| 层 | 选型 | 理由 |
|---|---|---|
| UI | Flutter | 一套代码 iOS/Android 通吃 |
| FFI | flutter_rust_bridge（首选）/ dart:ffi（兜底） | 自动生成绑定，减少手写胶水 |
| Rust 核心 | 现有代码改造为静态库（cdylib/staticlib） | 逻辑复用 |
| iOS 目标 | aarch64-apple-ios | Rust std 支持 |
| 内嵌浏览器 | flutter_inappwebview | OAuth 回调 + ZCode 验证码 |
| 本地存储 | 沿用现有 JSON/SQLite（rusqlite bundled） | 改路径即可 |
| 部署 | TrollStore / ldid 假签名 / 侧载 | 越狱机不走上架 |

---

## 4. 核心改造点（按依赖顺序）

### 4.1 单进程化（最关键）
- 把「网关 spawn 插件」改为「网关直接调用 provider trait」
- `crates/provider-sdk/src/serve.rs` 的 serve 主循环抽出来，作为进程内入口函数供 FFI 调用
- 移除 `std::process::Command` 的 spawn 路径

### 4.2 剥离 Tauri 依赖
- 网关逻辑从 `desktop-tauri/src-tauri/server` 抽成纯 Rust 库 crate
- 删掉系统托盘、桌面窗口、Tauri IPC
- 网关对外仍保留本地 HTTP 端点（供 APP 内 Flutter 调用）

### 4.3 FFI 桥接
- 定义边界 API：`init` / `login(provider)` / `chat(model)` / `balance` / `checkin`
- 用 flutter_rust_bridge 从 Rust 导出函数给 Dart
- 异步用 tokio + Dart 的 async/await 对接

### 4.4 路径适配
- `AppData`、`%APPDATA%`、`home_dir`、`temp_dir` → iOS 沙盒 Documents / tmp
- 设备标识存储（UMID、device id 等）统一落到 APP 沙盒目录

### 4.5 Windows 专属代码处理
| 位置 | 处理 |
|---|---|
| `zcode/activation.rs` tzutil/reg | 已有 `#[cfg(not(windows))]` 降级，免改 |
| `zcode/plan.rs` / `claim.rs` platform 字符串 | 已有 else 默认分支，可伪装 |
| `crypto.rs` DPAPI | 非 Windows 是报错桩，砍掉「导入桌面凭证」功能即可 |

### 4.6 OAuth 回调 webview
- 6 家 provider 有 loopback 回调（Antigravity / Accio / Trae / AutoClaw / CodeArts / LobsterAI）
- 用 flutter_inappwebview 打开授权 URL，webview 跳回 `127.0.0.1:<port>` 回调
- Rust 侧在进程内绑本地端口（iOS 越狱下可行）

### 4.7 ZCode 验证码（无痕验证）
三条路，按优先级：
1. **路 C**：编码套餐通道不需要验证码，先让 ZCode 核心可用（零额外工作）
2. **路 A**：flutter_inappwebview 跑同一份 `aliyun-captcha.js`，静默铸令牌（等价替换 Tauri webview）
3. **路 B**：Rust 内嵌 JS/DOM 引擎（rquickjs + DOM 模拟层），headless 也能铸

### 4.8 Qoder UMID 风控组件
- `qoder/risk.rs` spawn 桌面二进制 `runtime-info`
- iOS 上 spawn 不可行、架构不匹配
- **决策**：砍掉「国际版每日领 100 Credits」活动；核心（模型调用/登录/签到）不受影响
- 进阶：逆向 runtime-info 用纯 Rust 重写（工作量大，暂缓）

---

## 5. 分阶段实施计划

### 阶段 0：验证编译（1-2 天）
- `rustup target add aarch64-apple-ios`
- 把 `crates/providers` 编成 iOS 静态库，确认 Rust 层能过编译
- 卡点即第一个坑，先暴露

### 阶段 1：单进程化 + 剥 Tauri（1-2 周）
- 抽网关纯逻辑库
- provider trait 改进程内调用
- 编译通过、本地单进程能跑通 init + 一家 provider

### 阶段 2：FFI 桥接 + 最小 Flutter 壳（1-2 周）
- flutter_rust_bridge 打通 Dart⇄Rust
- 最小 UI：登录一家 provider + 发一条模型请求

### 阶段 3：路径适配 + 全量 provider 移植（1-2 周）
- 路径全部迁到沙盒
- 逐家 provider 过一遍，确认核心可用

### 阶段 4：OAuth webview + ZCode 验证码（1-2 周）
- flutter_inappwebview 接入
- ZCode 走路 C（先）→ 路 A（后）

### 阶段 5：部署 + 后台（1 周）
- ldid 假签名 / TrollStore 安装
- 后台：越狱 launchd 常驻，或容忍 iOS 挂起（见 §8）

### 阶段 6：Android 扩展（后期）
- 加 `aarch64-linux-android` 等 Rust target + NDK
- Flutter UI 零改动，只补 FFI 构建配置

---

## 6. 目录结构规划（迁移后仓库）

**核心原则：Flutter 端在独立目录，与源项目代码完全隔离，互不影响。**

```
OpenAI-/
├── crates/              # 原有 Rust 源码（源项目，保持不动）
│   ├── providers/
│   ├── provider-protocol/
│   └── provider-sdk/
├── scripts/             # 原有构建脚本（保持不动）
├── ui/                  # 原有桌面端 UI（保持不动）
├── pack.json            # 原有插件清单（保持不动）
│
├── flutter-app/         # ← 新增：Flutter 端（独立目录，不影响上面任何代码）
│   ├── lib/             # Dart UI
│   ├── ios/             # iOS 工程
│   ├── android/         # Android 工程（后期）
│   └── rust_bridge/     # flutter_rust_bridge 绑定
│
├── docs/                # ← 新增：文档与记忆点
│   ├── iOS开发方案.md
│   └── memory/          # 每个步骤的「记忆点」
└── README.md            # 原有 README（保持不动）
```

> 源项目更新同步时，只动 `crates/ scripts/ ui/ pack.json` 这些原有部分；
> `flutter-app/` 与 `docs/` 是独立新增目录，永远不受源项目更新影响。

---

## 7. 构建链配置

### iOS
```
rustup target add aarch64-apple-ios
# 静态库产物链接进 Flutter
```

### 签名（越狱）
- ldid 假签名：`ldid -S <binary>`
- TrollStore 免签安装，或侧载
- 不走上架，无证书限制

---

## 8. 后台运行方案

越狱机有两种选择：

| 方案 | 说明 | 适用 |
|---|---|---|
| launchd 常驻 daemon | 核心跑成系统守护进程，APP 只是前端 | 最稳，需越狱 |
| APP 内运行 + 容忍挂起 | 常规 APP，后台可能被 iOS 挂起 | 简单，但重活可能断 |

推荐：**核心逻辑做成 launchd daemon，Flutter APP 通过本地 HTTP 连 daemon**。这样既保留 GUI，又不怕后台被杀。

---

## 9. 风险与对策

| 风险 | 对策 |
|---|---|
| 上游 provider 环境检测拒手机请求 | 逐家实测，设备标识伪装成桌面值 |
| ZCode 验证码风控限速 | 按「库存低才补」节奏铸令牌，勿猛铸 |
| Qoder 国际版领积分不可用 | 接受，核心功能不受影响 |
| 部分 provider 上游 API 变更 | 保持 upstream 同步，定期 rebase |
| 单进程失去崩溃隔离 | 用 catch_unwind 在请求边界兜住（现有代码已有 guard） |

---

## 10. 工作量估算

| 阶段 | 估算 |
|---|---|
| 验证编译 | 1-2 天 |
| 单进程化 + 剥 Tauri | 1-2 周 |
| FFI + 最小壳 | 1-2 周 |
| 路径 + 全量 provider | 1-2 周 |
| OAuth webview + 验证码 | 1-2 周 |
| 部署 + 后台 | 1 周 |
| **总计** | **约 6-10 周（单人）** |

---

## 附录：provider 逐家适配清单

| Provider | iOS 判定 | 需要的处理 |
|---|---|---|
| ZCode | 🟡 核心可用 | 验证码走编码通道/路 A；platform 字符串伪装 |
| Qoder | 🟢 核心可用 | 砍 UMID 组件（国际版领积分废） |
| AutoClaw | 🟡 改路径 | AppData→沙盒；DPAPI 导入废 |
| OrcaTerm | 🟡 改路径 | Tauri store 明文路径 |
| Antigravity / Accio / Trae / CodeArts / LobsterAI | 🟡 换 webview | loopback 回调换 flutter_inappwebview |
| 其余（ModelScope、Cline、Kuku、MonkeyCode、OfficeAce、WorkBuddy 等） | 🟢 大概率能跑 | 改路径即可 |
