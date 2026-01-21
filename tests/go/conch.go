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

	// Function pointers - Component model executor
	conchLastError            func() uintptr
	conchResultFree           func(uintptr)
	conchHasEmbeddedComponent func() uint8
	conchExecutorNewEmbedded  func() uintptr
	conchExecutorNew          func(uintptr) uintptr
	conchExecutorNewFromBytes func(uintptr, uintptr) uintptr
	conchExecutorFree         func(uintptr)
	conchExecute              func(uintptr, uintptr) uintptr
	conchExecuteWithStdin     func(uintptr, uintptr, uintptr, uintptr) uintptr

	// Function pointers - Core executor (wasip1 / brush-based)
	conchHasEmbeddedShell         func() uint8
	conchCoreExecutorNewEmbedded  func() uintptr
	conchCoreExecutorNew          func(uintptr) uintptr
	conchCoreExecutorNewFromBytes func(uintptr, uintptr) uintptr
	conchCoreExecutorFree         func(uintptr)
	conchCoreExecute              func(uintptr, uintptr) uintptr
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
		purego.RegisterLibFunc(&conchHasEmbeddedComponent, lib, "conch_has_embedded_component")
		purego.RegisterLibFunc(&conchExecutorNew, lib, "conch_executor_new")
		purego.RegisterLibFunc(&conchExecutorNewFromBytes, lib, "conch_executor_new_from_bytes")
		purego.RegisterLibFunc(&conchExecutorFree, lib, "conch_executor_free")
		purego.RegisterLibFunc(&conchExecute, lib, "conch_execute")
		purego.RegisterLibFunc(&conchExecuteWithStdin, lib, "conch_execute_with_stdin")

		// Only register embedded executor if available (may not be exported if feature disabled)
		if conchHasEmbeddedComponent() == 1 {
			purego.RegisterLibFunc(&conchExecutorNewEmbedded, lib, "conch_executor_new_embedded")
		}

		// Register core executor functions
		purego.RegisterLibFunc(&conchHasEmbeddedShell, lib, "conch_has_embedded_shell")
		purego.RegisterLibFunc(&conchCoreExecutorNew, lib, "conch_core_executor_new")
		purego.RegisterLibFunc(&conchCoreExecutorNewFromBytes, lib, "conch_core_executor_new_from_bytes")
		purego.RegisterLibFunc(&conchCoreExecutorFree, lib, "conch_core_executor_free")
		purego.RegisterLibFunc(&conchCoreExecute, lib, "conch_core_execute")

		// Only register embedded shell if available
		if conchHasEmbeddedShell() == 1 {
			purego.RegisterLibFunc(&conchCoreExecutorNewEmbedded, lib, "conch_core_executor_new_embedded")
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

// ResultFree frees a ConchResult structure.
// Safe to call with a zero/nil pointer.
func ResultFree(result *ConchResult) {
	if err := Init(); err != nil {
		return
	}

	if result == nil {
		conchResultFree(0)
		return
	}

	conchResultFree(uintptr(unsafe.Pointer(result)))
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

// ErrNoEmbeddedComponent is returned when trying to use the embedded component
// but the library was not built with the embedded-component feature
var ErrNoEmbeddedComponent = errors.New("library was not built with embedded-component feature")

// HasEmbeddedComponent returns true if the library was built with the embedded component.
func HasEmbeddedComponent() bool {
	if err := Init(); err != nil {
		return false
	}
	return conchHasEmbeddedComponent() == 1
}

// HasEmbeddedShell returns true if the library was built with the embedded shell (brush-based).
func HasEmbeddedShell() bool {
	if err := Init(); err != nil {
		return false
	}
	return conchHasEmbeddedShell() == 1
}

// ErrNoEmbeddedShell is returned when trying to use the embedded shell
// but the library was not built with the embedded-shell feature
var ErrNoEmbeddedShell = errors.New("library was not built with embedded-shell feature")

// Executor wraps a ConchExecutor handle
type Executor struct {
	handle uintptr
}

// findComponent searches for the WASM component in common locations
func findComponent() (string, error) {
	_, thisFile, _, _ := runtime.Caller(0)
	testDir := filepath.Dir(thisFile)
	repoRoot := filepath.Join(testDir, "..", "..")

	// Search paths in order of preference
	searchPaths := []string{
		filepath.Join(repoRoot, "target", "wasm32-unknown-unknown", "release", "conch_wasm.component.wasm"),
		filepath.Join(repoRoot, "target", "wasm32-unknown-unknown", "debug", "conch_wasm.component.wasm"),
	}

	for _, path := range searchPaths {
		if _, err := os.Stat(path); err == nil {
			return path, nil
		}
	}

	return "", fmt.Errorf("component not found in search paths: %v", searchPaths)
}

// NewExecutor creates a new shell executor from a component file path.
func NewExecutor(componentPath string) (*Executor, error) {
	if err := Init(); err != nil {
		return nil, err
	}

	cPath, err := cString(componentPath)
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

// NewExecutorFromBytes creates a new shell executor from component bytes.
func NewExecutorFromBytes(data []byte) (*Executor, error) {
	if err := Init(); err != nil {
		return nil, err
	}

	if len(data) == 0 {
		return nil, errors.New("component data is empty")
	}

	handle := conchExecutorNewFromBytes(uintptr(unsafe.Pointer(&data[0])), uintptr(len(data)))
	if handle == 0 {
		return nil, fmt.Errorf("failed to create executor: %s", LastError())
	}

	return &Executor{handle: handle}, nil
}

// NewExecutorDefault creates a new shell executor using the default component location.
// If the library was built with embedded-component feature, uses that.
// Otherwise, searches for the component file.
func NewExecutorDefault() (*Executor, error) {
	// Prefer embedded component if available
	if HasEmbeddedComponent() {
		return NewExecutorEmbedded()
	}

	// Fall back to file-based loading
	path, err := findComponent()
	if err != nil {
		return nil, err
	}
	return NewExecutor(path)
}

// NewExecutorEmbedded creates a new shell executor using the embedded WASM component.
// Returns an error if the library was not built with the embedded-component feature.
func NewExecutorEmbedded() (*Executor, error) {
	if err := Init(); err != nil {
		return nil, err
	}

	if !HasEmbeddedComponent() {
		return nil, ErrNoEmbeddedComponent
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

// Execute runs a shell script and returns the result.
func (e *Executor) Execute(script string) (*Result, error) {
	return e.ExecuteWithStdin(script, nil)
}

// ExecuteWithStdin runs a shell script with stdin input.
func (e *Executor) ExecuteWithStdin(script string, stdin []byte) (*Result, error) {
	if e.handle == 0 {
		return nil, errors.New("executor is closed")
	}

	cScript, err := cString(script)
	if err != nil {
		return nil, err
	}
	defer freeString(cScript)

	var resultPtr uintptr
	if len(stdin) == 0 {
		resultPtr = conchExecute(e.handle, cScript)
	} else {
		resultPtr = conchExecuteWithStdin(
			e.handle,
			cScript,
			uintptr(unsafe.Pointer(&stdin[0])),
			uintptr(len(stdin)),
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

// ============================================================================
// CoreExecutor - brush-based wasip1 shell executor
// ============================================================================

// CoreExecutor wraps a ConchCoreExecutor handle (brush-based shell)
type CoreExecutor struct {
	handle uintptr
}

// NewCoreExecutor creates a new core shell executor from a module file path.
func NewCoreExecutor(modulePath string) (*CoreExecutor, error) {
	if err := Init(); err != nil {
		return nil, err
	}

	cPath, err := cString(modulePath)
	if err != nil {
		return nil, err
	}
	defer freeString(cPath)

	handle := conchCoreExecutorNew(cPath)
	if handle == 0 {
		return nil, fmt.Errorf("failed to create core executor: %s", LastError())
	}

	return &CoreExecutor{handle: handle}, nil
}

// NewCoreExecutorFromBytes creates a new core shell executor from module bytes.
func NewCoreExecutorFromBytes(data []byte) (*CoreExecutor, error) {
	if err := Init(); err != nil {
		return nil, err
	}

	if len(data) == 0 {
		return nil, errors.New("module data is empty")
	}

	handle := conchCoreExecutorNewFromBytes(uintptr(unsafe.Pointer(&data[0])), uintptr(len(data)))
	if handle == 0 {
		return nil, fmt.Errorf("failed to create core executor: %s", LastError())
	}

	return &CoreExecutor{handle: handle}, nil
}

// NewCoreExecutorEmbedded creates a new core shell executor using the embedded WASM module.
// Returns an error if the library was not built with the embedded-shell feature.
func NewCoreExecutorEmbedded() (*CoreExecutor, error) {
	if err := Init(); err != nil {
		return nil, err
	}

	if !HasEmbeddedShell() {
		return nil, ErrNoEmbeddedShell
	}

	handle := conchCoreExecutorNewEmbedded()
	if handle == 0 {
		return nil, fmt.Errorf("failed to create core executor: %s", LastError())
	}

	return &CoreExecutor{handle: handle}, nil
}

// Close frees the core executor resources.
func (e *CoreExecutor) Close() {
	if e.handle != 0 {
		conchCoreExecutorFree(e.handle)
		e.handle = 0
	}
}

// Execute runs a shell script and returns the result.
func (e *CoreExecutor) Execute(script string) (*Result, error) {
	if e.handle == 0 {
		return nil, errors.New("executor is closed")
	}

	cScript, err := cString(script)
	if err != nil {
		return nil, err
	}
	defer freeString(cScript)

	resultPtr := conchCoreExecute(e.handle, cScript)
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
