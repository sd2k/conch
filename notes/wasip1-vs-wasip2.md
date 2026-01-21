# wasip1 vs wasip2 for Conch

**Date**: 2026-01-22

## Summary

The VFS integration plan uses `wasm32-wasip1` + adapter, but `wasm32-wasip2` might work directly for pure Rust guests like brush.

## wit-dylib Findings

From the [wit-dylib README](https://github.com/bytecodealliance/wasm-tools/blob/main/crates/wit-dylib/README.md):

- wit-dylib actually references **wasip2**: `$WASI_SDK_PATH/share/wasi-sysroot/lib/wasm32-wasip2/libc.so`
- The `wasi_snapshot_preview1.reactor.wasm` adapter is for **bridging** p1↔p2 compatibility when linking mixed modules
- It's not a hard requirement if all modules are wasip2

## Why eryx uses wasip1

eryx uses `wasm32-wasip1` because:
1. CPython is built for wasip1 (external dependency)
2. Native extensions (`.so` files) need to link against CPython
3. Everything must be the same target to link together

## Options for Conch

**Option A: wasip2 all the way**
- Build brush shell to `wasm32-wasip2`
- No adapter needed for base case
- Dynamic command extensions (future) built as wasip2 too
- Simpler architecture
- Requires: brush compiles cleanly to wasip2

**Option B: wasip1 + adapter (current plan)**
- Build brush shell to `wasm32-wasip1`
- Use `wasi_snapshot_preview1.reactor.wasm` adapter on host
- More compatible, proven pattern from eryx
- Required if we ever link against C libraries

## Recommendation

Start with **Option B** (wasip1 + adapter) as planned:
- Proven pattern
- Preserves flexibility for future C library linking
- Can always simplify to wasip2-only later if desired

If wasip2 build works cleanly for brush and we don't need C interop, could revisit and simplify.

## Test Commands

```bash
# Check if brush builds for wasip2
cargo build -p conch-shell --target wasm32-wasip2 --release

# Current working build (wasip1)
cargo build -p conch-shell --target wasm32-wasip1 --release
```
