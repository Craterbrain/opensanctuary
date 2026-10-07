#!/bin/bash
# Fully automated Windows test run on the Windows 10 sandbox VM.
#
# Why this exists: src/network/windows.rs shells out to real powershell.exe
# for Hyper-V detection etc. — untestable under Wine and meaningless on Linux.
# This cross-compiles the test binary via cargo-xwin (x86_64-pc-windows-msvc
# -- see docs/installer.md and .cargo/config.toml: two independent mingw-w64
# toolchains, zig-as-mingw included, both produced a real access-violation
# crash around WebView2Loader.dll's dllimport thunk, which is why this no
# longer targets x86_64-pc-windows-gnu at all), pushes it to a real Windows
# VM over WinRM, and runs it there.
#
# Run this on demand when you're testing Windows-specific code paths
# (src/network/windows.rs, webview/, etc.) — not on every change. It's slow
# (cross-compile + multi-MB WinRM push), unlike vms/linux-test.sh.
#
# Usage: vms/windows-test.sh [test filter]
#   vms/windows-test.sh                              # all lib tests
#   vms/windows-test.sh check_hyperv_support          # one test

set -euo pipefail

DOM="os-next-win10-test"
WINRM_PORT=15985
PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENV_PY="$HOME/vms/win10/.venv/bin/python3"
HELPER="$PROJECT_DIR/vms/win_pywinrm.py"

echo "==> Ensuring $DOM is running..."
if ! virsh --connect qemu:///session domstate "$DOM" 2>/dev/null | grep -q running; then
  virsh --connect qemu:///session start "$DOM"
fi

echo "==> Waiting for WinRM (127.0.0.1:$WINRM_PORT)..."
for i in $(seq 1 90); do
  if bash -c "echo > /dev/tcp/127.0.0.1/$WINRM_PORT" 2>/dev/null; then
    break
  fi
  sleep 2
  if [ "$i" -eq 90 ]; then
    echo "WinRM never came up after 180s" >&2
    exit 1
  fi
done

echo "==> Cross-compiling test binary (x86_64-pc-windows-msvc via cargo-xwin)..."
cd "$PROJECT_DIR"

# `JSON_OUT=$(...)` failing would otherwise trip `set -e` and exit the
# script right here, before the "Build failed" reporting below ever runs --
# same trap as the final test-run line further down.
set +e
JSON_OUT=$(cargo xwin test --no-run --target x86_64-pc-windows-msvc --lib --message-format=json 2>/tmp/os-next-wintest-build.log)
BUILD_STATUS=$?
set -e
if [ $BUILD_STATUS -ne 0 ]; then
  echo "Build failed:" >&2
  cat /tmp/os-next-wintest-build.log >&2
  exit $BUILD_STATUS
fi

EXE=$(echo "$JSON_OUT" | jq -r 'select(.reason=="compiler-artifact" and .profile.test==true and .target.kind[0]=="lib") | .executable' | grep -v null | tail -1)
if [ -z "$EXE" ]; then
  echo "Could not find compiled test executable in cargo output" >&2
  exit 1
fi
echo "    -> $EXE ($(du -h "$EXE" | cut -f1))"

echo "==> Preparing C:\\os-next-test (and excluding it from Defender real-time scanning --"
echo "    without this, Defender deletes the exe mid-transfer since it's an unsigned,"
echo "    incrementally-written binary from an unusual toolchain)..."
"$VENV_PY" "$HELPER" run 'powershell -NoProfile -Command "New-Item -ItemType Directory -Path C:\os-next-test -Force | Out-Null"' >/dev/null || true
"$VENV_PY" "$HELPER" run 'powershell -NoProfile -Command "Add-MpPreference -ExclusionPath C:\os-next-test -ErrorAction SilentlyContinue"' >/dev/null || true
"$VENV_PY" "$HELPER" run 'powershell -NoProfile -Command "Get-Process os_next_tests -ErrorAction SilentlyContinue | Stop-Process -Force"' >/dev/null || true

echo "==> Pushing test binary to VM..."
"$VENV_PY" "$HELPER" push "$EXE" 'C:\os-next-test\os_next_tests.exe'
# No WebView2Loader.dll to push: on x86_64-pc-windows-msvc, webview2-com-sys
# statically links WebView2LoaderStatic instead of dynamically importing
# WebView2Loader.dll (see docs/installer.md), and +crt-static
# (.cargo/config.toml) means no VCRUNTIME dependency either -- this exe is
# fully self-contained.

echo "==> Running tests on VM (via an interactive-logon scheduled task -- a plain"
echo "    WinRM shell can't touch Windows Credential Manager: it runs under a"
echo "    network-logon session, which DPAPI rejects with"
echo "    ERROR_NO_SUCH_LOGON_SESSION. This VM has autologon set up (an admin"
echo "    PowerShell open at boot), so a Scheduled Task with an Interactive-logon"
echo "    principal can execute inside that real session instead.)..."
FILTER="${1:-}"
set +e
"$VENV_PY" "$HELPER" run_interactive "C:\\os-next-test\\os_next_tests.exe --test-threads=1 $FILTER"
STATUS=$?
set -e

echo "==> Done (exit $STATUS)"
exit $STATUS
