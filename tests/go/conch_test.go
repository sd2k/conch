package conch

import (
	"strings"
	"testing"
	"unsafe"
)

// TestInit verifies the library can be loaded
func TestInit(t *testing.T) {
	err := Init()
	if err != nil {
		t.Skipf("Skipping FFI tests: %v", err)
	}
}

// TestIsAvailable checks the availability helper
func TestIsAvailable(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}
}

// TestLibraryPath verifies we can get the library path
func TestLibraryPath(t *testing.T) {
	path, err := LibraryPath()
	if err != nil {
		t.Skipf("Skipping: %v", err)
	}
	t.Logf("Library loaded from: %s", path)
}

// TestLastErrorInitiallyEmpty verifies no error is set initially
func TestLastErrorInitiallyEmpty(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}

	// Note: In a fresh thread/goroutine, the error should be empty
	// However, since Go may reuse OS threads, we can't guarantee this
	// Just verify the function doesn't crash
	_ = LastError()
}

// TestLastErrorReturnsString verifies LastError returns a string type
func TestLastErrorReturnsString(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}

	err := LastError()
	// We can't set errors from Go side yet, so just verify the type
	if err != "" {
		t.Logf("LastError returned: %q", err)
	}
}

// TestConchResultLayout verifies the struct layout matches Rust
func TestConchResultLayout(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}

	// Verify struct size is reasonable for the expected layout
	size := unsafe.Sizeof(ConchResult{})

	// On 64-bit systems:
	// - int32 (4) + pad (4) = 8
	// - uintptr (8) = 8
	// - uintptr (8) = 8
	// - uintptr (8) = 8
	// - uintptr (8) = 8
	// - uint8 (1) + pad (7) = 8
	// Total = 48 bytes
	expectedSize := uintptr(48)

	if size != expectedSize {
		t.Errorf("ConchResult size = %d, expected %d", size, expectedSize)
	}
}

// TestConchResultFields verifies struct field access
func TestConchResultFields(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}

	result := ConchResult{
		ExitCode:   42,
		StdoutData: 0,
		StdoutLen:  100,
		StderrData: 0,
		StderrLen:  50,
		Truncated:  1,
	}

	if result.ExitCode != 42 {
		t.Errorf("ExitCode = %d, want 42", result.ExitCode)
	}
	if result.StdoutLen != 100 {
		t.Errorf("StdoutLen = %d, want 100", result.StdoutLen)
	}
	if result.StderrLen != 50 {
		t.Errorf("StderrLen = %d, want 50", result.StderrLen)
	}
	if result.Truncated != 1 {
		t.Errorf("Truncated = %d, want 1", result.Truncated)
	}
}

func BenchmarkLastError(b *testing.B) {
	if !IsAvailable() {
		b.Skip("Skipping: conch library not available")
	}

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = LastError()
	}
}

// ==================== Executor Tests ====================

// skipIfNoEmbeddedShell skips the test if the embedded shell is not available
func skipIfNoEmbeddedShell(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}
	if !HasEmbeddedShell() {
		t.Skip("Skipping: library not built with embedded-shell feature")
	}
}

func TestHasEmbeddedShell(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}

	hasEmbedded := HasEmbeddedShell()
	t.Logf("HasEmbeddedShell() = %v", hasEmbedded)
}

func TestNewExecutorEmbedded(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	if exec.handle == 0 {
		t.Error("executor handle is zero")
	}
}

func TestExecuteEcho(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("echo hello world")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "hello world" {
		t.Errorf("Stdout = %q, want %q", stdout, "hello world")
	}
}

func TestExecuteVariable(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("NAME=conch; echo $NAME")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "conch" {
		t.Errorf("Stdout = %q, want %q", stdout, "conch")
	}
}

func TestExecuteArithmetic(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("echo $((2 + 3))")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "5" {
		t.Errorf("Stdout = %q, want %q", stdout, "5")
	}
}

func TestExecuteConditional(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("if true; then echo yes; else echo no; fi")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "yes" {
		t.Errorf("Stdout = %q, want %q", stdout, "yes")
	}
}

func TestExecuteLoop(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("for i in 1 2 3; do echo $i; done")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	expected := "1\n2\n3"
	if stdout != expected {
		t.Errorf("Stdout = %q, want %q", stdout, expected)
	}
}

func TestExecuteFalse(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("false")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 1 {
		t.Errorf("ExitCode = %d, want 1", result.ExitCode)
	}
}

func TestExecutorClose(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}

	exec.Close()

	// Verify handle is zeroed
	if exec.handle != 0 {
		t.Error("handle should be zero after Close()")
	}

	// Execute should fail on closed executor
	_, err = exec.Execute("echo test")
	if err == nil {
		t.Error("Execute() on closed executor should return error")
	}
}

func TestExecutorDoubleClose(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}

	// Double close should be safe
	exec.Close()
	exec.Close()
}

func TestExecuteWithLimits(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	limits := ResourceLimits{
		MaxCPUMs:       10000,
		MaxMemoryBytes: 128 * 1024 * 1024,
		MaxOutputBytes: 1024 * 1024,
		TimeoutMs:      60000,
	}

	result, err := exec.ExecuteWithLimits("echo custom limits", limits)
	if err != nil {
		t.Fatalf("ExecuteWithLimits() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "custom limits" {
		t.Errorf("Stdout = %q, want %q", stdout, "custom limits")
	}
}

func TestDefaultLimits(t *testing.T) {
	limits := DefaultLimits()

	if limits.MaxCPUMs != 5000 {
		t.Errorf("MaxCPUMs = %d, want 5000", limits.MaxCPUMs)
	}
	if limits.MaxMemoryBytes != 64*1024*1024 {
		t.Errorf("MaxMemoryBytes = %d, want %d", limits.MaxMemoryBytes, 64*1024*1024)
	}
	if limits.MaxOutputBytes != 1024*1024 {
		t.Errorf("MaxOutputBytes = %d, want %d", limits.MaxOutputBytes, 1024*1024)
	}
	if limits.TimeoutMs != 30000 {
		t.Errorf("TimeoutMs = %d, want 30000", limits.TimeoutMs)
	}
}

func BenchmarkExecuteEcho(b *testing.B) {
	if !IsAvailable() {
		b.Skip("Skipping: conch library not available")
	}
	if !HasEmbeddedShell() {
		b.Skip("Skipping: library not built with embedded-shell feature")
	}

	exec, err := NewExecutorEmbedded()
	if err != nil {
		b.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_, _ = exec.Execute("echo hello")
	}
}

// ==================== Builtin Tests ====================

func TestBuiltinCat(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("echo 'line1\nline2\nline3' | cat")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if !strings.Contains(stdout, "line1") {
		t.Errorf("Stdout = %q, should contain 'line1'", stdout)
	}
}

func TestBuiltinHead(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("echo -e 'a\nb\nc\nd\ne' | head -n 2")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	lines := strings.Split(stdout, "\n")
	if len(lines) > 2 {
		t.Errorf("Expected at most 2 lines, got %d: %q", len(lines), stdout)
	}
}

func TestBuiltinTail(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("echo -e 'a\nb\nc\nd\ne' | tail -n 2")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	lines := strings.Split(stdout, "\n")
	if len(lines) > 2 {
		t.Errorf("Expected at most 2 lines, got %d: %q", len(lines), stdout)
	}
}

func TestBuiltinWcLines(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("echo -e 'a\nb\nc' | wc -l")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	// wc -l should return 3 or 4 depending on trailing newline handling
	if !strings.Contains(stdout, "3") && !strings.Contains(stdout, "4") {
		t.Errorf("wc -l output = %q, expected to contain '3' or '4'", stdout)
	}
}

func TestBuiltinGrepBasic(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("echo -e 'foo\nbar\nbaz' | grep bar")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "bar" {
		t.Errorf("Stdout = %q, want %q", stdout, "bar")
	}
}

func TestBuiltinGrepNoMatch(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("echo 'hello' | grep xyz")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	// grep returns 1 when no match found
	if result.ExitCode != 1 {
		t.Errorf("ExitCode = %d, want 1 (no match)", result.ExitCode)
	}
}

func TestBuiltinGrepCaseInsensitive(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("echo 'Hello World' | grep -i hello")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if !strings.Contains(stdout, "Hello") {
		t.Errorf("Stdout = %q, should contain 'Hello'", stdout)
	}
}

func TestBuiltinJqIdentity(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo '{"name":"test"}' | jq .`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := string(result.Stdout)
	if !strings.Contains(stdout, "name") || !strings.Contains(stdout, "test") {
		t.Errorf("Stdout = %q, should contain JSON with name:test", stdout)
	}
}

func TestBuiltinJqFieldAccess(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo '{"name":"conch","version":1}' | jq .name`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	// jq outputs strings with quotes by default
	if !strings.Contains(stdout, "conch") {
		t.Errorf("Stdout = %q, should contain 'conch'", stdout)
	}
}

func TestBuiltinJqRawOutput(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo '{"name":"conch"}' | jq -r .name`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	// With -r, output should be unquoted
	if stdout != "conch" {
		t.Errorf("Stdout = %q, want %q", stdout, "conch")
	}
}

// ==================== Multiple Execution Tests ====================

func TestMultipleExecutions(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	// Multiple executions should work independently
	for i := 0; i < 5; i++ {
		result, err := exec.Execute("echo test")
		if err != nil {
			t.Fatalf("Execute() iteration %d error = %v", i, err)
		}
		if result.ExitCode != 0 {
			t.Errorf("Iteration %d: ExitCode = %d, want 0", i, result.ExitCode)
		}
	}
}

func TestVariablesNotShared(t *testing.T) {
	skipIfNoEmbeddedShell(t)

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	// Set a variable in first execution
	result1, err := exec.Execute("MY_VAR=secret; echo $MY_VAR")
	if err != nil {
		t.Fatalf("Execute() 1 error = %v", err)
	}
	stdout1 := strings.TrimSpace(string(result1.Stdout))
	if stdout1 != "secret" {
		t.Errorf("First execution: Stdout = %q, want %q", stdout1, "secret")
	}

	// Second execution should not see the variable (fresh state)
	result2, err := exec.Execute("echo ${MY_VAR:-unset}")
	if err != nil {
		t.Fatalf("Execute() 2 error = %v", err)
	}
	stdout2 := strings.TrimSpace(string(result2.Stdout))
	if stdout2 != "unset" {
		t.Errorf("Second execution: Stdout = %q, want %q (variable should not persist)", stdout2, "unset")
	}
}
