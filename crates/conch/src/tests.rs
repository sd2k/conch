//! Integration tests for the WASM shell executor.

#[cfg(all(test, feature = "embedded-shell"))]
#[allow(clippy::expect_used)]
mod embedded_tests {
    use crate::limits::ResourceLimits;
    use crate::runtime::Conch;

    fn conch() -> Conch {
        Conch::embedded(1).expect("Failed to create embedded Conch")
    }

    #[tokio::test]
    async fn test_echo() {
        let conch = conch();
        let limits = ResourceLimits::default();
        let result = conch
            .execute("echo hello", limits)
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
        let conch = conch();
        let limits = ResourceLimits::default();
        let result = conch
            .execute("echo hello | cat", limits)
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
        let conch = conch();
        let limits = ResourceLimits::default();

        let result = conch
            .execute("echo 'line1' | cat", limits)
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
        let conch = conch();
        let limits = ResourceLimits::default();

        let result = conch
            .execute("echo hello | wc -c", limits)
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
        let conch = conch();
        let limits = ResourceLimits::default();

        let result = conch
            .execute("echo bar | grep bar", limits)
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
        let conch = conch();
        let limits = ResourceLimits::default();

        let result = conch
            .execute(r#"echo '{"name":"test"}' | jq '.name'"#, limits)
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
        let conch = conch();
        let limits = ResourceLimits::default();

        let result = conch
            .execute(r#"echo "hello world" | head -n 1"#, limits)
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
        let conch = conch();
        let limits = ResourceLimits::default();

        let result = conch
            .execute(r#"echo "hello world" | head -c 5"#, limits)
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
        let conch = conch();
        let limits = ResourceLimits::default();

        let result = conch
            .execute(r#"echo -n "hello world" | tail -c 5"#, limits)
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
