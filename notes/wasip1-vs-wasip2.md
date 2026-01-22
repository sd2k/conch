# wasip1 vs wasip2 for Conch

**Date**: 2026-01-22  
**Updated**: 2026-01-22

## Current Status

The shell currently builds to `wasm32-wasip1` but **also compiles cleanly to `wasm32-wasip2`**:

```bash
# Both work:
cargo build -p conch-shell --target wasm32-wasip1 --release  # 6.2 MB, core module
cargo build -p conch-shell --target wasm32-wasip2 --release  # 6.6 MB, native component
```

The wasip2 build produces a native component (version 0x1000d) while wasip1 produces a core module (version 0x1 MVP).

## Why We Originally Used wasip1

The original reasoning (copied from eryx pattern) was:
1. eryx uses wasip1 because CPython and native extensions target wasip1
2. `wit_component::Linker` links `.so` files together, needs consistent target
3. The `wasi_snapshot_preview1.reactor.wasm` adapter bridges p1→p2

**However, conch doesn't have these constraints:**
- brush is pure Rust, compiles to either target
- We don't link against C libraries (currently)
- We control the entire toolchain

## wit-component::Linker and wasip2

The `wit_component::Linker` should work with wasip2 modules too. It operates at the component model level - taking core WASM modules and linking them into a component.

The wasip1 adapter is only needed when:
- You have modules built for wasip1 that need to run on a p2 host
- You're mixing wasip1 and wasip2 modules

If everything is wasip2 from the start, no adapter is needed.

## Comparison

| Aspect | wasip1 | wasip2 |
|--------|--------|--------|
| Output | Core module | Native component |
| Adapter needed | Yes (for p2 host) | No |
| jco compatible | Via adapter | Direct |
| VFS shadowing | Shadow WASI p1 or use adapter+p2 | Shadow `wasi:filesystem` directly |
| Future dynamic linking | Works with wit-dylib | Should also work |
| Host complexity | Need p1 APIs or adapter | Component model + bindgen |

## Benefits of Switching to wasip2

1. **Native component** - No adapter layer, cleaner architecture
2. **Direct jco support** - Can transpile to JS without extra steps
3. **Cleaner VFS** - Shadow `wasi:filesystem` interfaces directly on host
4. **Component model bindgen** - Typed bindings on host via `wasmtime::component::bindgen!`
5. **Future-proof** - wasip2 is the direction WASI is going

## What Would Need to Change

To switch from wasip1 to wasip2:

### 1. Build target
```toml
# mise.toml
run = "cargo build -p conch-shell --target wasm32-wasip2 --release"
```

### 2. Host runtime (`wasm_core.rs`)
```rust
// Before (wasip1):
use wasmtime::{Module, Store, ...};
use wasmtime_wasi::p1::WasiP1Ctx;

// After (wasip2):
use wasmtime::component::{Component, Linker, bindgen};
use wasmtime_wasi::p2::{WasiCtx, WasiView};

bindgen!({ world: "shell", path: "path/to/shell.wit" });
```

### 3. Shell exports
Currently the shell uses C-style FFI exports (`execute`, `get_stdout`, etc.). For a proper component, it needs to implement a WIT interface. The shell would need to export WIT-defined functions.

This is the trickiest part - brush writes to WASI stdout/stderr, we'd need to either:
- Keep the current C-FFI approach and wrap with an adapter
- Modify how output is captured to work with component model exports

### 4. VFS Integration
With wasip2, we can directly implement `wasi:filesystem` host traits (like eryx-vfs does), shadowing the default wasmtime-wasi implementation for `/ctx/...` paths.

## Recommendation

**Switch to wasip2** when we're ready to implement VFS integration. The component model gives us:
- Cleaner VFS via direct `wasi:filesystem` shadowing
- Better JS integration via jco
- Typed host bindings via bindgen

For now, wasip1 works fine. The migration is not urgent but should be done before:
- VFS integration (cleaner with p2)
- JS/jco integration (direct with p2)
- If we want typed component bindings

## Dynamic Linking (Future)

If we later want to dynamically load shell commands as separate WASM modules:
- `wit_component::Linker` can link wasip2 modules together
- This happens at "sandbox creation time" (like eryx), not true runtime linking
- The component model doesn't support true runtime dynamic linking yet
- This is independent of wasip1 vs wasip2 choice

## VFS and wasip2

The VFS architecture (see `vfs-architecture.md`) strongly favors wasip2:

1. **WASI filesystem shadowing requires component model**
   - We implement `wasi:filesystem` host traits
   - Use `linker.allow_shadowing(true)` to override default bindings
   - This is a component model feature (`wasmtime::component::Linker`)

2. **eryx uses this pattern successfully**
   - `eryx-vfs` implements WASI filesystem interfaces on the host
   - Guest (Python) uses normal file operations, unaware of VFS
   - Routes paths like `/data/*` to VFS, `/python-stdlib/*` to real FS

3. **For conch, this means**
   - Shell uses standard WASI filesystem (brush already does this)
   - Host shadows `wasi:filesystem` to route `/ctx/*` paths to `ContextFs`
   - No custom WIT imports needed in the shell
   - Any WASI-compatible tool could run in the sandbox

The wasip2 component model makes this clean. With wasip1, we'd need:
- The adapter layer to bridge p1→p2
- More complex linker setup
- Less direct access to component model features

## References

- eryx: Uses wasip1 because of CPython/native extension constraints
- wit-component: Links core modules into components, works with either target
- wasmtime: Supports both p1 (via `wasmtime_wasi::p1`) and p2 (via `wasmtime_wasi::p2`)
- jco: Can transpile components to JS; wasip2 components are native, wasip1 needs adapter
- `vfs-architecture.md`: Detailed VFS design notes based on eryx pattern