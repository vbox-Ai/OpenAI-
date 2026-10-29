//! Rust 核心（libagent2api_providers.a）的 dart:ffi 绑定。
//!
//! 与 `crates/providers/src/ffi.rs` 一一对应。
//! 字符串契约：Rust 返回的字符串读完必须调 `stringFree` 释放。

import 'dart:convert';
import 'dart:ffi';
import 'dart:isolate';

import 'package:ffi/ffi.dart';

// ── 原生签名 ─────────────────────────────────────────────────
typedef _InitNative = Int32 Function(Pointer<Utf8>, Pointer<Utf8>, Uint16);
typedef _InitDart = int Function(Pointer<Utf8>, Pointer<Utf8>, int);

typedef _ProviderInfosNative = Pointer<Utf8> Function();
typedef _ProviderInfosDart = Pointer<Utf8> Function();

typedef _CallNative = Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>);
typedef _CallDart = Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>);

typedef _StringFreeNative = Void Function(Pointer<Utf8>);
typedef _StringFreeDart = void Function(Pointer<Utf8>);

typedef _ShutdownNative = Void Function();
typedef _ShutdownDart = void Function();

// 流式回调（长度前缀）
typedef _ChunkCallbackNative = Void Function(
    Pointer<Uint8>, UintPtr, Pointer<Void>);
typedef _ChunkCallbackDart = void Function(
    Pointer<Uint8>, int, Pointer<Void>);

typedef _DoneCallbackNative = Void Function(
    Int32, Pointer<Utf8>, Pointer<Void>);
typedef _DoneCallbackDart = void Function(int, Pointer<Utf8>, Pointer<Void>);

typedef _ConversationStartNative = Uint64 Function(
    Pointer<Utf8>,
    Pointer<NativeFunction<_ChunkCallbackNative>>,
    Pointer<NativeFunction<_ChunkCallbackNative>>,
    Pointer<NativeFunction<_DoneCallbackNative>>,
    Pointer<Void>);
typedef _ConversationStartDart = int Function(
    Pointer<Utf8>,
    Pointer<NativeFunction<_ChunkCallbackNative>>,
    Pointer<NativeFunction<_ChunkCallbackNative>>,
    Pointer<NativeFunction<_DoneCallbackNative>>,
    Pointer<Void>);

typedef _ConversationCancelNative = Void Function(Uint64);
typedef _ConversationCancelDart = void Function(int);

typedef _NotifyCallbackNative = Void Function(
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Void>);
typedef _NotifyCallbackDart = void Function(
    Pointer<Utf8>, Pointer<Utf8>, Pointer<Void>);

typedef _SetNotifyNative = Void Function(
    Pointer<NativeFunction<_NotifyCallbackNative>>, Pointer<Void>);
typedef _SetNotifyDart = void Function(
    Pointer<NativeFunction<_NotifyCallbackNative>>, Pointer<Void>);

/// 进程内宿主的 Dart 封装。
class Agent2Providers {
  Agent2Providers(DynamicLibrary lib)
      : _init = lib.lookupFunction<_InitNative, _InitDart>(
            'agent2api_providers_init'),
        _providerInfos = lib.lookupFunction<_ProviderInfosNative,
            _ProviderInfosDart>('agent2api_providers_provider_infos'),
        _call = lib.lookupFunction<_CallNative, _CallDart>(
            'agent2api_providers_call'),
        _stringFree = lib.lookupFunction<_StringFreeNative, _StringFreeDart>(
            'agent2api_providers_string_free'),
        _shutdown = lib.lookupFunction<_ShutdownNative, _ShutdownDart>(
            'agent2api_providers_shutdown'),
        _conversationStart =
            lib.lookupFunction<_ConversationStartNative,
                _ConversationStartDart>('agent2api_providers_conversation_start'),
        _conversationCancel =
            lib.lookupFunction<_ConversationCancelNative,
                _ConversationCancelDart>('agent2api_providers_conversation_cancel'),
        _setNotify = lib.lookupFunction<_SetNotifyNative, _SetNotifyDart>(
            'agent2api_providers_set_notify_callback');

  final _InitDart _init;
  final _ProviderInfosDart _providerInfos;
  final _CallDart _call;
  final _StringFreeDart _stringFree;
  final _ShutdownDart _shutdown;
  final _ConversationStartDart _conversationStart;
  final _ConversationCancelDart _conversationCancel;
  final _SetNotifyDart _setNotify;

  /// 加载静态库。iOS 上动态库名由 Flutter 插件自动处理；
  /// 这里默认按平台找 `libagent2api_providers`。
  static Agent2Providers open() {
    return Agent2Providers(DynamicLibrary.open('libagent2api_providers.a'));
  }

  /// 初始化宿主。返回错误码（0 = 成功）。
  int init(String dataDir, String target, int loopbackPort) {
    final d = dataDir.toNativeUtf8();
    final t = target.toNativeUtf8();
    try {
      return _init(d, t, loopbackPort);
    } finally {
      malloc.free(d);
      malloc.free(t);
    }
  }

  /// 拿全部 provider 握手信息（JSON 数组）。
  List<dynamic> providerInfos() {
    final ptr = _providerInfos();
    if (ptr == nullptr) return const [];
    try {
      return jsonDecode(ptr.toDartString()) as List<dynamic>;
    } finally {
      _stringFree(ptr);
    }
  }

  /// 通用控制面调用，返回 `{"ok":bool,"result":...|"error":...}`。
  Map<String, dynamic> call(String method, Map<String, dynamic> params) {
    final m = method.toNativeUtf8();
    final p = jsonEncode(params).toNativeUtf8();
    final ptr = _call(m, p);
    malloc.free(m);
    malloc.free(p);
    if (ptr == nullptr) {
      return {'ok': false, 'error': 'FFI 返回 null'};
    }
    try {
      return jsonDecode(ptr.toDartString()) as Map<String, dynamic>;
    } finally {
      _stringFree(ptr);
    }
  }

  /// 发起流式对话，返回会话 id（0 = 失败）。
  int conversationStart(
    Map<String, dynamic> params, {
    required void Function(String headJson) onHead,
    required void Function(String chunk) onChunk,
    required void Function(bool ok, String? error) onDone,
  }) {
    final p = jsonEncode(params).toNativeUtf8();
    final headCb = _makeChunkCallback(onHead);
    final chunkCb = _makeChunkCallback(onChunk);
    final doneCb = _makeDoneCallback(onDone);
    try {
      final id = _conversationStart(
          p, headCb, chunkCb, doneCb, nullptr);
      return id;
    } finally {
      malloc.free(p);
    }
  }

  Pointer<NativeFunction<_ChunkCallbackNative>> _makeChunkCallback(
      void Function(String) sink) {
    final callable = NativeCallable<_ChunkCallbackNative>.listener(
      (Pointer<Uint8> data, int len, Pointer<Void> _) {
        sink(utf8.decode(data.asTypedList(len)));
      },
    );
    // 注意：callable 必须被持有到会话结束，否则回调会悬空。
    _activeCallables.add(callable);
    return callable.nativeFunction;
  }

  Pointer<NativeFunction<_DoneCallbackNative>> _makeDoneCallback(
      void Function(bool, String?) sink) {
    final callable = NativeCallable<_DoneCallbackNative>.listener(
      (int ok, Pointer<Utf8> errPtr, Pointer<Void> _) {
        sink(ok == 1, errPtr == nullptr ? null : errPtr.toDartString());
      },
    );
    _activeCallables.add(callable);
    return callable.nativeFunction;
  }

  final List<NativeCallable<dynamic>> _activeCallables = [];

  void conversationCancel(int id) => _conversationCancel(id);

  void setNotifyCallback(void Function(String kind, String payloadJson) cb) {
    final callable = NativeCallable<_NotifyCallbackNative>.listener(
      (Pointer<Utf8> kind, Pointer<Utf8> payload, Pointer<Void> _) {
        cb(kind.toDartString(), payload.toDartString());
      },
    );
    _activeCallables.add(callable);
    _setNotify(callable.nativeFunction, nullptr);
  }

  void shutdown() => _shutdown();
}
