#!/bin/bash
# Fully automated Windows test run on the Windows 10 sandbox VM.
#
# Why this exists: src/network/windows.rs shells out to real powershell.exe
# for Hyper-V detection etc. — untestable under Wine and meaningless on Linux.
# This cross-compiles the test binary with zig-as-mingw, pushes it to a real
# Windows VM over WinRM, and runs it there.
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

MINGW_GCC="/home/jasonb/.local/bin/x86_64-w64-mingw32-gcc"
MINGW_GXX="/home/jasonb/.local/bin/x86_64-w64-mingw32-g++"
MINGW_AR="/home/jasonb/.local/bin/x86_64-w64-mingw32-ar"

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

echo "==> Cross-compiling test binary (x86_64-pc-windows-gnu, real Windows unwinding)..."
cd "$PROJECT_DIR"
export CC_x86_64_pc_windows_gnu="$MINGW_GCC"
export CXX_x86_64_pc_windows_gnu="$MINGW_GXX"
export AR_x86_64_pc_windows_gnu="$MINGW_AR"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$MINGW_GCC"
export OSNEXT_LINK_ZIG_UNWIND=1

JSON_OUT=$(cargo test --no-run --target x86_64-pc-windows-gnu --lib --message-format=json 2>/tmp/os-next-wintest-build.log)
BUILD_STATUS=$?
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

DLL="$PROJECT_DIR/target/x86_64-pc-windows-gnu/debug/build/webview2-com-sys-*/out/x64/WebView2Loader.dll"
DLL=$(ls $DLL 2>/dev/null | head -1)

echo "==> Preparing C:\\os-next-test (and excluding it from Defender real-time scanning --"
echo "    without this, Defender deletes the exe mid-transfer since it's an unsigned,"
echo "    incrementally-written binary from an unusual toolchain)..."
"$VENV_PY" "$HELPER" run 'powershell -NoProfile -Command "New-Item -ItemType Directory -Path C:\os-next-test -Force | Out-Null"' >/dev/null || true
"$VENV_PY" "$HELPER" run 'powershell -NoProfile -Command "Add-MpPreference -ExclusionPath C:\os-next-test -ErrorAction SilentlyContinue"' >/dev/null || true
"$VENV_PY" "$HELPER" run 'powershell -NoProfile -Command "Get-Process os_next_tests -ErrorAction SilentlyContinue | Stop-Process -Force"' >/dev/null || true

echo "==> Pushing test binary to VM..."
"$VENV_PY" "$HELPER" push "$EXE" 'C:\os-next-test\os_next_tests.exe'

if [ -n "$DLL" ]; then
  echo "==> Pushing WebView2Loader.dll (needed to resolve the exe's import table)..."
  "$VENV_PY" "$HELPER" push "$DLL" 'C:\os-next-test\WebView2Loader.dll'
fi

echo "==> Running tests on VM..."
FILTER="${1:-}"
"$VENV_PY" "$HELPER" run "C:\\os-next-test\\os_next_tests.exe --test-threads=1 $FILTER"
STATUS=$?

echo "==> Done (exit $STATUS)"
exit $STATUS
