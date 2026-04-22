#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/../../.." && pwd)"

build_module() {
  manifest_path="$1"
  cargo build \
    --manifest-path "$manifest_path" \
    --target wasm32-wasip1 \
    --release
}

copy_module() {
  src="$1"
  dst="$2"
  cp "$src" "$dst"
}

build_module "$REPO_ROOT/src/lib/stdio/Cargo.toml"
build_module "$REPO_ROOT/src/lib/core/Cargo.toml"
build_module "$REPO_ROOT/src/lib/string/Cargo.toml"
build_module "$REPO_ROOT/src/lib/math/Cargo.toml"
build_module "$REPO_ROOT/src/lib/fs/Cargo.toml"
build_module "$REPO_ROOT/src/lib/random/Cargo.toml"
build_module "$REPO_ROOT/src/lib/net/Cargo.toml"
build_module "$REPO_ROOT/src/lib/nexus_host_bridge/Cargo.toml"

copy_module \
  "$REPO_ROOT/src/lib/stdio/target/wasm32-wasip1/release/nexus_stdio_wasm.wasm" \
  "$REPO_ROOT/nxlib/stdlib/stdio.wasm"
copy_module \
  "$REPO_ROOT/src/lib/core/target/wasm32-wasip1/release/nexus_core_wasm.wasm" \
  "$REPO_ROOT/nxlib/stdlib/core.wasm"
copy_module \
  "$REPO_ROOT/src/lib/string/target/wasm32-wasip1/release/nexus_string_wasm.wasm" \
  "$REPO_ROOT/nxlib/stdlib/string.wasm"
copy_module \
  "$REPO_ROOT/src/lib/math/target/wasm32-wasip1/release/nexus_math_wasm.wasm" \
  "$REPO_ROOT/nxlib/stdlib/math.wasm"
copy_module \
  "$REPO_ROOT/src/lib/fs/target/wasm32-wasip1/release/nexus_fs_wasm.wasm" \
  "$REPO_ROOT/nxlib/stdlib/fs.wasm"
copy_module \
  "$REPO_ROOT/src/lib/random/target/wasm32-wasip1/release/nexus_random_wasm.wasm" \
  "$REPO_ROOT/nxlib/stdlib/random.wasm"
copy_module \
  "$REPO_ROOT/src/lib/net/target/wasm32-wasip1/release/nexus_net_wasm.wasm" \
  "$REPO_ROOT/nxlib/stdlib/net.wasm"
copy_module \
  "$REPO_ROOT/src/lib/nexus_host_bridge/target/wasm32-wasip1/release/nexus_nexus_host_bridge_wasm.wasm" \
  "$REPO_ROOT/nxlib/stdlib/nexus-host-bridge.wasm"

echo "Wrote $REPO_ROOT/nxlib/stdlib/{stdio,core,string,math,fs,random,net,nexus-host-bridge}.wasm"
