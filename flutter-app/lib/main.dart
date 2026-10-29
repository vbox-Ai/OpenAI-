//! 移动端壳：复用网关 Web UI。
//!
//! 方案：不重写 UI，用 webview 加载网关那套 Web 前端（HTML/JS/CSS），
//! 它通过 HTTP 调本地 Rust server，server 底层接 FFI（EmbeddedHost）。
//!
//! ── 当前状态（阶段 2 骨架）─────────────────────────────────
//! webview 已就位，但「本地 HTTP 桥 server」还没写 —— 那是复用 Web UI 路线的
//! 唯一关键块（见 docs/iOS开发方案.md §2.5.5 / §14）。桥 server 起来前，
//! 下面这个 127.0.0.1:3065 是空的，页面会白屏，属预期。

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_inappwebview/flutter_inappwebview.dart';

import 'ffi.dart';

void main() {
  runApp(const Agent2App());
}

class Agent2App extends StatelessWidget {
  const Agent2App({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Agent2API Mobile',
      theme: ThemeData.dark(useMaterial3: true),
      home: const WebUiPage(),
    );
  }
}

class WebUiPage extends StatefulWidget {
  const WebUiPage({super.key});

  @override
  State<WebUiPage> createState() => _WebUiPageState();
}

class _WebUiPageState extends State<WebUiPage> {
  Agent2Providers? _core;
  InAppWebViewController? _controller;

  @override
  void initState() {
    super.initState();
    _bootstrap();
  }

  Future<void> _bootstrap() async {
    try {
      final core = Agent2Providers.open();
      // dataDir：真机用沙盒 Documents；骨架先给临时目录。
      final dir = Directory.systemTemp.path;
      final code = core.init(dir, 'aarch64-apple-ios', 3065);
      if (code != 0) {
        setState(() {});
        return;
      }
      _core = core;
    } catch (_) {
      // init 失败也继续加载 webview，便于排查白屏。
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: InAppWebView(
        // 网关 Web UI 的入口。桥 server 起来后指向它；也可先指本地打包的
        // assets（flutter_inappwebview 支持 loadFile / localhost）。
        initialUrlRequest: URLRequest(
          url: WebUri('http://127.0.0.1:3065/'),
        ),
        onWebViewCreated: (controller) {
          _controller = controller;
        },
        initialSettings: InAppWebViewSettings(
          javaScriptEnabled: true,
          domStorageEnabled: true,
          // 本地 HTTP 桥走 loopback，需放行
          allowsLinkPreview: false,
        ),
      ),
    );
  }

  @override
  void dispose() {
    _core?.shutdown();
    _controller?.dispose();
    super.dispose();
  }
}
