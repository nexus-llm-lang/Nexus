# Nexus Project Review (Security / Performance / Portability)

## Executive Summary
- Security: **High risk**。ネットワーク制御は暫定で allow-all に戻しており、SSRF 面のリスクが高い。`ariadne` の `.unwrap()` は解消済み。
- Performance: **Medium risk**。FFI 文字列コピー、ビルド時の外部プロセス連打がボトルネック。
- Portability: **Medium risk**。`build --wasm` の `wasm-tools` 依存は解消済み。`wasm-merge` 依存（packed bundling 時）と wasmtime 固有 API 依存が残る。

## Findings

### Security
- `[High]` ネットワーク制御は暫定で allow-all に戻している。URL allowlist/blocklist 判定を無効化しており、任意の URL へ outbound request が可能。
  Reference: `src/runtime/mod.rs:49`, `src/runtime/mod.rs:57`
- `[Resolved]` FFI 文字列パスに汎用上限を導入済み（`MAX_FFI_STRING_BYTES`）。
  Reference: `src/interpreter/mod.rs:28`, `src/interpreter/mod.rs:273`, `src/interpreter/mod.rs:1037`
- `[Resolved]` `ariadne Report::print` の `.unwrap()` は削除済み。
  Reference: `src/main.rs:1034`, `src/main.rs:1056`, `src/main.rs:1074`

### Performance
- `[Medium]` 文字列 ABI がコピー中心。`pass_string_to_wasm` / `read_string_from_wasm` / `store_string_result` で往復コピーが発生。
  Reference: `src/interpreter/mod.rs:1033`, `src/interpreter/mod.rs:1078`, `src/lib/fs/src/lib.rs:169`, `src/lib/net/src/lib.rs:26`, `src/lib/string/src/lib.rs:23`
- `[Resolved]` HTTP 戻り値の書き戻しの append-only 実装は解消。host bridge で guest `allocate` を使って返却領域を確保する方式へ変更。
  Reference: `src/interpreter/mod.rs:284`, `src/interpreter/mod.rs:317`
- `[Low]` ビルド時に外部ツール起動は残るが、`wasm-merge` 可用性チェックのループ内再実行は解消し、ツール有無チェックをキャッシュ化済み。
  Reference: `src/main.rs:781`, `src/main.rs:87`, `src/main.rs:883`

### Portability
- `[Resolved]` `build --wasm` パスの `wasm-tools` CLI 依存は解消。component embed/new/compose を Rust crate (`wit-component` / `wasm-compose`) で内製化。
  Reference: `src/main.rs` (`encode_core_wasm_as_component`, `compose_component_with_nexus_host_adapter`)
- `[Medium]` `nexus run` の HTTP 実装が wasmtime の runtime API に寄っている。ランタイム差し替えが難しい。
  Reference: `src/interpreter/mod.rs:19-20` (`wasmtime_wasi_http`), `src/runtime/wasm_exec.rs:14-16`
- `[Medium]` URL パーサが簡易実装で RFC 準拠度が低い（`http/https` 以外拒否、authority/path 分解のみ）。
  Reference: `src/lib/net_host_adapter/src/lib.rs:45`, `src/lib/net_host_adapter/src/lib.rs:57`
- `[Low]` packed 実行形式は独自トレーラ方式。一般的な配布ツールとの互換性が低い。
  Reference: `src/main.rs:752` (`append_embedded_wasm`), `src/main.rs:762` (`split_embedded_wasm`)
