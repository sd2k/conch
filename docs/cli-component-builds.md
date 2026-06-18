# Building CLI components (manifest-driven, multi-lane)

How conch turns an upstream CLI (gh, gcx, …) into a WASI component you can
register and run. Implements ADR #26 and issue #51.

## TL;DR

```sh
mise run list-clis           # show available manifests
mise run build-cli -- gh     # build clis/gh.toml → scratch/gh-component/component.wasm
```

Each CLI is described by a manifest in `clis/<name>.toml`. The `conch-build`
driver (`crates/conch-build`) reads a manifest and dispatches to the lane named
by its `lang` field. Adding a CLI is a config change + a compile spike, not a
bespoke script.

## Lanes (per ADR #26)

| Lane | `lang` | Toolchain | Target | Status |
|------|--------|-----------|--------|--------|
| Rust | `rust` | cargo (mise-pinned) | wasip2 | stub — needs a spike (#51) |
| C/C++ | `c` | wasi-sdk `wasm32-wasip2-clang` | wasip2 | stub — #52 (jq), #79 (curl) |
| Go | `go` | wasip3 Go fork + wasm-tools | wasip3 | implemented (gh, gcx) |

The Rust/C lanes currently `bail!` with a pointer to their tracking issue; the
structure is in place so wiring them up is a focused change.

## Manifest format

See `clis/gh.toml` and `clis/gcx.toml`. Fields:

- `name`, `description`, `lang` (`rust|c|go`)
- `[source]` `repo`, `ref` (pinned tag/commit), `dir` (local working copy)
- `[build]` `package` (what to build), `vendor_patch` (script run after vendoring)
- `[component]` `world` (wasm-tools embed world, e.g. `command`)
- `[cwasm]` `enabled`, `flags` (extra `wasmtime compile` flags)
- `[output]` `dir`

## Toolchains

The driver uses tools from `PATH` (mise provides them) plus the Go fork:

- **wasm-tools** — pinned in `mise.toml`. Must be **≥ 1.245.1**: older releases
  (e.g. 1.236.1) fail `component new` on the async `wasi:cli/run` world with
  "no export `run` found".
- **wasmtime** (CLI) — pinned to match the `wasmtime` crate in `Cargo.lock`
  (`mise run check-wasmtime-version`). Used for the optional cwasm step.
- **wasip3 Go fork** — `jellevandenhooff/go`, not yet pinned/hermetic. The driver
  looks for it at `scratch/go-wasip3` or `$CONCH_GO_WASIP3_ROOT`. Making this
  reproducible (a pinned image) is issue #53.

## Known issue: wasi p3 snapshot skew (blocks `gh` at runtime) — #53/#27

The Go lane **builds** `gh` reproducibly today, but the resulting component does
not yet **instantiate** in the conch host:

```
component imports instance `wasi:cli/environment@0.3.0-rc-2026-02-09`,
but a matching implementation was not found in the linker
```

Cause — a wasi Preview 3 snapshot mismatch between the host and the guest:

| Component | wasi p3 snapshot |
|-----------|------------------|
| go-wasip3 fork @ a414d8c7 (guest WIT) | `0.3.0-rc-2026-02-09` |
| source-built wasmtime 44.0.0 (old path dep, demo worked) | `0.3.0-rc-2026-02-09` ✅ |
| **crates.io wasmtime 44.0.3 (current crate)** | **`0.3.0-rc-2026-03-15`** ❌ |

The original `gh pr list | upper` demo worked because the host (scratch wasmtime
44.0.0) and the Go fork agreed on the `02-09` snapshot. De-pinning to the
published wasmtime `44.x` advanced the host to `03-15`, ahead of the fork.

**Fix (toolchain work, tracked in #53):** advance the wasip3 Go fork to a commit
whose bundled WIT targets `0.3.0-rc-2026-03-15`, rebuild the fork, and re-run
`mise run build-cli -- gh`. (Pinning the wasmtime crate back to `=44.0.0`
conflicts with eryx-vfs's `^44` requirement, so moving the fork forward is the
right direction.)
