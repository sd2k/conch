# VFS Architecture for Conch

**Date**: 2026-01-22

## Key Insight from eryx

The eryx project uses a brilliant pattern for VFS: **it doesn't define a custom WIT interface for filesystem operations**. Instead, it implements the standard `wasi:filesystem` interfaces on the host side and shadows/replaces the default wasmtime-wasi bindings.

This means:
- The guest (Python/shell) uses normal WASI filesystem calls (`open`, `read`, `write`, etc.)
- The guest has no idea it's talking to a virtual filesystem
- The virtualization is completely transparent — a host-side implementation detail

## How eryx Does It

### 1. The VFS WIT file is for HOST bindings, not guest imports

```wit
// eryx-vfs/wit/world.wit
package eryx:vfs;

world virtual-filesystem {
  import wasi:filesystem/types@0.2.6;
  import wasi:filesystem/preopens@0.2.6;
}
```

This tells `wasmtime::component::bindgen!` to generate the trait signatures that the host must implement. The guest never sees this.

### 2. Linker shadowing replaces the default filesystem

```rust
// In the host, when setting up the linker:
wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;  // Add standard WASI

// Then override filesystem with custom implementation
linker.allow_shadowing(true);
eryx_vfs::add_hybrid_vfs_to_linker(&mut linker)?;
linker.allow_shadowing(false);
```

### 3. The "Hybrid" VFS routes by path

eryx uses `HybridVfsCtx` which routes filesystem calls based on path:
- `/data/*` → `InMemoryStorage` (sandboxed, writable)
- `/python-stdlib/*` → Real filesystem (read-only passthrough)

This allows:
- User data to be sandboxed in memory
- Python stdlib to "just work" from the host filesystem
- Storage to be persisted/restored across sessions

## How Conch Should Do It

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  conch-shell.wasm (guest)                                       │
│  - Uses standard WASI filesystem calls (via brush)              │
│  - No knowledge of VFS — just sees paths like /ctx/self/tools/  │
└─────────────────────────────────────────────────────────────────┘
                              │
                    WASI filesystem calls
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Host (wasmtime)                                                │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  ConchVfs (implements wasi:filesystem Host traits)        │  │
│  │                                                           │  │
│  │  Routes by path:                                          │  │
│  │  - /ctx/self/tools/*    → ContextFs (tool call history)   │  │
│  │  - /ctx/self/messages/* → ContextFs (conversation)        │  │
│  │  - /ctx/parent/*        → ContextFs (parent agent)        │  │
│  │  - /tmp/*               → InMemoryStorage (scratch)       │  │
│  │  - /workspace/*         → RealFs (optional, read-only)    │  │
│  └───────────────────────────────────────────────────────────┘  │
│                              │                                  │
│                              ▼                                  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  ContextProvider (trait)                                  │  │
│  │  - get_tool_call(agent_id, id) → ToolCall                 │  │
│  │  - list_tool_calls(agent_id) → Vec<ToolCallSummary>       │  │
│  │  - get_message(agent_id, index) → Message                 │  │
│  │  - etc.                                                   │  │
│  └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### Implementation Steps

1. **Create `conch-vfs` crate** (similar to `eryx-vfs`)
   - WIT file that imports `wasi:filesystem` interfaces
   - Use `bindgen!` to generate host trait signatures
   - Implement traits using our existing `ContextFs`

2. **Implement path routing**
   ```rust
   impl ConchVfsCtx {
       fn route_path(&self, path: &str) -> PathDestination {
           if path.starts_with("/ctx/") {
               PathDestination::Context(self.context_fs.parse_path(path))
           } else if path.starts_with("/tmp/") {
               PathDestination::Scratch(path)
           } else {
               PathDestination::NotFound
           }
       }
   }
   ```

3. **Add to linker with shadowing**
   ```rust
   // In CoreShellExecutor or Conch
   wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
   linker.allow_shadowing(true);
   conch_vfs::add_to_linker(&mut linker)?;
   linker.allow_shadowing(false);
   ```

4. **The guest (brush shell) needs no changes**
   - Commands like `cat /ctx/self/tools/123/result.json` just work
   - `ls /ctx/self/tools/` lists tool calls
   - Pipes and redirects work normally: `cat /ctx/self/tools/*/result.json | jq .`

### Benefits of This Approach

1. **No custom WIT exports needed in the shell**
   - brush uses standard WASI, no modifications required
   - Any WASI-compatible tool could potentially run in the sandbox

2. **Transparent to the guest**
   - Shell scripts work exactly as they would with a real filesystem
   - Easy to test — can mock the ContextProvider

3. **Flexible routing**
   - Can mix virtual paths (`/ctx/`) with real paths (`/workspace/`)
   - Can add new virtual paths without changing the guest

4. **Works with wasip1 OR wasip2**
   - Same pattern works with either target
   - wasip2 is cleaner (native component model)
   - wasip1 works with adapter

### What We Already Have

The `ContextFs` in `crates/conch/src/vfs.rs` already implements:
- Path parsing (`/ctx/self/tools/<id>/result.json`, etc.)
- Access control (parent/child agent permissions)
- Lazy loading via `ContextProvider` trait
- Caching of tool call data

We just need to:
1. Create the WASI filesystem bindings (like `eryx-vfs/src/bindings.rs`)
2. Implement the Host traits using `ContextFs`
3. Wire it into the linker

### Comparison with Original Plan

The original WIT in `shell.wit` had custom VFS imports:
```wit
// OLD - removed
import vfs-read: func(path: string) -> result<list<u8>, string>;
import vfs-readdir: func(path: string) -> result<list<string>, string>;
```

This would have required:
- The shell to explicitly call these custom imports
- Modifying brush to use custom VFS calls instead of WASI
- Two separate code paths (WASI for real fs, custom for VFS)

The eryx pattern is much cleaner — the guest doesn't need to know about VFS at all.

## References

- `eryx/crates/eryx-vfs/` — Complete VFS implementation
- `eryx/crates/eryx-vfs/src/linker.rs` — How to add VFS to linker
- `eryx/crates/eryx-vfs/src/bindings.rs` — How to generate WASI host bindings
- `eryx/crates/eryx/src/wasm.rs` — How it's wired into the executor