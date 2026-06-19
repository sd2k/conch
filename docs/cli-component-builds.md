# Building CLI components (manifest-driven, multi-lane)

How conch turns an upstream CLI (gh, gcx, …) into a WASI component you can
register and run. Implements ADR #26 and issue #51.

## TL;DR

```sh
mise run list-clis            # show available manifests
mise run check-clis           # validate all manifests without building
mise run build-cli -- gh      # build clis/gh.toml → scratch/gh-component/component.wasm
mise run demo-sqlite          # build sqlite3 (C lane, single) + run a query through conch
mise run demo-curl            # build curl (C lane, cmake) + fetch http:// through conch
mise run demo-ripgrep         # build rg (Rust lane) + search a mounted dir through conch
mise run demo-coreutils       # build uutils coreutils (Rust lane) + run utils through conch
```

Each CLI is described by a manifest in `clis/<name>.toml`. The `conch-build`
driver (`crates/conch-build`) reads a manifest and dispatches to the lane named
by its `lang` field. Adding a CLI is a config change + a compile spike, not a
bespoke script.

## Lanes (per ADR #26)

| Lane | `lang` | Toolchain | Target | Status |
|------|--------|-----------|--------|--------|
| Rust | `rust` | cargo (mise-pinned) | wasip2 | implemented (ripgrep #85; coreutils #86) |
| C/C++ | `c` | wasi-sdk `wasm32-wasip2` clang | wasip2 | implemented (sqlite3 #52; curl plaintext HTTP #79) |
| Go | `go` | wasip3 Go fork + wasm-tools | wasip3 | implemented (gh, gcx) |

### Rust lane (cargo → wasip2)

`mise run demo-ripgrep` builds `rg` and runs a search through conch. The simplest
lane: since Rust 1.82 the `wasm32-wasip2` target **emits a component directly**,
so the lane is just `cargo build --release --target wasm32-wasip2 --bin <bin>`,
then the artifact `target/wasm32-wasip2/release/<bin>.wasm` is the component — no
`wasm-tools` step, no wasi-sdk. Config: `[build] bin` (required), optional
`cargo_flags`, and `[build.env]` (extra env for the cargo invocation). The
`wasm32-wasip2` rustup target must be installed (the mise `wasm-target` task);
cargo comes from PATH.

**coreutils (#86):** `mise run demo-coreutils` builds **uutils** as a multicall
`coreutils` component (`clis/coreutils.toml`) that replaces conch-shell's
hand-rolled `cat`/`head`/`tail`/`ls`/`wc`/`cp`/`mv`/`rm`/`mkdir`/`touch` builtins
(plus the missing `sort`). Registered under each util name, it dispatches on
`argv[0]` (conch sets it to the command name). Two wrinkles, both in the
manifest: `uucore` uses an unstable `std::os::wasi` API on wasip2, so a
`vendor_patch` adds `#![feature(wasip2)]` and `[build.env] RUSTC_BOOTSTRAP="1"`
lets the pinned stable toolchain accept it; and `[cwasm] enabled` is **important
for perf** — the `.wasm` path recompiles the ~5 MB module on every spawn (~141 ms
vs ~8 ms for the precompiled cwasm; build under mise so `wasmtime` is the pinned
44.x). Needs the `I32Exit` exit-handling fix (uutils calls `std::process::exit`).
`grep`/`jq` stay shell builtins (not coreutils).

**ripgrep spike findings (#85):** builds and runs clean with **no source patches
and no special flags** — the cleanest candidate yet. The default build uses the
pure-Rust regex engine (`features:-pcre2`), and ripgrep's mmap usage
**auto-falls-back** on wasm, so `--no-mmap` isn't needed. Verified end-to-end
through conch: `rg -n main /proj` finds matches in a mounted dir, and
`rg … | upper` pipes. (`rg` is the most-used external command in real sessions —
see epic #84.)

### C lane (wasi-sdk → wasip2)

`mise run demo-sqlite` builds the `sqlite3` CLI from the amalgamation and runs a
query through conch. The lane is a **single `clang` invocation**: wasi-sdk's
`wasm32-wasip2` target links via `wasm-component-ld`, so clang emits a
**component directly** (imports `wasi:*@0.2.x`, exports `wasi:cli/run`) — no
`wasm-tools component new` step, unlike the Go lane.

The SDK is installed on demand by `mise run ensure-wasi-sdk` and kept **out of
`[tools]`/PATH** — a wasm clang on PATH makes host proc-macro builds pick the
wrong compiler (eryx's hard-won note). Build tasks export `WASI_SDK_PATH`; the
lane reads it from the environment.

The C lane has two build systems, set by `[build] system`:

- **`single`** (default) — one `clang` call over `[build] sources`
  (amalgamation-style, e.g. sqlite3). Config: `sources`, `cflags`, `link_flags`.
- **`cmake`** — configure + build a CMake project with the wasi-sdk wasip2
  toolchain file, then copy `[build] artifact` (a build-dir-relative path) to
  `component.wasm`. Config adds `cmake_flags`; `cflags`/`link_flags` are passed
  via `CMAKE_C_FLAGS`/`CMAKE_EXE_LINKER_FLAGS`. Used by curl. `cmake` + `ninja`
  must be on PATH (the wasi clang still comes from the toolchain file).

See `clis/sqlite3.toml` (single) and `clis/curl.toml` (cmake) for worked
examples of the WASI workarounds a real C CLI needs.

**sqlite3 spike findings (#52):** compiles clean once you (a) link the wasi-sdk
emulation libs for `signal`/`getpid`, (b) define `__minux` to drop the
`getrusage`-based `.timer` (WASI has no `getrusage`), and (c) set
`SQLITE_NOHAVE_SYSTEM`/`SQLITE_THREADSAFE=0`/`OMIT_LOAD_EXTENSION` to avoid
`system()`, pthreads, and `dlopen`. Verified end-to-end through conch:
in-memory queries, SQL via stdin, pipelines (`sqlite3 … | upper`), **and
file-backed databases** — the unix VFS's `fcntl` locking works over the WASI
filesystem, so the "watch for locking" risk did not bite. (jq, the original
candidate, was dropped: the shell already embeds jaq, so a jq component is
redundant; sqlite is non-redundant and an easier amalgamation build.)

**curl spike findings (#79):** `mise run demo-curl` builds curl (CMake, SSL +
all optional deps off) and runs `curl --version` and `curl -sSI http://…`
through conch. **Plaintext HTTP works end-to-end** — DNS + TCP connect are
served by the host's `wasi:sockets` (the conch child gets `inherit_network` +
`allow_ip_name_lookup`). The build needs, in `clis/curl.toml`:
- `-DPOLLPRI=0` (WASI `poll.h` lacks the OOB-poll flag),
- `-D_WASI_EMULATED_SIGNAL` + `-lwasi-emulated-signal`,
- `-mllvm -wasm-enable-sjlj` so `setjmp.h` compiles (the alarm-based DNS-timeout
  path includes it unconditionally; it is never actually exercised),
- `-DCURL_DISABLE_SOCKETPAIR` — WASI has no `socketpair`, and curl's TCP-based
  fallback can't run, so the multi-handle wakeup must be disabled, and
- a one-line `vendor_patch` disabling `getsockname` in `set_local_ip()`:
  wasi-libc's `getsockname` **`abort()`s** on an unmappable host error instead
  of returning -1, which would otherwise crash every transfer. The local-IP
  info it gathers is non-essential.

**TLS/HTTPS (M3) is not done yet** — the host exposes raw TCP (`wasi:sockets`),
not `wasi:http`, so TLS must be linked into the guest (a wasi-buildable
mbedTLS/wolfSSL). That's the next step for #79.

## Manifest format

See `clis/gh.toml` and `clis/gcx.toml`. Fields:

- `name`, `description`, `lang` (`rust|c|go`)
- `[source]` `repo`, `ref` (pinned tag/commit), `dir` (local working copy)
- `[build]`
  - Rust lane: `bin` (cargo binary; artifact is `<bin>.wasm`), optional
    `cargo_flags`, `[build.env]`; see `clis/rg.toml`, `clis/coreutils.toml`
  - Go lane: `package` (what to build), `vendor_patch` (script run after vendoring)
  - C lane: `system` (`single`|`cmake`); `single` uses `sources`/`cflags`/
    `link_flags`; `cmake` uses `cmake_flags` + `artifact`. `vendor_patch` runs
    before compiling. See `clis/sqlite3.toml` and `clis/curl.toml`
- `[component]` `world` (Go lane's wasm-tools embed world, e.g. `command`;
  optional, defaults to `command` — the C lane ignores it)
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

- **wasi-sdk** (C lane) — pinned via `[vars] wasi_sdk_version` in `mise.toml`,
  installed on demand by `mise run ensure-wasi-sdk`. Kept **off** `[tools]`/PATH
  on purpose (a wasm clang on PATH breaks host proc-macro builds).
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
