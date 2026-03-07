#!/usr/bin/env bash
# Full build pipeline for gh CLI as a WASI component.
# Run from the scratch/ directory.
set -euo pipefail

SCRATCH="$(cd "$(dirname "$0")" && pwd)"
GOROOT="$SCRATCH/go-wasip3"
GH_DIR="$SCRATCH/gh-cli"
WT="$SCRATCH/wasm-tools/target/release/wasm-tools"
WASMTIME="$SCRATCH/wasmtime/target/release/wasmtime"
WIT_DIR="$GOROOT/src/internal/wasi/wit"
FAKE_ETC="$SCRATCH/fake-etc"
OUT="$SCRATCH/gh-component"

mkdir -p "$OUT" "$FAKE_ETC"

echo "=== Step 1: Vendor dependencies ==="
cd "$GH_DIR"
GOROOT="$GOROOT" "$GOROOT/bin/go" mod vendor 2>&1 | tail -3

echo "=== Step 2: Patch vendor ==="
bash "$SCRATCH/patch-gh-vendor.sh"

echo "=== Step 3: Compile to wasm32 ==="
GOROOT="$GOROOT" GOOS=wasip3 GOARCH=wasm32 \
    "$GOROOT/bin/go" build -mod=vendor -o "$OUT/gh.wasm" ./cmd/gh

echo "=== Step 4: Componentize ==="
"$WT" component embed --all-features "$WIT_DIR" --world command \
    "$OUT/gh.wasm" -o "$OUT/embedded.wasm"
"$WT" component new "$OUT/embedded.wasm" -o "$OUT/component.wasm"
rm "$OUT/embedded.wasm"

echo "=== Step 5: Set up fake /etc ==="
cp /run/systemd/resolve/stub-resolv.conf "$FAKE_ETC/resolv.conf" 2>/dev/null || \
    cp /etc/resolv.conf "$FAKE_ETC/resolv.conf" 2>/dev/null || true
cp /etc/hosts "$FAKE_ETC/hosts" 2>/dev/null || true

echo "=== Done ==="
ls -lh "$OUT/component.wasm"

echo ""
echo "Run with:"
echo "  $WASMTIME run \\"
echo "    -S p3 -W component-model-async -W component-model-async-builtins \\"
echo "    -W max-wasm-stack=8388608 \\"
echo "    -S inherit-env -S inherit-network -S tcp -S udp -S allow-ip-name-lookup \\"
echo "    --dir $FAKE_ETC::/etc --dir / \\"
echo "    --env SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \\"
echo "    -- $OUT/component.wasm <args>"
