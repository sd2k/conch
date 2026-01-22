//! Integration tests for the WASM shell executor.

#[cfg(all(test, feature = "embedded-shell"))]
#[allow(clippy::expect_used)]
mod embedded_tests {
    use crate::limits::ResourceLimits;
    use crate::wasm_core::CoreShellExecutor;

    fn executor() -> CoreShellExecutor {
        CoreShellExecutor::embedded().expect("Failed to create embedded executor")
    }

    #[tokio::test]
    async fn test_echo() {
        let exec = executor();
        let limits = ResourceLimits::default();
        let result = exec
            .execute("echo hello", &limits)
            .await
            .expect("execute failed");

        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.trim() == "hello", "stdout: {:?}", stdout);
    }

    #[tokio::test]
    async fn test_simple_pipe() {
        let exec = executor();
        let limits = ResourceLimits::default();
        let result = exec
            .execute("echo hello | cat", &limits)
            .await
            .expect("execute failed");

        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.trim() == "hello", "stdout: {:?}", stdout);
    }

    #[tokio::test]
    async fn test_cat_builtin() {
        let exec = executor();
        let limits = ResourceLimits::default();

        let result = exec
            .execute("echo 'line1' | cat", &limits)
            .await
            .expect("execute failed");
        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert_eq!(stdout.trim(), "line1", "stdout: {:?}", stdout);
    }

    #[tokio::test]
    async fn test_wc_builtin() {
        let exec = executor();
        let limits = ResourceLimits::default();

        let result = exec
            .execute("echo hello | wc -c", &limits)
            .await
            .expect("execute failed");
        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[tokio::test]
    async fn test_grep_builtin() {
        let exec = executor();
        let limits = ResourceLimits::default();

        let result = exec
            .execute("echo bar | grep bar", &limits)
            .await
            .expect("execute failed");
        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[tokio::test]
    async fn test_jq_builtin() {
        let exec = executor();
        let limits = ResourceLimits::default();

        let result = exec
            .execute(r#"echo '{"name":"test"}' | jq '.name'"#, &limits)
            .await
            .expect("execute failed");
        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[tokio::test]
    async fn test_head_lines() {
        let exec = executor();
        let limits = ResourceLimits::default();

        let result = exec
            .execute(r#"echo "hello world" | head -n 1"#, &limits)
            .await
            .expect("execute failed");

        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert_eq!(stdout.trim(), "hello world", "stdout: {:?}", stdout);
    }

    #[tokio::test]
    async fn test_head_bytes() {
        let exec = executor();
        let limits = ResourceLimits::default();

        let result = exec
            .execute(r#"echo "hello world" | head -c 5"#, &limits)
            .await
            .expect("execute failed");

        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert_eq!(stdout, "hello", "stdout: {:?}", stdout);
    }

    #[tokio::test]
    async fn test_tail_bytes() {
        let exec = executor();
        let limits = ResourceLimits::default();

        let result = exec
            .execute(r#"echo -n "hello world" | tail -c 5"#, &limits)
            .await
            .expect("execute failed");

        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert_eq!(stdout, "world", "stdout: {:?}", stdout);
    }
}
