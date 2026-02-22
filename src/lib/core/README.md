# Nexus stdlib wasm backends

`nxlib/stdlib/*.nx` が参照する wasm バックエンドの Rust 実装は `src/lib/*`（`src/lib/core` を含む）にあります。

## Module Mapping

| NX module | wasm file | Rust crate |
| --- | --- | --- |
| `nxlib/stdlib/stdio.nx` | `nxlib/stdlib/stdio.wasm` | `src/lib/stdio` |
| `nxlib/stdlib/core.nx` | `nxlib/stdlib/core.wasm` | `src/lib/core` |
| `nxlib/stdlib/string.nx` | `nxlib/stdlib/string.wasm` | `src/lib/string` |
| `nxlib/stdlib/math.nx` | `nxlib/stdlib/math.wasm` | `src/lib/math` |
| `nxlib/stdlib/fs.nx` | `nxlib/stdlib/fs.wasm` | `src/lib/fs` |
| `nxlib/stdlib/random.nx` | `nxlib/stdlib/random.wasm` | `src/lib/random` |
| `nxlib/stdlib/net.nx` | `nxlib/stdlib/net.wasm` | `src/lib/net` |
| component adapter | `nxlib/stdlib/net-host-adapter.wasm` | `src/lib/net_host_adapter` |

## Build

Repository root で `cargo build` を実行すると `build.rs` が上記 wasm を自動ビルドして
`nxlib/stdlib/` にコピーします。

```sh
cargo build
```

スキップしたい場合:

```sh
NEXUS_SKIP_WASM_BUILD=1 cargo build
```

手動ビルドは `src/lib/core/build.sh` を使ってください。
