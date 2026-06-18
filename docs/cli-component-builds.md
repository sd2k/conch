# Building CLI components (manifest-driven, multi-lane)

How conch turns an upstream CLI (gh, gcx, …) into a WASI component you can
register and run. Implements ADR #26 and issue #51.

## TL;DR

```sh
mise run list-clis           # show available manifests
mise run check-clis          # validate all manifests without building
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

## CI

- **`CLI Manifests`** job (`ci.yml`) — runs `conch-build --check` on every PR
  touching `clis/**` or `crates/conch-build/**`. Validates each manifest parses
  and its `vendor_patch` exists. Toolchain-free, so it always runs.
- **`Rebuild CLI components`** workflow (`rebuild-cli-components.yml`) — weekly +
  manual `workflow_dispatch`. Validates manifests, then rebuilds the Go-lane
  components to catch upstream/toolchain bit-rot (#29). The rebuild step is
  currently guarded: it skips with a notice until the pinned wasip3 Go toolchain
  image lands (#53), then it rebuilds gh/gcx for real and fails on breakage.

## Toolchains

The driver uses tools from `PATH` (mise provides them) plus the Go fork:

- **wasm-tools** — pinned in `mise.toml`. Must be **≥ 1.245.1**: older releases
  (e.g. 1.236.1) fail `component new` on the async `wasi:cli/run` world with
  "no export `run` found".
- **wasmtime** (CLI) — pinned to match the `wasmtime` crate in `Cargo.lock`
  (`mise run check-wasmtime-version`). Used for the optional cwasm step.
- **wasip3 Go fork** — our controlled fork **`sd2k/go`**, branch
  `wasip3-2026-03-15`, ported to the WASI `0.3.0-rc-2026-03-15` snapshot that
  wasmtime 44.x implements. `ci/bootstrap-go-wasip3.sh` clones + builds the
  pinned commit and prints the GOROOT; the driver finds it via
  `$CONCH_GO_WASIP3_ROOT` (or `scratch/go-wasip3`). Bump the pin in that script
  when moving the fork (issue #53).

## wasi p3 snapshot: keeping host and guest in lockstep (#53/#27)

Component instantiation is **version-exact**: the `gh` component must import the
same wasi p3 snapshot the host (the `wasmtime` crate) implements, or you get:

```
component imports instance `wasi:cli/...@0.3.0-rc-<DATE>`,
but a matching implementation was not found in the linker
```

| Component | wasi p3 snapshot |
|-----------|------------------|
| `wasmtime` crate 44.x (the conch host) | `0.3.0-rc-2026-03-15` |
| **sd2k/go fork @ `wasip3-2026-03-15`** (guest) | `0.3.0-rc-2026-03-15` ✅ |
| upstream jellevandenhooff/go @ a414d8c7 (old) | `0.3.0-rc-2026-02-09` ❌ |

When this de-synced (the de-pin to published wasmtime moved the host to `03-15`
while the upstream fork was on `02-09`), the fix was to **port the fork
forward** rather than pin wasmtime back (eryx-vfs requires `^44.0.2`, so
`=44.0.0` won't resolve). The port lives in `sd2k/go`; porting notes:
- the `02-09 → 03-15` deltas are `enum → variant` error-code/descriptor-type
  (filesystem, sockets) and a `random` param rename;
- wasm-tools spells async funcs `[async method]`/`[async]` in `03-15` — witgen
  was taught to normalize those to canonical names;
- componentizing the async `wasi:cli/run` export needs wasm-tools `>= 1.246.2`.

The `gh demo (wasip3)` workflow (`.github/workflows/gh-demo.yml`) rebuilds the
toolchain + `gh` and runs `gh --version` through conch in CI, so a future
host/fork de-sync fails loudly.
