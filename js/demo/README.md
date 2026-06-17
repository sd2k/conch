# conch browser demo

An interactive in-page terminal ([xterm.js](https://xtermjs.org/)) wired to the
[`@bsull/conch`](../conch-shell) npm package. Type bash commands and watch them
run in the WebAssembly sandbox — no server, no installs.

Scope is the shell + builtins (`echo`, `cat`, `ls`, `grep`, `head`, `tail`,
`wc`, `jq` via jaq, filesystem builtins `mkdir`/`touch`/`rm`/`cp`/`mv`, `test`,
`cd`, `pwd`, `export`, loops/functions/conditionals) over an in-memory virtual
filesystem. Not yet implemented: `sed`, `awk`, `sort`, `printf`, `cut`, `tr`,
`find`, `seq`. **Not** in scope: networking, real files, or subprocesses (`gh`
and friends need the wasmtime host — see issue #54).

## Running

From the repo root (uses mise's monorepo tasks; builds the WASM + transpiles
first, then starts Vite):

```bash
MISE_EXPERIMENTAL=1 mise run //js:demo          # dev server
MISE_EXPERIMENTAL=1 mise run //js:demo-build     # production build to dist/
```

Or directly with npm (requires `//js:build` to have produced the
`../conch-shell` artifacts first):

```bash
cd js/demo
npm install
npm run dev        # http://localhost:5173
npm run build      # -> dist/
npm run preview
```

## How it works

- `src/lib/shell.svelte.ts` instantiates the stateful `Shell`, seeds the VFS
  with a few sample files (`setFileData(fromPaths(...))`), and exposes a
  synchronous `runLine()`. `Shell.execute()` is synchronous, so no JSPI /
  `SharedArrayBuffer` (and therefore no COOP/COEP headers) are required.
- `src/lib/Terminal.svelte` is a hand-rolled xterm.js line editor: prompt,
  cursor movement, backspace, history (↑/↓), `Ctrl-C`/`Ctrl-L`/`Ctrl-A`/`Ctrl-E`/`Ctrl-U`,
  plus JS-side `clear`/`help`. Each entered line is handed to `runLine()` and the
  result is written back to the terminal.
- `src/App.svelte` shows load status and example "pill" buttons that drop a
  command into the prompt and run it.

`@bsull/conch` is referenced via a `file:` link to the local `../conch-shell`
package so the demo always reflects the current build. The Vite config aliases
`@bytecodealliance/preview2-shim/*` to that package's browser shims (mirroring
`js/testpkg/vitest.config.ts`).
