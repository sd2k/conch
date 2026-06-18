#!/usr/bin/env bash
# Patches gcx for wasip3/wasm32 compilation.
# Run from the grafana-cloud-cli directory after `go mod vendor`.
set -euo pipefail

V=vendor

echo "=== Adding wasip3 stub files ==="

# 1. prometheus/prometheus/tsdb/fileutil — mmap and flock need stubs
#    mmap_unix.go matches wasip3 via `!windows && !plan9 && !js`, but uses unix.Mmap
#    flock.go is unconstrained and calls newLock which needs a platform impl
cat > "$V/github.com/prometheus/prometheus/tsdb/fileutil/mmap_wasip3.go" << 'GO'
//go:build wasip3

package fileutil

import (
	"errors"
	"os"
)

func mmap(f *os.File, length int) ([]byte, error) {
	return nil, errors.New("unsupported")
}

func munmap(b []byte) (err error) {
	return errors.New("unsupported")
}
GO

cat > "$V/github.com/prometheus/prometheus/tsdb/fileutil/flock_wasip3.go" << 'GO'
//go:build wasip3

package fileutil

import "errors"

type unixLock struct{}

func (l *unixLock) Release() error {
	return errors.New("unsupported")
}

func (l *unixLock) set(lock bool) error {
	return errors.New("unsupported")
}

func newLock(fileName string) (Releaser, error) {
	return nil, errors.New("unsupported")
}
GO

echo "=== Patching overly-broad build constraints ==="

# mmap_unix.go: !windows && !plan9 && !js -> add !wasip3
sed -i 's|!windows && !plan9 && !js|!windows \&\& !plan9 \&\& !js \&\& !wasip3|' \
    "$V/github.com/prometheus/prometheus/tsdb/fileutil/mmap_unix.go"

echo "=== Patching source for WASI compatibility ==="

# signal.NotifyContext spawns a goroutine calling signal_recv() which blocks
# forever in WASI (no signals). Replace with plain context.WithCancel.
sed -i 's|cmdutil.Ctx, cmdutil.Cancel = signal.NotifyContext(cmd.Context(), os.Interrupt, syscall.SIGTERM)|cmdutil.Ctx, cmdutil.Cancel = context.WithCancel(cmd.Context()) // wasip3: no signals|' \
    cmd/root.go

# Remove unused signal/syscall imports (only if no other uses remain)
sed -i '/"os\/signal"/d; /"syscall"/d' cmd/root.go

echo "=== Done. Patched $(find $V -name '*wasip3*' | wc -l) vendor stub files + 1 source file ==="
