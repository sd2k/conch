<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import { runLine, SEED_FILES } from "./shell.svelte";

  // Bound from the parent so example pills can inject a command.
  let { inject = $bindable<((cmd: string, run?: boolean) => void) | null>(null) } =
    $props();

  const PROMPT = "\x1b[1;32m$\x1b[0m ";

  let host: HTMLDivElement;
  let term: Terminal;
  let fit: FitAddon;

  // Line-editor state.
  let line = "";
  let cursor = 0; // index within `line`
  const history: string[] = [];
  let historyIdx = -1; // -1 == editing a fresh line
  let stash = ""; // in-progress line stashed when browsing history

  const nl = (s: string) => s.replace(/\n/g, "\r\n");

  function writePrompt() {
    term.write(PROMPT);
  }

  function banner() {
    const files = Object.keys(SEED_FILES).join("  ");
    term.writeln("\x1b[1;36mconch\x1b[0m — a bash-compatible shell in WebAssembly");
    term.writeln("");
    term.writeln("Everything runs in your browser, in a sandboxed virtual filesystem.");
    term.writeln("Try:  \x1b[33mecho hello | wc -c\x1b[0m   ·   \x1b[33mcat /data/people.json | jq '.[].name'\x1b[0m");
    term.writeln("");
    term.writeln(`Seeded files: \x1b[2m${files}\x1b[0m`);
    term.writeln("Builtins: echo cat ls head tail wc grep jq mkdir touch rm cp mv cd pwd export");
    term.writeln("Type \x1b[33mhelp\x1b[0m for more, \x1b[33mclear\x1b[0m to reset the screen.");
    term.writeln("");
  }

  function help() {
    term.writeln("conch demo — interactive bash shell compiled to WebAssembly.");
    term.writeln("");
    term.writeln("State persists across lines: variables, functions and aliases stick.");
    term.writeln("  \x1b[33mgreeting=hello\x1b[0m then \x1b[33mecho $greeting\x1b[0m");
    term.writeln("  \x1b[33mgreet() { echo \"hi, $1\"; }\x1b[0m then \x1b[33mgreet world\x1b[0m");
    term.writeln("");
    term.writeln("Pipes, loops and conditionals work:");
    term.writeln("  \x1b[33mfor i in 1 2 3; do echo $i; done\x1b[0m");
    term.writeln("  \x1b[33mgrep a /data/fruits.txt | wc -l\x1b[0m");
    term.writeln("");
    term.writeln("Not implemented yet: sed, awk, sort, printf, cut, tr, find, seq.");
    term.writeln("No networking or subprocesses — gh and friends need the wasmtime host.");
    term.writeln("");
  }

  /** Redraw the current input line in place (single visual line). */
  function refresh() {
    term.write("\r" + PROMPT + line + "\x1b[K");
    const back = line.length - cursor;
    if (back > 0) term.write(`\x1b[${back}D`);
  }

  function setLine(next: string, cur = next.length) {
    line = next;
    cursor = cur;
    refresh();
  }

  function execute(cmd: string) {
    const trimmed = cmd.trim();
    if (trimmed) {
      history.push(cmd);
      if (history.length > 500) history.shift();
    }
    historyIdx = -1;
    stash = "";

    if (!trimmed) {
      writePrompt();
      return;
    }

    // Convenience commands handled on the JS side.
    if (trimmed === "clear") {
      term.clear();
      writePrompt();
      return;
    }
    if (trimmed === "help") {
      help();
      writePrompt();
      return;
    }

    try {
      const result = runLine(cmd);
      if (result.stdout) term.write(nl(result.stdout));
      if (result.stderr) term.write(`\x1b[31m${nl(result.stderr)}\x1b[0m`);
    } catch (e) {
      term.writeln(`\x1b[31mshell error: ${(e as Error).message}\x1b[0m`);
    }
    writePrompt();
  }

  function onData(data: string) {
    // Multi-byte / pasted input: handle the common control sequences first.
    switch (data) {
      case "\r": // Enter
        term.write("\r\n");
        execute(line);
        line = "";
        cursor = 0;
        return;
      case "\x7f": // Backspace
        if (cursor > 0) {
          setLine(line.slice(0, cursor - 1) + line.slice(cursor), cursor - 1);
        }
        return;
      case "\x03": // Ctrl-C
        term.write("^C\r\n");
        line = "";
        cursor = 0;
        historyIdx = -1;
        writePrompt();
        return;
      case "\x0c": // Ctrl-L
        term.clear();
        refresh();
        return;
      case "\x01": // Ctrl-A — start of line
        cursor = 0;
        refresh();
        return;
      case "\x05": // Ctrl-E — end of line
        cursor = line.length;
        refresh();
        return;
      case "\x15": // Ctrl-U — clear line
        setLine("", 0);
        return;
      case "\x1b[A": // Up
        historyPrev();
        return;
      case "\x1b[B": // Down
        historyNext();
        return;
      case "\x1b[C": // Right
        if (cursor < line.length) {
          cursor++;
          refresh();
        }
        return;
      case "\x1b[D": // Left
        if (cursor > 0) {
          cursor--;
          refresh();
        }
        return;
    }

    // Ignore other escape sequences / control chars.
    if (data.charCodeAt(0) < 0x20 && data !== "\t") return;

    // Printable text (possibly a multi-char paste). Strip newlines from pastes.
    const text = data.replace(/[\r\n]+/g, " ");
    setLine(line.slice(0, cursor) + text + line.slice(cursor), cursor + text.length);
  }

  function historyPrev() {
    if (history.length === 0) return;
    if (historyIdx === -1) {
      stash = line;
      historyIdx = history.length - 1;
    } else if (historyIdx > 0) {
      historyIdx--;
    }
    setLine(history[historyIdx]);
  }

  function historyNext() {
    if (historyIdx === -1) return;
    if (historyIdx < history.length - 1) {
      historyIdx++;
      setLine(history[historyIdx]);
    } else {
      historyIdx = -1;
      setLine(stash);
    }
  }

  // Exposed to the parent: drop a command into the prompt, optionally run it.
  function injectCommand(cmd: string, run = false) {
    term.focus();
    if (run) {
      term.write("\r" + PROMPT + cmd + "\r\n");
      execute(cmd);
      line = "";
      cursor = 0;
    } else {
      setLine(cmd);
    }
  }

  onMount(() => {
    term = new Terminal({
      fontFamily:
        'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
      fontSize: 14,
      cursorBlink: true,
      convertEol: false,
      theme: {
        background: "#0d1117",
        foreground: "#c9d1d9",
        cursor: "#58a6ff",
        selectionBackground: "#264f78",
      },
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new WebLinksAddon());
    term.open(host);
    fit.fit();

    banner();
    writePrompt();

    term.onData(onData);
    inject = injectCommand;

    const ro = new ResizeObserver(() => fit.fit());
    ro.observe(host);
    return () => ro.disconnect();
  });

  onDestroy(() => term?.dispose());
</script>

<div class="terminal" bind:this={host}></div>

<style>
  .terminal {
    width: 100%;
    height: 100%;
    padding: 0.5rem;
    box-sizing: border-box;
    background: #0d1117;
    border-radius: 8px;
  }
</style>
