# Flutter 移动端（flutter-app）

Agent2API 的 Flutter 壳 + Rust FFI 绑定。**iOS 优先，Android 后期扩展。**

## 当前状态（阶段 2 骨架）

已完成：

- `lib/ffi.dart`：Rust 核心（`libagent2api_providers.a`）的完整 dart:ffi 绑定
  - init / provider_infos / call（通用控制面）/ shutdown
  - conversation_start / conversation_cancel（流式，chunk 回调）
  - set_notify_callback（models_changed / auth_changed）
- `lib/main.dart`：最小可跑界面（初始化 + provider 清单展示）
- `pubspec.yaml`

## 补齐平台工程（需要一台 Mac + Xcode）

本目录只含 Dart 代码。平台壳（iOS/Android 工程）用 Flutter 命令行生成：

```bash
cd flutter-app
flutter create . --org com.example --project-name agent2api_mobile
```

这会在 `flutter-app/` 下生成 `ios/`、`android/` 等平台工程，**不会覆盖已有的 `lib/`**。

### iOS 关键配置

1. **静态库链接**：把 CI 产物 `libagent2api_providers.a` 放进 `ios/Frameworks/`，
   并在 Xcode 的 `Build Phases → Link Binary With Libraries` 加入它。
   （`ffi.dart` 里 `DynamicLibrary.open('libagent2api_providers.a')` 对应这个链接。）

2. **ATS（App Transport Security）**：OAuth 回调走 `http://127.0.0.1`，
   必须在 `ios/Runner/Info.plist` 加：

```xml
<key>NSAppTransportSecurity</key>
<dict>
    <key>NSAllowsLocalNetworking</key>
    <true/>
</dict>
```

3. **越狱部署**：`ldid -S` 假签名后经 TrollStore / SSH 安装，或走 AltStore 侧载。

## Android（后期）

- Rust 加 `aarch64-linux-android` 等 target，产出 `.so`
- `ffi.dart` 的 `Agent2Providers.open()` 按平台改库名即可，Dart 逻辑零改动
