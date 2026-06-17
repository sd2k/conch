<script lang="ts">
  import { onMount } from "svelte";
  import Terminal from "./lib/Terminal.svelte";
  import { getShellState, initShell } from "./lib/shell.svelte";

  let state = $derived(getShellState());

  let inject: ((cmd: string, run?: boolean) => void) | null = $state(null);

  const EXAMPLES: { label: string; cmd: string }[] = [
    { label: "hello", cmd: 'echo "hello from wasm"' },
    { label: "pipe + wc", cmd: "cat /data/fruits.txt | grep a | wc -l" },
    { label: "jq", cmd: "cat /data/people.json | jq '.[].name'" },
    { label: "for loop", cmd: "for i in 1 2 3; do echo \"line $i\"; done" },
    { label: "variables", cmd: 'name=conch; echo "shell: $name"' },
    { label: "function", cmd: 'greet() { echo "hi, $1!"; }; greet world' },
    { label: "head", cmd: "head -n 2 /data/poem.txt" },
  ];

  function runExample(cmd: string) {
    inject?.(cmd, true);
  }

  onMount(() => {
    initShell();
  });
</script>

<main>
  <header>
    <h1>conch <span class="sub">a bash shell in WebAssembly</span></h1>
    <p class="tagline">
      A sandboxed, bash-compatible shell compiled to WASM with
      <a href="https://github.com/sd2k/conch" target="_blank" rel="noreferrer"
        >conch</a
      >. Runs entirely in your browser — no server, no installs.
    </p>
  </header>

  <div class="status" class:ready={state.status === "ready"} class:err={state.status === "error"}>
    {#if state.status === "loading"}
      <span class="dot" ></span> loading shell…
    {:else if state.status === "ready"}
      <span class="dot"></span> ready in {state.loadMs} ms
    {:else}
      <span class="dot"></span> failed to load: {state.message}
    {/if}
  </div>

  <div class="examples">
    {#each EXAMPLES as ex}
      <button
        disabled={state.status !== "ready"}
        title={ex.cmd}
        onclick={() => runExample(ex.cmd)}>{ex.label}</button
      >
    {/each}
  </div>

  <div class="term-wrap">
    <Terminal bind:inject />
  </div>

  <footer>
    No networking, real files, or subprocesses — see the
    <a href="https://github.com/sd2k/conch/issues/54" target="_blank" rel="noreferrer">demo issue</a>
    for what's in scope.
  </footer>
</main>

<style>
  :global(body) {
    margin: 0;
    background: #010409;
    color: #c9d1d9;
    font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
  }

  main {
    max-width: 900px;
    margin: 0 auto;
    padding: 2rem 1.25rem 3rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  header h1 {
    margin: 0;
    font-size: 1.8rem;
    color: #58a6ff;
  }
  .sub {
    font-size: 1rem;
    font-weight: 400;
    color: #8b949e;
  }
  .tagline {
    margin: 0.4rem 0 0;
    color: #8b949e;
    line-height: 1.5;
  }
  a {
    color: #58a6ff;
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.85rem;
    color: #8b949e;
  }
  .status .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #d29922;
  }
  .status.ready .dot {
    background: #3fb950;
  }
  .status.err .dot {
    background: #f85149;
  }
  .status.err {
    color: #f85149;
  }

  .examples {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .examples button {
    background: #161b22;
    color: #c9d1d9;
    border: 1px solid #30363d;
    border-radius: 999px;
    padding: 0.35rem 0.85rem;
    font-size: 0.85rem;
    cursor: pointer;
    transition: border-color 0.12s, background 0.12s;
  }
  .examples button:hover:not(:disabled) {
    border-color: #58a6ff;
    background: #1f2630;
  }
  .examples button:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .term-wrap {
    height: 460px;
    border: 1px solid #30363d;
    border-radius: 8px;
    overflow: hidden;
  }

  footer {
    font-size: 0.8rem;
    color: #6e7681;
  }
</style>
