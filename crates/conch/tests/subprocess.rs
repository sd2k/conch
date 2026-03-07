//! Integration tests for subprocess component spawning.
//!
//! These tests verify the end-to-end flow:
//! 1. Host creates a shell instance with a component registry
//! 2. Shell encounters an unknown command
//! 3. Guest calls back to host via WIT process interface
//! 4. Host instantiates the command as a WASI component
//! 5. Stdio is piped and exit code propagated

#![cfg(feature = "embedded-shell")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use conch::{ComponentRegistry, ComponentShellExecutor, ResourceLimits};
use eryx_vfs::{ArcStorage, DirPerms, FilePerms, HybridVfsCtx, InMemoryStorage};

/// Path to the pre-built conch-test-cmd WASM component.
const TEST_CMD_WASM: &str = env!("CARGO_MANIFEST_DIR");

fn limits() -> ResourceLimits {
    ResourceLimits::default()
}

/// Load the conch-test-cmd WASM bytes from the build output.
fn load_test_cmd_bytes() -> Vec<u8> {
    let wasm_path = format!(
        "{}/../../target/wasm32-wasip2/release/conch-test-cmd.wasm",
        TEST_CMD_WASM
    );
    std::fs::read(&wasm_path)
        .unwrap_or_else(|e| panic!("Failed to read {wasm_path}: {e}. Run: cargo build -p conch-test-cmd --target wasm32-wasip2 --release"))
}

async fn create_shell_with_registry(
    executor: &ComponentShellExecutor,
    registry: Arc<ComponentRegistry>,
) -> conch::ShellInstance<ArcStorage> {
    let limits = limits();
    let storage = ArcStorage::new(std::sync::Arc::new(InMemoryStorage::new()));
    let mut hybrid_ctx = HybridVfsCtx::new(storage);
    hybrid_ctx.add_vfs_preopen("/scratch", DirPerms::all(), FilePerms::all());

    executor
        .create_instance_with_registry(&limits, hybrid_ctx, None, registry)
        .await
        .expect("Failed to create shell instance with registry")
}

/// Direct test — tests the child component with epoch interruption (matching real config).
#[tokio::test]
async fn test_child_component_with_epoch() {
    use wasmtime::component::{Component, Linker};
    use wasmtime::{Config, Engine, Store};
    use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
    use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

    struct State {
        wasi: WasiCtx,
        table: ResourceTable,
    }
    impl WasiView for State {
        fn ctx(&mut self) -> WasiCtxView<'_> {
            WasiCtxView {
                ctx: &mut self.wasi,
                table: &mut self.table,
            }
        }
    }

    let mut config = Config::new();
    config.async_support(true);
    config.epoch_interruption(true); // Match parent engine config
    let engine = Engine::new(&config).unwrap();

    let wasm_bytes = load_test_cmd_bytes();
    let component = Component::new(&engine, &wasm_bytes).unwrap();

    // Run on a separate thread (matching actual child code path)
    let result = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let stdin = MemoryInputPipe::new(b"hello\n".to_vec());
            let stdout = MemoryOutputPipe::new(1024 * 1024);
            let stderr = MemoryOutputPipe::new(1024 * 1024);

            let wasi = WasiCtxBuilder::new()
                .stdin(stdin)
                .stdout(stdout.clone())
                .stderr(stderr.clone())
                .build();

            let mut store = Store::new(
                &engine,
                State {
                    wasi,
                    table: ResourceTable::new(),
                },
            );
            store.set_epoch_deadline(1_000_000_000);

            let mut linker = Linker::<State>::new(&engine);
            wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();

            let cmd = wasmtime_wasi::p2::bindings::Command::instantiate_async(
                &mut store, &component, &linker,
            )
            .await
            .unwrap();

            let result = cmd.wasi_cli_run().call_run(&mut store).await;
            let contents = stdout.contents();
            let output = String::from_utf8_lossy(&contents).to_string();
            (result, output)
        })
    })
    .await
    .unwrap();

    assert!(result.0.is_ok(), "call_run failed: {:?}", result.0);
    assert!(
        result.1.contains("HELLO"),
        "Expected HELLO, got: {:?}",
        result.1
    );
}

/// Direct test that bypasses the shell — just tests the child component execution.
#[tokio::test]
async fn test_child_component_direct() {
    use wasmtime::component::{Component, Linker};
    use wasmtime::{Config, Engine, Store};
    use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
    use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

    struct State {
        wasi: WasiCtx,
        table: ResourceTable,
    }
    impl WasiView for State {
        fn ctx(&mut self) -> WasiCtxView<'_> {
            WasiCtxView {
                ctx: &mut self.wasi,
                table: &mut self.table,
            }
        }
    }

    let mut config = Config::new();
    config.async_support(true);
    let engine = Engine::new(&config).unwrap();

    let wasm_bytes = load_test_cmd_bytes();
    let component = Component::new(&engine, &wasm_bytes).unwrap();

    let stdin = MemoryInputPipe::new(b"hello world\n".to_vec());
    let stdout = MemoryOutputPipe::new(1024 * 1024);
    let stderr = MemoryOutputPipe::new(1024 * 1024);

    let wasi = WasiCtxBuilder::new()
        .stdin(stdin)
        .stdout(stdout.clone())
        .stderr(stderr.clone())
        .build();

    let mut store = Store::new(
        &engine,
        State {
            wasi,
            table: ResourceTable::new(),
        },
    );

    let mut linker = Linker::<State>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();

    let cmd =
        wasmtime_wasi::p2::bindings::Command::instantiate_async(&mut store, &component, &linker)
            .await
            .unwrap();

    let result = cmd.wasi_cli_run().call_run(&mut store).await;
    assert!(result.is_ok(), "call_run failed: {result:?}");

    let contents = stdout.contents();
    let output = String::from_utf8_lossy(&contents);
    assert!(
        output.contains("HELLO WORLD"),
        "Expected HELLO WORLD, got: {output:?}"
    );
}

#[tokio::test]
async fn test_subprocess_echo_pipe_upper() {
    let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");

    // Register the test command as "upper"
    let mut registry = ComponentRegistry::new();
    registry.register_wasm("upper", load_test_cmd_bytes());

    let mut instance = create_shell_with_registry(&executor, Arc::new(registry)).await;
    let limits = limits();

    // Execute: echo hello | upper
    // The echo builtin writes "hello\n" to stdout, which becomes stdin for "upper".
    // The upper command reads stdin, uppercases it, and writes to stdout.
    let result = instance
        .execute("echo hello | upper", &limits)
        .await
        .expect("execute failed");

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);

    assert_eq!(
        result.exit_code, 0,
        "Expected exit code 0, got {}. stdout: {stdout:?}, stderr: {stderr:?}",
        result.exit_code,
    );
    assert!(
        stdout.contains("HELLO"),
        "Expected stdout to contain 'HELLO', got: {stdout:?}, stderr: {stderr:?}",
    );
}

#[tokio::test]
async fn test_subprocess_empty_stdin() {
    let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");

    let mut registry = ComponentRegistry::new();
    registry.register_wasm("upper", load_test_cmd_bytes());

    let mut instance = create_shell_with_registry(&executor, Arc::new(registry)).await;
    let limits = limits();

    // Run upper with no stdin — should produce empty output
    let result = instance
        .execute("echo -n '' | upper", &limits)
        .await
        .expect("execute failed");

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);

    assert_eq!(
        result.exit_code, 0,
        "Expected exit code 0, got {}. stderr: {stderr:?}",
        result.exit_code,
    );
    // Empty input → empty output
    assert!(
        stdout.trim().is_empty(),
        "Expected empty stdout, got: {stdout:?}",
    );
}

#[tokio::test]
async fn test_subprocess_exit_code_propagation() {
    let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");

    let mut registry = ComponentRegistry::new();
    registry.register_wasm("upper", load_test_cmd_bytes());

    let mut instance = create_shell_with_registry(&executor, Arc::new(registry)).await;
    let limits = limits();

    // upper should exit 0
    let result = instance
        .execute("echo test | upper", &limits)
        .await
        .expect("execute failed");

    assert_eq!(result.exit_code, 0, "upper should exit 0");
}

#[tokio::test]
async fn test_subprocess_command_not_found() {
    let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");

    // Empty registry — no commands available
    let registry = ComponentRegistry::new();

    let mut instance = create_shell_with_registry(&executor, Arc::new(registry)).await;
    let limits = limits();

    // Try to run a command that doesn't exist in the registry
    let result = instance
        .execute("nonexistent_cmd hello", &limits)
        .await
        .expect("execute failed");

    // Should fail with non-zero exit code
    assert_ne!(
        result.exit_code, 0,
        "Expected non-zero exit code for unknown command",
    );
}

#[tokio::test]
async fn test_subprocess_multiline_input() {
    let executor = ComponentShellExecutor::embedded().expect("Failed to create executor");

    let mut registry = ComponentRegistry::new();
    registry.register_wasm("upper", load_test_cmd_bytes());

    let mut instance = create_shell_with_registry(&executor, Arc::new(registry)).await;
    let limits = limits();

    // Multi-line input using echo with -e
    let result = instance
        .execute("echo -e 'hello\\nworld' | upper", &limits)
        .await
        .expect("execute failed");

    let stdout = String::from_utf8_lossy(&result.stdout);

    assert_eq!(result.exit_code, 0);
    assert!(
        stdout.contains("HELLO"),
        "Expected HELLO in output: {stdout:?}",
    );
    assert!(
        stdout.contains("WORLD"),
        "Expected WORLD in output: {stdout:?}",
    );
}
