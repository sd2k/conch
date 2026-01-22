# Conch Cleanup Roadmap

**Date**: 2026-01-22

## Current State

The codebase has remnants from an initial rushed implementation. We've already cleaned up some issues (removed dead component model code, added `InstancePre`), but there's more to do.

### What Works Today
- Shell compiles to `wasm32-wasip1` and runs via `CoreShellExecutor`
- Basic resource limits (CPU epochs, memory, output)
- Go FFI bindings via purego (no CGO)
- `ContextFs` and `ContextProvider` implemented but not wired up

### Target Architecture

**Single build target (wasip2), multiple host languages:**

| Host | Interface | Executor | VFS |
|------|-----------|----------|-----|
| Rust (native) | Rust API | ComponentShellExecutor | WASI shadowing |
| Go (via FFI) | C ABI | ComponentShellExecutor | WASI shadowing |
| JavaScript (jco) | JS bindings | jco-transpiled | JS WASI shim |

The key insight: **Go calls into Rust via FFI, and Rust uses the component model internally**. Go doesn't need to understand components — it just calls C functions that return results.

---

## Phase 1: Clean Up Shell Exports

**Goal**: Make the shell's interface cleaner and more consistent.

### Current State
The shell uses C-style FFI exports:
```rust
#[no_mangle]
pub extern "C" fn execute(script_ptr: *const u8, script_len: usize) -> i32;
pub extern "C" fn get_stdout_len() -> usize;
pub extern "C" fn get_stdout(buf_ptr: *mut u8, buf_len: usize) -> usize;
// etc.
```

This works but is awkward — requires multiple calls to get results.

### Option A: Keep C-FFI (for Go compatibility)
- Works with both wasip1 and wasip2 targets
- Go can call these via the WASM memory interface
- No component model required

### Option B: Add WIT exports (for component model hosts)
- Define proper WIT interface for shell execution
- Use `wit-bindgen` in the shell to generate exports
- Rust/JS hosts use typed bindings via `bindgen!`

### Recommendation
**Do both**: Keep C-FFI exports for Go, add WIT exports for component model hosts. The shell can export both.

---

## Phase 2: wasip2 Build Target

**Goal**: Build shell as native component for Rust/JS hosts.

### Changes Required

1. **Add wasip2 build task in `mise.toml`**
   ```toml
   [tasks.wasm-build-p2]
   run = "cargo build -p conch-shell --target wasm32-wasip2 --release"
   ```

2. **Create WIT interface for shell** (optional, see Phase 1)
   ```wit
   package conch:shell@0.1.0;
   
   world shell {
       // Standard WASI imports (filesystem, clocks, etc.)
       // These get shadowed by host for VFS
       
       record execute-result {
           exit-code: s32,
           stdout: list<u8>,
           stderr: list<u8>,
       }
       
       export execute: func(script: string) -> execute-result;
   }
   ```

3. **Embed both modules**
   ```rust
   #[cfg(feature = "embedded-shell-p1")]
   static SHELL_P1: &[u8] = include_bytes!("...wasm32-wasip1/.../conch_shell.wasm");
   
   #[cfg(feature = "embedded-shell-p2")]
   static SHELL_P2: &[u8] = include_bytes!("...wasm32-wasip2/.../conch_shell.wasm");
   ```

---

## Phase 3: Component Model Executor (Rust)

**Goal**: Add a proper component model executor alongside the existing core executor.

### New Structure
```
crates/conch/src/
├── executor/
│   ├── mod.rs           # Common traits and types
│   ├── core.rs          # CoreShellExecutor (wasip1, current code)
│   └── component.rs     # ComponentShellExecutor (wasip2, new)
├── vfs/
│   ├── mod.rs           # Re-exports
│   ├── context.rs       # ContextFs, ContextProvider (existing)
│   ├── bindings.rs      # WASI filesystem bindgen (new)
│   └── host.rs          # Host trait implementations (new)
```

### Component Executor Implementation
```rust
// crates/conch/src/executor/component.rs

use wasmtime::component::{Component, Linker, bindgen};
use wasmtime_wasi::p2::{WasiCtx, WasiView};

bindgen!({
    world: "shell",
    path: "wit/shell.wit",
    async: true,
});

pub struct ComponentShellExecutor {
    engine: Arc<Engine>,
    instance_pre: Arc<ShellPre<ExecutorState>>,
}

struct ExecutorState {
    wasi: WasiCtx,
    table: ResourceTable,
    vfs_ctx: ConchVfsCtx,  // For VFS shadowing
}

impl ComponentShellExecutor {
    pub fn new(component_bytes: &[u8]) -> Result<Self, Error> {
        let engine = Engine::new(&config)?;
        let component = Component::new(&engine, component_bytes)?;
        
        let mut linker = Linker::new(&engine);
        
        // Add standard WASI
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        
        // Shadow filesystem with our VFS
        linker.allow_shadowing(true);
        conch_vfs::add_to_linker(&mut linker)?;
        linker.allow_shadowing(false);
        
        let instance_pre = linker.instantiate_pre(&component)?;
        let instance_pre = ShellPre::new(instance_pre)?;
        
        Ok(Self { engine, instance_pre })
    }
    
    pub async fn execute(&self, script: &str, ctx: &ExecutionContext) -> Result<ExecutionResult> {
        let state = ExecutorState::new(ctx);
        let mut store = Store::new(&self.engine, state);
        
        let instance = self.instance_pre.instantiate_async(&mut store).await?;
        let result = instance.call_execute(&mut store, script).await?;
        
        Ok(ExecutionResult {
            exit_code: result.exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
            ..
        })
    }
}
```

---

## Phase 4: VFS Integration

**Goal**: Wire `ContextFs` to WASM execution via WASI filesystem shadowing.

### Create `conch-vfs` module (or inline in `crates/conch/src/vfs/`)

1. **WIT file for host bindings**
   ```wit
   // wit/vfs.wit
   package conch:vfs;
   
   world conch-filesystem {
       import wasi:filesystem/types@0.2.6;
       import wasi:filesystem/preopens@0.2.6;
   }
   ```

2. **Generate bindings**
   ```rust
   // src/vfs/bindings.rs
   wasmtime::component::bindgen!({
       path: "wit/vfs.wit",
       world: "conch-filesystem",
       with: {
           "wasi:filesystem/types/descriptor": ConchDescriptor,
           "wasi:filesystem/types/directory-entry-stream": ConchReaddirIterator,
       },
   });
   ```

3. **Implement Host traits using ContextFs**
   ```rust
   // src/vfs/host.rs
   impl types::Host for ConchVfsState<'_> {
       async fn read(&mut self, fd: Resource<Descriptor>, len: u64) -> Result<Vec<u8>> {
           let desc = self.table.get(fd)?;
           match desc {
               ConchDescriptor::Context(path) => {
                   self.context_fs.read(&path).await
               }
               ConchDescriptor::Scratch(path) => {
                   self.scratch.read(&path)
               }
           }
       }
       // ... other methods
   }
   ```

4. **Path routing**
   ```rust
   impl ConchVfsCtx {
       fn open(&mut self, path: &str) -> Result<ConchDescriptor> {
           if path.starts_with("/ctx/") {
               Ok(ConchDescriptor::Context(path.to_string()))
           } else if path.starts_with("/tmp/") {
               Ok(ConchDescriptor::Scratch(path.to_string()))
           } else {
               Err(ErrorCode::NoEntry)
           }
       }
   }
   ```

---

## Phase 5: JavaScript/jco Support

**Goal**: Enable running conch in browser/Node.js via jco.

### Steps
1. Build shell to wasip2 (Phase 2)
2. Use jco to transpile: `jco transpile conch_shell.wasm -o dist/`
3. Implement WASI filesystem shim in JavaScript
4. The shim routes `/ctx/*` paths to a JS `ContextProvider`

```javascript
// Example JS usage
import { instantiate } from './dist/conch_shell.js';

const contextProvider = {
    async getToolCall(agentId, id) { /* ... */ },
    async listToolCalls(agentId) { /* ... */ },
};

const filesystem = createConchFilesystem(contextProvider);

const shell = await instantiate({
    'wasi:filesystem/types': filesystem,
    'wasi:filesystem/preopens': {
        getDirectories() {
            return [[filesystem.rootDescriptor, '/ctx']];
        }
    }
});

const result = await shell.execute('cat /ctx/self/tools/123/result.json | jq .name');
```

---

## Summary: What to Build

| Phase | Component | Priority |
|-------|-----------|----------|
| 1 | Clean shell exports (WIT interface) | Medium |
| 2 | wasip2 build target | High |
| 3 | Component executor (replaces core executor) | High |
| 4 | VFS integration | High |
| 5 | jco/JS support | Medium |

### Suggested Order
1. **Phase 2 + 3**: Get wasip2 component executor working (no VFS yet)
2. **Phase 4**: Add VFS integration  
3. **Phase 5**: Test with jco
4. **Phase 1**: Clean up shell exports if needed

### Go Support
Go continues to work via FFI — the C ABI doesn't change. Internally, the FFI functions (`conch_execute`, etc.) will use the new `ComponentShellExecutor` instead of `CoreShellExecutor`. Go callers don't need to know or care about this change.

### Files to Create/Modify

**New:**
- `crates/conch/src/executor/mod.rs`
- `crates/conch/src/executor/component.rs`
- `crates/conch/src/vfs/bindings.rs`
- `crates/conch/src/vfs/host.rs`
- `crates/conch/wit/shell.wit` (WIT interface)
- `crates/conch/wit/vfs.wit` (for host bindings)

**Modify:**
- `crates/conch/src/vfs.rs` → `crates/conch/src/vfs/context.rs`
- `crates/conch/src/ffi.rs` (use ComponentShellExecutor internally)
- `mise.toml` (switch to wasip2 build)
- `Cargo.toml` (update dependencies)

**Delete:**
- `crates/conch/src/wasm_core.rs` (replaced by component executor)

**Keep as-is:**
- Go bindings (`tests/go/`) — same C ABI, different internal executor
- MCP server (uses Rust executor)
- CLI (uses Rust executor)