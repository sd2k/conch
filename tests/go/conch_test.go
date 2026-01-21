package conch

import (
	"os"
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

// TestResultFreeNullSafe verifies ResultFree handles nil safely
func TestResultFreeNullSafe(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}

	// Should not panic
	ResultFree(nil)
}

// TestResultFreeZeroResult verifies ResultFree handles zeroed struct
func TestResultFreeZeroResult(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}

	result := &ConchResult{}
	// Should not panic - all pointers are zero/nil
	ResultFree(result)
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

	// Verify field offsets
	var r ConchResult
	base := uintptr(unsafe.Pointer(&r))

	exitCodeOffset := uintptr(unsafe.Pointer(&r.ExitCode)) - base
	stdoutDataOffset := uintptr(unsafe.Pointer(&r.StdoutData)) - base
	stdoutLenOffset := uintptr(unsafe.Pointer(&r.StdoutLen)) - base
	stderrDataOffset := uintptr(unsafe.Pointer(&r.StderrData)) - base
	stderrLenOffset := uintptr(unsafe.Pointer(&r.StderrLen)) - base
	truncatedOffset := uintptr(unsafe.Pointer(&r.Truncated)) - base

	// Expected offsets on 64-bit
	if exitCodeOffset != 0 {
		t.Errorf("ExitCode offset = %d, expected 0", exitCodeOffset)
	}
	if stdoutDataOffset != 8 {
		t.Errorf("StdoutData offset = %d, expected 8", stdoutDataOffset)
	}
	if stdoutLenOffset != 16 {
		t.Errorf("StdoutLen offset = %d, expected 16", stdoutLenOffset)
	}
	if stderrDataOffset != 24 {
		t.Errorf("StderrData offset = %d, expected 24", stderrDataOffset)
	}
	if stderrLenOffset != 32 {
		t.Errorf("StderrLen offset = %d, expected 32", stderrLenOffset)
	}
	if truncatedOffset != 40 {
		t.Errorf("Truncated offset = %d, expected 40", truncatedOffset)
	}
}

// TestConchResultFields verifies we can set and read struct fields
func TestConchResultFields(t *testing.T) {
	result := ConchResult{
		ExitCode:   42,
		StdoutData: 0x1000, // fake pointer
		StdoutLen:  100,
		StderrData: 0x2000, // fake pointer
		StderrLen:  50,
		Truncated:  1,
	}

	if result.ExitCode != 42 {
		t.Errorf("ExitCode = %d, expected 42", result.ExitCode)
	}
	if result.StdoutData != 0x1000 {
		t.Errorf("StdoutData = %x, expected 0x1000", result.StdoutData)
	}
	if result.StdoutLen != 100 {
		t.Errorf("StdoutLen = %d, expected 100", result.StdoutLen)
	}
	if result.StderrData != 0x2000 {
		t.Errorf("StderrData = %x, expected 0x2000", result.StderrData)
	}
	if result.StderrLen != 50 {
		t.Errorf("StderrLen = %d, expected 50", result.StderrLen)
	}
	if result.Truncated != 1 {
		t.Errorf("Truncated = %d, expected 1", result.Truncated)
	}
}

// TestGoStringEmpty verifies goString handles empty/nil cases
func TestGoStringEmpty(t *testing.T) {
	s := goString(0)
	if s != "" {
		t.Errorf("goString(0) = %q, expected empty string", s)
	}
}

// TestGoBytesEmpty verifies goBytes handles empty/nil cases
func TestGoBytesEmpty(t *testing.T) {
	b := goBytes(0, 0)
	if b != nil {
		t.Errorf("goBytes(0, 0) = %v, expected nil", b)
	}

	b = goBytes(0, 10)
	if b != nil {
		t.Errorf("goBytes(0, 10) = %v, expected nil", b)
	}

	b = goBytes(0x1000, 0)
	if b != nil {
		t.Errorf("goBytes(0x1000, 0) = %v, expected nil", b)
	}
}

// Benchmark tests

func BenchmarkLastError(b *testing.B) {
	if !IsAvailable() {
		b.Skip("Skipping: conch library not available")
	}

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = LastError()
	}
}

func BenchmarkResultFreeNil(b *testing.B) {
	if !IsAvailable() {
		b.Skip("Skipping: conch library not available")
	}

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		ResultFree(nil)
	}
}

// ==================== Executor Tests ====================

// skipIfNoComponent skips the test if the WASM component is not available
func skipIfNoComponent(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}
	// Check for embedded component first, then file-based
	if HasEmbeddedComponent() {
		return
	}
	if _, err := findComponent(); err != nil {
		t.Skipf("Skipping: %v", err)
	}
}

func TestHasEmbeddedComponent(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}

	hasEmbedded := HasEmbeddedComponent()
	t.Logf("HasEmbeddedComponent() = %v", hasEmbedded)
}

func TestNewExecutorEmbedded(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}
	if !HasEmbeddedComponent() {
		t.Skip("Skipping: library not built with embedded-component feature")
	}

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	// Verify it works
	result, err := exec.Execute("echo embedded")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "embedded" {
		t.Errorf("Stdout = %q, want %q", stdout, "embedded")
	}
}

func TestNewExecutorDefault(t *testing.T) {
	skipIfNoComponent(t)

	exec, err := NewExecutorDefault()
	if err != nil {
		t.Fatalf("NewExecutorDefault() error = %v", err)
	}
	defer exec.Close()

	if exec.handle == 0 {
		t.Error("executor handle is zero")
	}
}

func TestNewExecutorFromBytes(t *testing.T) {
	skipIfNoComponent(t)

	path, err := findComponent()
	if err != nil {
		t.Fatalf("findComponent() error = %v", err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("ReadFile() error = %v", err)
	}

	exec, err := NewExecutorFromBytes(data)
	if err != nil {
		t.Fatalf("NewExecutorFromBytes() error = %v", err)
	}
	defer exec.Close()

	if exec.handle == 0 {
		t.Error("executor handle is zero")
	}
}

func TestExecuteEcho(t *testing.T) {
	skipIfNoComponent(t)

	exec, err := NewExecutorDefault()
	if err != nil {
		t.Fatalf("NewExecutorDefault() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("echo hello world")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0", result.ExitCode)
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "hello world" {
		t.Errorf("Stdout = %q, want %q", stdout, "hello world")
	}
}

func TestExecuteVariable(t *testing.T) {
	skipIfNoComponent(t)

	exec, err := NewExecutorDefault()
	if err != nil {
		t.Fatalf("NewExecutorDefault() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("NAME=conch; echo $NAME")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0", result.ExitCode)
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "conch" {
		t.Errorf("Stdout = %q, want %q", stdout, "conch")
	}
}

func TestExecutePipeline(t *testing.T) {
	skipIfNoComponent(t)

	exec, err := NewExecutorDefault()
	if err != nil {
		t.Fatalf("NewExecutorDefault() error = %v", err)
	}
	defer exec.Close()

	// Use printf which handles escape sequences consistently
	stdin := []byte("a\nb\nc\n")
	result, err := exec.ExecuteWithStdin("head -n 2", stdin)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0", result.ExitCode)
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	expected := "a\nb"
	if stdout != expected {
		t.Errorf("Stdout = %q, want %q", stdout, expected)
	}
}

func TestExecuteWithStdin(t *testing.T) {
	skipIfNoComponent(t)

	exec, err := NewExecutorDefault()
	if err != nil {
		t.Fatalf("NewExecutorDefault() error = %v", err)
	}
	defer exec.Close()

	stdin := []byte("line1\nline2\nline3\n")
	result, err := exec.ExecuteWithStdin("head -n 1", stdin)
	if err != nil {
		t.Fatalf("ExecuteWithStdin() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0", result.ExitCode)
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "line1" {
		t.Errorf("Stdout = %q, want %q", stdout, "line1")
	}
}

func TestExecuteExitCode(t *testing.T) {
	skipIfNoComponent(t)

	exec, err := NewExecutorDefault()
	if err != nil {
		t.Fatalf("NewExecutorDefault() error = %v", err)
	}
	defer exec.Close()

	// Use false which returns exit code 1, since exit is a shell builtin
	// that may not be available
	result, err := exec.Execute("false")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 1 {
		t.Errorf("ExitCode = %d, want 1", result.ExitCode)
	}
}

func TestExecuteJq(t *testing.T) {
	skipIfNoComponent(t)

	exec, err := NewExecutorDefault()
	if err != nil {
		t.Fatalf("NewExecutorDefault() error = %v", err)
	}
	defer exec.Close()

	stdin := []byte(`{"name": "conch", "version": "0.1.0"}`)
	result, err := exec.ExecuteWithStdin("jq .name", stdin)
	if err != nil {
		t.Fatalf("ExecuteWithStdin() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0", result.ExitCode)
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != `"conch"` {
		t.Errorf("Stdout = %q, want %q", stdout, `"conch"`)
	}
}

func TestExecutorClose(t *testing.T) {
	skipIfNoComponent(t)

	exec, err := NewExecutorDefault()
	if err != nil {
		t.Fatalf("NewExecutorDefault() error = %v", err)
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
	skipIfNoComponent(t)

	exec, err := NewExecutorDefault()
	if err != nil {
		t.Fatalf("NewExecutorDefault() error = %v", err)
	}

	exec.Close()
	exec.Close() // Should not panic
}

// Benchmarks

func BenchmarkExecuteEcho(b *testing.B) {
	if !IsAvailable() {
		b.Skip("Skipping: conch library not available")
	}
	if _, err := findComponent(); err != nil {
		b.Skipf("Skipping: %v", err)
	}

	exec, err := NewExecutorDefault()
	if err != nil {
		b.Fatalf("NewExecutorDefault() error = %v", err)
	}
	defer exec.Close()

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_, _ = exec.Execute("echo hello")
	}
}

// ==================== CoreExecutor Tests (brush-based) ====================

// skipIfNoShell skips the test if the embedded shell is not available
func skipIfNoShell(t *testing.T) {
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

func TestNewCoreExecutorEmbedded(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	if exec.handle == 0 {
		t.Error("executor handle is zero")
	}
}

func TestCoreExecuteEcho(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
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

func TestCoreExecuteVariable(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute("NAME=brush; echo $NAME")
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "brush" {
		t.Errorf("Stdout = %q, want %q", stdout, "brush")
	}
}

func TestCoreExecuteArithmetic(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
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

func TestCoreExecuteConditional(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
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

func TestCoreExecuteLoop(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
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

func TestCoreExecuteFalse(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
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

func TestCoreExecutorClose(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
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

func BenchmarkCoreExecuteEcho(b *testing.B) {
	if !IsAvailable() {
		b.Skip("Skipping: conch library not available")
	}
	if !HasEmbeddedShell() {
		b.Skip("Skipping: library not built with embedded-shell feature")
	}

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		b.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_, _ = exec.Execute("echo hello")
	}
}

// =============================================================================
// Custom Builtin Tests (cat, head, tail, wc, grep, jq)
// =============================================================================

func TestCoreBuiltinCat(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	// Test cat with echo pipe
	result, err := exec.Execute(`echo -e "line1\nline2\nline3" | cat`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	expected := "line1\nline2\nline3"
	if stdout != expected {
		t.Errorf("Stdout = %q, want %q", stdout, expected)
	}
}

func TestCoreBuiltinCatNumbered(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo -e "a\nb\nc" | cat -n`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := string(result.Stdout)
	// Should contain line numbers
	if !strings.Contains(stdout, "1") || !strings.Contains(stdout, "a") {
		t.Errorf("Stdout should contain numbered lines, got: %q", stdout)
	}
}

func TestCoreBuiltinHead(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	// Test head with default 10 lines (but input has fewer)
	result, err := exec.Execute(`echo -e "1\n2\n3\n4\n5" | head -n 3`)
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

func TestCoreBuiltinHeadBytes(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo "hello world" | head -c 5`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := string(result.Stdout)
	if stdout != "hello" {
		t.Errorf("Stdout = %q, want %q", stdout, "hello")
	}
}

func TestCoreBuiltinTail(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo -e "1\n2\n3\n4\n5" | tail -n 2`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	expected := "4\n5"
	if stdout != expected {
		t.Errorf("Stdout = %q, want %q", stdout, expected)
	}
}

func TestCoreBuiltinTailBytes(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo -n "hello world" | tail -c 5`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := string(result.Stdout)
	if stdout != "world" {
		t.Errorf("Stdout = %q, want %q", stdout, "world")
	}
}

func TestCoreBuiltinWcLines(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo -e "a\nb\nc" | wc -l`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "3" {
		t.Errorf("Stdout = %q, want %q", stdout, "3")
	}
}

func TestCoreBuiltinWcWords(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo "one two three four" | wc -w`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "4" {
		t.Errorf("Stdout = %q, want %q", stdout, "4")
	}
}

func TestCoreBuiltinWcChars(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo -n "hello" | wc -c`)
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

func TestCoreBuiltinGrepBasic(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo -e "apple\nbanana\napricot" | grep apple`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "apple" {
		t.Errorf("Stdout = %q, want %q", stdout, "apple")
	}
}

func TestCoreBuiltinGrepMultipleMatches(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo -e "apple\nbanana\napricot" | grep "^a"`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	expected := "apple\napricot"
	if stdout != expected {
		t.Errorf("Stdout = %q, want %q", stdout, expected)
	}
}

func TestCoreBuiltinGrepCaseInsensitive(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo -e "Apple\nBANANA\napple" | grep -i apple`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	expected := "Apple\napple"
	if stdout != expected {
		t.Errorf("Stdout = %q, want %q", stdout, expected)
	}
}

func TestCoreBuiltinGrepInvert(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo -e "apple\nbanana\napricot" | grep -v banana`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	expected := "apple\napricot"
	if stdout != expected {
		t.Errorf("Stdout = %q, want %q", stdout, expected)
	}
}

func TestCoreBuiltinGrepCount(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo -e "apple\nbanana\napricot" | grep -c "^a"`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "2" {
		t.Errorf("Stdout = %q, want %q", stdout, "2")
	}
}

func TestCoreBuiltinGrepNoMatch(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo "hello" | grep xyz`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	// grep returns 1 when no matches found
	if result.ExitCode != 1 {
		t.Errorf("ExitCode = %d, want 1 (no match)", result.ExitCode)
	}
}

func TestCoreBuiltinJqIdentity(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo '{"a":1}' | jq .`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	// jq pretty-prints by default
	if !strings.Contains(stdout, `"a"`) || !strings.Contains(stdout, "1") {
		t.Errorf("Stdout = %q, should contain the JSON object", stdout)
	}
}

func TestCoreBuiltinJqFieldAccess(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo '{"name":"test","value":42}' | jq .name`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != `"test"` {
		t.Errorf("Stdout = %q, want %q", stdout, `"test"`)
	}
}

func TestCoreBuiltinJqRawOutput(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo '{"name":"test"}' | jq -r .name`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "test" {
		t.Errorf("Stdout = %q, want %q (raw, no quotes)", stdout, "test")
	}
}

func TestCoreBuiltinJqArrayIteration(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo '[1,2,3]' | jq '.[]'`)
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

func TestCoreBuiltinJqCompact(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo '{"a": 1, "b": 2}' | jq -c .`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	// Compact output should be on one line
	if strings.Contains(stdout, "\n") {
		t.Errorf("Stdout should be compact (no newlines), got: %q", stdout)
	}
	if !strings.Contains(stdout, `"a"`) || !strings.Contains(stdout, `"b"`) {
		t.Errorf("Stdout = %q, should contain the JSON object", stdout)
	}
}

func TestCoreBuiltinJqFilter(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`echo '[{"a":1},{"a":2},{"a":3}]' | jq '[.[] | select(.a > 1)]'`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	// Should contain objects with a=2 and a=3
	if !strings.Contains(stdout, "2") || !strings.Contains(stdout, "3") {
		t.Errorf("Stdout = %q, should contain filtered results", stdout)
	}
	if strings.Contains(stdout, `"a": 1`) || strings.Contains(stdout, `"a":1`) {
		t.Errorf("Stdout = %q, should NOT contain a=1", stdout)
	}
}

func TestCoreBuiltinJqNullInput(t *testing.T) {
	skipIfNoShell(t)

	exec, err := NewCoreExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewCoreExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	result, err := exec.Execute(`jq -n '1 + 2'`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0. Stderr: %s", result.ExitCode, string(result.Stderr))
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "3" {
		t.Errorf("Stdout = %q, want %q", stdout, "3")
	}
}
