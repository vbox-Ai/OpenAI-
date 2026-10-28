//! 最小可跑界面：初始化 Rust 核心 + 展示 provider 清单。
//!
//! 阶段 2 骨架：先证明「Flutter ↔ Rust FFI」链路通，UI 在后续阶段丰富。

import 'dart:io';

import 'package:flutter/material.dart';

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
      home: const ProviderListPage(),
    );
  }
}

class ProviderListPage extends StatefulWidget {
  const ProviderListPage({super.key});

  @override
  State<ProviderListPage> createState() => _ProviderListPageState();
}

class _ProviderListPageState extends State<ProviderListPage> {
  Agent2Providers? _core;
  List<dynamic> _providers = const [];
  String? _error;

  @override
  void initState() {
    super.initState();
    _bootstrap();
  }

  Future<void> _bootstrap() async {
    try {
      final core = Agent2Providers.open();
      // dataDir：真机用沙盒 Documents；这里骨架先给个临时目录。
      final dir = Directory.systemTemp.path;
      final code = core.init(dir, 'aarch64-apple-ios', 0);
      if (code != 0) {
        setState(() => _error = 'init 失败，错误码 $code');
        return;
      }
      _core = core;
      setState(() {
        _providers = core.providerInfos();
      });
    } catch (e) {
      setState(() => _error = e.toString());
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Agent2API Mobile')),
      body: _error != null
          ? Center(child: Text(_error!, style: const TextStyle(color: Colors.red)))
          : _providers.isEmpty
              ? const Center(child: CircularProgressIndicator())
              : ListView.builder(
                  itemCount: _providers.length,
                  itemBuilder: (context, i) {
                    final p = _providers[i] as Map<String, dynamic>;
                    return ListTile(
                      leading: const Icon(Icons.extension),
                      title: Text(p['label']?.toString() ?? p['id']?.toString() ?? '?'),
                      subtitle: Text(p['id']?.toString() ?? ''),
                    );
                  },
                ),
    );
  }

  @override
  void dispose() {
    _core?.shutdown();
    super.dispose();
  }
}
