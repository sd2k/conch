/**
 * Process (subprocess spawning) shim for conch-shell.
 *
 * Subprocess component spawning is a host-mediated feature that runs
 * separate WASI components. It is not available in browser/Node.js
 * environments, so all operations return command-not-found errors.
 */

export class Child {
  /**
   * @param {string} cmd
   * @param {string[]} args
   * @param {[string, string][]} env
   * @param {string} cwd
   */
  static spawn(_cmd, _args, _env, _cwd) {
    return { tag: 'err', val: 'command-not-found' };
  }

  /** @param {Uint8Array} data */
  writeStdin(_data) {
    return { tag: 'err', val: 'io-error' };
  }

  closeStdin() {}

  /** @param {number} maxBytes */
  readStdout(_maxBytes) {
    return { tag: 'ok', val: new Uint8Array() };
  }

  /** @param {number} maxBytes */
  readStderr(_maxBytes) {
    return { tag: 'ok', val: new Uint8Array() };
  }

  wait() {
    return { tag: 'err', val: 'io-error' };
  }
}
