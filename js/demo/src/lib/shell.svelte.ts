import { Shell, type ExecutionResult } from "@bsull/conch";
import { setFileData, fromPaths } from "@bsull/conch/vfs";

/**
 * Files seeded into the in-memory VFS so the demo has something to play with
 * (cat / grep / head / tail / wc / jq all operate on these).
 */
export const SEED_FILES: Record<string, string> = {
  "/data/fruits.txt": [
    "apple",
    "banana",
    "cherry",
    "date",
    "elderberry",
    "fig",
    "grape",
  ].join("\n") + "\n",
  "/data/people.json": JSON.stringify(
    [
      { name: "Ada", role: "engineer", langs: ["rust", "wasm"] },
      { name: "Grace", role: "admiral", langs: ["cobol"] },
      { name: "Linus", role: "maintainer", langs: ["c"] },
    ],
    null,
    2,
  ) + "\n",
  "/data/poem.txt": [
    "the shell within the page",
    "no server and no cage",
    "just bytes that run",
    "a sandboxed sun",
    "compiled for the modern age",
  ].join("\n") + "\n",
  "/notes/todo.md": [
    "# todo",
    "- [x] compile bash to wasm",
    "- [x] run it in the browser",
    "- [] take over the world",
  ].join("\n") + "\n",
};

export type ShellState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; shell: Shell; loadMs: number };

let state = $state<ShellState>({ status: "loading" });

export function getShellState(): ShellState {
  return state;
}

/**
 * Instantiate the shell and seed the VFS. Synchronous from the caller's point
 * of view — the WASM core is already compiled by the time `@bsull/conch`'s
 * module-level top-level await resolves (handled by the dynamic import below).
 */
export async function initShell(): Promise<void> {
  const start = performance.now();
  try {
    // Dynamic import so the module's top-level await (WASM compile) is awaited
    // before we touch the API.
    const { Shell } = await import("@bsull/conch");
    setFileData(fromPaths(SEED_FILES));
    const shell = new Shell();
    state = {
      status: "ready",
      shell,
      loadMs: Math.round(performance.now() - start),
    };
  } catch (e) {
    state = { status: "error", message: (e as Error).message ?? String(e) };
  }
}

/** Run one line of shell input. Throws only if the shell itself fails. */
export function runLine(script: string): ExecutionResult {
  if (state.status !== "ready") {
    throw new Error("shell not ready");
  }
  return state.shell.execute(script);
}
