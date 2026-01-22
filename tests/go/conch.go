// Package conch provides Go bindings for the Conch shell library using purego.
//
// This package uses purego to call into the Conch shared library without CGO,
// making cross-compilation easier and removing the need for a C toolchain.
package conch

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"unsafe"

	"github.com/ebitengine/purego"
)

// ConchResult matches the C struct layout from ffi.rs
// #[repr(C)]
//
//	pub struct ConchResult {
//	    pub exit_code: i32,
//	    pub stdout_data: *mut c_char,
//	    pub stdout_len: usize,
//	    pub stderr_data: *mut c_char,
//	    pub stderr_len: usize,
//	    pub truncated: u8,
//	}
type ConchResult struct {
	ExitCode   int32
	_pad0      [4]byte // padding to align pointer
	StdoutData uintptr // *c_char
	StdoutLen  uintptr // size_t
	StderrData uintptr // *c_char
	StderrLen  uintptr // size_t
	Truncated  uint8
	_pad1      [7]byte // padding to align struct
}

// Result is the Go-friendly version of ConchResult
type Result struct {
	ExitCode  int
	Stdout    []byte
	Stderr    []byte
	Truncated bool
}

var (
	libOnce sync.Once
	lib     uintptr
	libErr  error

	// Function pointers
	conchLastError            func() uintptr
	conchResultFree           func(uintptr)
	conchHasEmbeddedShell     func() uint8
	conchExecutorNewEmbedded  func() uintptr
	conchExecutorNew          func(uintptr) uintptr
	conchExecutorNewFromBytes func(uintptr, uintptr) uintptr
	conchExecutorFree         func(uintptr)
	conchExecute              func(uintptr, uintptr) uintptr
	conchExecuteWithLimits    func(uintptr, uintptr, uint64, uint64, uint64, uint64) uintptr
)

// libName returns the platform-specific library name
func libName() string {
	switch runtime.GOOS {
	case "darwin":
		return "libconch.dylib"
	case "windows":
		return "conch.dll"
	default:
		return "libconch.so"
	}
}

// findLibrary searches for the conch library in common locations
func findLibrary() (string, error) {
	name := libName()

	// Get the directory of this source file for relative paths
	_, thisFile, _, _ := runtime.Caller(0)
	testDir := filepath.Dir(thisFile)
	repoRoot := filepath.Join(testDir, "..", "..")

	// Search paths in order of preference
	searchPaths := []string{
		// Release build (preferred for FFI testing)
		filepath.Join(repoRoot, "target", "release", name),
		// Debug build
		filepath.Join(repoRoot, "target", "debug", name),
		// System paths
		filepath.Join("/usr/local/lib", name),
		filepath.Join("/usr/lib", name),
	}

	// Also check LD_LIBRARY_PATH on Linux
	if runtime.GOOS == "linux" {
		if ldPath := os.Getenv("LD_LIBRARY_PATH"); ldPath != "" {
			for _, dir := range filepath.SplitList(ldPath) {
				searchPaths = append([]string{filepath.Join(dir, name)}, searchPaths...)
			}
		}
	}

	for _, path := range searchPaths {
		if _, err := os.Stat(path); err == nil {
			return path, nil
		}
	}

	return "", fmt.Errorf("library %s not found in search paths: %v", name, searchPaths)
}

// Init initializes the conch library. It is safe to call multiple times.
func Init() error {
	libOnce.Do(func() {
		libPath, err := findLibrary()
		if err != nil {
			libErr = err
			return
		}

		lib, err = purego.Dlopen(libPath, purego.RTLD_NOW|purego.RTLD_GLOBAL)
		if err != nil {
			libErr = fmt.Errorf("failed to load library %s: %w", libPath, err)
			return
		}

		// Register functions
		purego.RegisterLibFunc(&conchLastError, lib, "conch_last_error")
		purego.RegisterLibFunc(&conchResultFree, lib, "conch_result_free")
		purego.RegisterLibFunc(&conchHasEmbeddedShell, lib, "conch_has_embedded_shell")
		purego.RegisterLibFunc(&conchExecutorNew, lib, "conch_executor_new")
		purego.RegisterLibFunc(&conchExecutorNewFromBytes, lib, "conch_executor_new_from_bytes")
		purego.RegisterLibFunc(&conchExecutorFree, lib, "conch_executor_free")
		purego.RegisterLibFunc(&conchExecute, lib, "conch_execute")
		purego.RegisterLibFunc(&conchExecuteWithLimits, lib, "conch_execute_with_limits")

		// Only register embedded executor if available
		if conchHasEmbeddedShell() == 1 {
			purego.RegisterLibFunc(&conchExecutorNewEmbedded, lib, "conch_executor_new_embedded")
		}
	})

	return libErr
}

// LastError returns the last error message from the conch library.
// Returns an empty string if no error is set.
func LastError() string {
	if err := Init(); err != nil {
		return ""
	}

	ptr := conchLastError()
	if ptr == 0 {
		return ""
	}

	return goString(ptr)
}

// goString converts a C string pointer to a Go string
func goString(ptr uintptr) string {
	if ptr == 0 {
		return ""
	}

	// Find the null terminator
	var length int
	for {
		b := *(*byte)(unsafe.Pointer(ptr + uintptr(length)))
		if b == 0 {
			break
		}
		length++
		// Safety limit
		if length > 1<<20 {
			break
		}
	}

	if length == 0 {
		return ""
	}

	// Copy the bytes to a Go string
	bytes := make([]byte, length)
	for i := 0; i < length; i++ {
		bytes[i] = *(*byte)(unsafe.Pointer(ptr + uintptr(i)))
	}

	return string(bytes)
}

// goBytes converts a C byte array to a Go byte slice
func goBytes(ptr uintptr, length int) []byte {
	if ptr == 0 || length == 0 {
		return nil
	}

	bytes := make([]byte, length)
	for i := 0; i < length; i++ {
		bytes[i] = *(*byte)(unsafe.Pointer(ptr + uintptr(i)))
	}

	return bytes
}

// IsAvailable checks if the conch library is available
func IsAvailable() bool {
	return Init() == nil
}

// LibraryPath returns the path to the loaded library, or an error if not loaded
func LibraryPath() (string, error) {
	if err := Init(); err != nil {
		return "", err
	}
	return findLibrary()
}

// ErrLibraryNotFound is returned when the conch library cannot be found
var ErrLibraryNotFound = errors.New("conch library not found")

// ErrNoEmbeddedShell is returned when trying to use the embedded shell
// but the library was not built with the embedded-shell feature
var ErrNoEmbeddedShell = errors.New("library was not built with embedded-shell feature")

// HasEmbeddedShell returns true if the library was built with the embedded shell module.
func HasEmbeddedShell() bool {
	if err := Init(); err != nil {
		return false
	}
	return conchHasEmbeddedShell() == 1
}

// ResourceLimits configures execution limits for shell scripts
type ResourceLimits struct {
	// MaxCPUMs is the maximum CPU time in milliseconds
	MaxCPUMs uint64
	// MaxMemoryBytes is the maximum memory in bytes
	MaxMemoryBytes uint64
	// MaxOutputBytes is the maximum output (stdout + stderr) in bytes
	MaxOutputBytes uint64
	// TimeoutMs is the wall-clock timeout in milliseconds
	TimeoutMs uint64
}

// DefaultLimits returns sensible default resource limits
func DefaultLimits() ResourceLimits {
	return ResourceLimits{
		MaxCPUMs:       5000,             // 5 seconds CPU
		MaxMemoryBytes: 64 * 1024 * 1024, // 64 MB
		MaxOutputBytes: 1024 * 1024,      // 1 MB output
		TimeoutMs:      30000,            // 30 second timeout
	}
}

// Executor wraps a ConchExecutor handle
type Executor struct {
	handle uintptr
}

// NewExecutor creates a new shell executor from a WASM module file path.
func NewExecutor(modulePath string) (*Executor, error) {
	if err := Init(); err != nil {
		return nil, err
	}

	cPath, err := cString(modulePath)
	if err != nil {
		return nil, err
	}
	defer freeString(cPath)

	handle := conchExecutorNew(cPath)
	if handle == 0 {
		return nil, fmt.Errorf("failed to create executor: %s", LastError())
	}

	return &Executor{handle: handle}, nil
}

// NewExecutorFromBytes creates a new shell executor from WASM module bytes.
func NewExecutorFromBytes(data []byte) (*Executor, error) {
	if err := Init(); err != nil {
		return nil, err
	}

	if len(data) == 0 {
		return nil, errors.New("module data is empty")
	}

	handle := conchExecutorNewFromBytes(uintptr(unsafe.Pointer(&data[0])), uintptr(len(data)))
	if handle == 0 {
		return nil, fmt.Errorf("failed to create executor: %s", LastError())
	}

	return &Executor{handle: handle}, nil
}

// NewExecutorEmbedded creates a new shell executor using the embedded WASM module.
// Returns an error if the library was not built with the embedded-shell feature.
func NewExecutorEmbedded() (*Executor, error) {
	if err := Init(); err != nil {
		return nil, err
	}

	if !HasEmbeddedShell() {
		return nil, ErrNoEmbeddedShell
	}

	handle := conchExecutorNewEmbedded()
	if handle == 0 {
		return nil, fmt.Errorf("failed to create executor: %s", LastError())
	}

	return &Executor{handle: handle}, nil
}

// Close frees the executor resources.
func (e *Executor) Close() {
	if e.handle != 0 {
		conchExecutorFree(e.handle)
		e.handle = 0
	}
}

// Execute runs a shell script with default resource limits and returns the result.
func (e *Executor) Execute(script string) (*Result, error) {
	return e.ExecuteWithLimits(script, DefaultLimits())
}

// ExecuteWithLimits runs a shell script with custom resource limits.
func (e *Executor) ExecuteWithLimits(script string, limits ResourceLimits) (*Result, error) {
	if e.handle == 0 {
		return nil, errors.New("executor is closed")
	}

	cScript, err := cString(script)
	if err != nil {
		return nil, err
	}
	defer freeString(cScript)

	var resultPtr uintptr
	if limits == DefaultLimits() {
		// Use the simpler execute function for default limits
		resultPtr = conchExecute(e.handle, cScript)
	} else {
		resultPtr = conchExecuteWithLimits(
			e.handle,
			cScript,
			limits.MaxCPUMs,
			limits.MaxMemoryBytes,
			limits.MaxOutputBytes,
			limits.TimeoutMs,
		)
	}

	if resultPtr == 0 {
		return nil, fmt.Errorf("execution failed: %s", LastError())
	}

	// Convert to Go result
	cResult := (*ConchResult)(unsafe.Pointer(resultPtr))
	result := &Result{
		ExitCode:  int(cResult.ExitCode),
		Stdout:    goBytes(cResult.StdoutData, int(cResult.StdoutLen)),
		Stderr:    goBytes(cResult.StderrData, int(cResult.StderrLen)),
		Truncated: cResult.Truncated != 0,
	}

	// Free the C result
	conchResultFree(resultPtr)

	return result, nil
}

// cString converts a Go string to a null-terminated C string
func cString(s string) (uintptr, error) {
	b := make([]byte, len(s)+1)
	copy(b, s)
	b[len(s)] = 0
	return uintptr(unsafe.Pointer(&b[0])), nil
}

// freeString is a no-op since we use Go-allocated memory
func freeString(ptr uintptr) {
	// Go GC handles this
}
