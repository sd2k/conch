package conch

import (
	"strings"
	"testing"
)

func TestSimplePipe(t *testing.T) {
	if !IsAvailable() {
		t.Skip("Skipping: conch library not available")
	}
	if !HasEmbeddedShell() {
		t.Skip("Skipping: library not built with embedded-shell feature")
	}

	exec, err := NewExecutorEmbedded()
	if err != nil {
		t.Fatalf("NewExecutorEmbedded() error = %v", err)
	}
	defer exec.Close()

	// Simplest pipe test
	result, err := exec.Execute(`echo hello | cat`)
	if err != nil {
		t.Fatalf("Execute() error = %v", err)
	}

	t.Logf("ExitCode: %d", result.ExitCode)
	t.Logf("Stdout: %q", string(result.Stdout))
	t.Logf("Stderr: %q", string(result.Stderr))

	if result.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0", result.ExitCode)
	}

	stdout := strings.TrimSpace(string(result.Stdout))
	if stdout != "hello" {
		t.Errorf("Stdout = %q, want %q", stdout, "hello")
	}
}
