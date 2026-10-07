# Test VMs

_Last edited: 2026-09-26 14:57_

Two disposable sandbox VMs (unprivileged QEMU/KVM, `qemu:///session` — no host
root needed) for testing platform-specific code that can't be validated on
the host or under Wine. Run these **on demand** when you're touching
platform-specific code (`src/network/linux.rs`, `src/network/windows.rs`,
keyring/DBus, webview) — not on every change; they're slow (VM boot, full
cross-compile, multi-hundred-MB transfer for Windows).

VM disk images live outside the repo at `~/vms/mint/` and `~/vms/win10/`
(tens of GB each). This `vms/` folder only holds the automation scripts.

## Linux (Mint 22.3 Cinnamon) — `vms/linux-test.sh`

Fully automated: starts the VM if needed, waits for SSH, rsyncs the repo
over, unlocks the user's keyring, runs `cargo test` natively in-VM, streams
results, and exits with the real test exit code.

```
vms/linux-test.sh                 # cargo test --lib
vms/linux-test.sh network::tls::  # filtered
```

Why in-VM and not just on the host: the VM's glibc differs from the host's,
so a host-built binary isn't representative — this builds natively inside
the VM instead of cross-compiling.

- SSH: `ssh -i ~/vms/mint/ssh_key -p 12222 tester@127.0.0.1`
- Login: `tester` / `TestVM123!` (passwordless sudo also configured)
- Manual VM control + VNC: `~/vms/mint/start.sh`

## Windows 10 — `vms/windows-test.sh`

Fully automated: starts the VM if needed, waits for WinRM, cross-compiles
the test binary locally via `cargo xwin` (`x86_64-pc-windows-msvc` -- see
docs/installer.md and .cargo/config.toml for why this replaced the earlier
mingw-w64 cross-compile), excludes the target folder from Defender, pushes
the exe over WinRM (no `WebView2Loader.dll` needed -- statically linked on
this target), runs it on the VM, streams results, and exits with the real
test exit code.

```
vms/windows-test.sh                              # all lib tests
vms/windows-test.sh check_hyperv_support          # filtered
```

- WinRM: `127.0.0.1:15985` (host-forwarded from guest 5985)
- Login: `tester` / `TestVM123!`
- Manual VM control + VNC: `~/vms/win10/start.sh`
- `vms/win_pywinrm.py` is a helper library (push/run over WinRM), not meant
  to be run directly — it also self-heals two VM quirks automatically:
  Defender silently truncating a growing unsigned exe mid-transfer, and
  orphaned `powershell.exe` processes left holding the file open after an
  interrupted transfer.
- The VM has `AutoAdminLogon` configured for `tester`
  (`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon`) — it boots
  straight to an interactive desktop with zero manual interaction (no
  password prompt, no lock-screen keypress needed). This isn't cosmetic:
  the actual test run depends on it (see below).
- The test binary itself is **not** run directly over the WinRM shell.
  `win_pywinrm.py`'s `run_interactive()` instead registers and triggers a
  Scheduled Task (`LogonType=Interactive`, `RunLevel=Highest`, principal
  `tester`) that executes it inside that already-logged-on desktop session,
  polls for a written exit-code file, then reads back stdout/exit code and
  unregisters the task. Plain WinRM (`run()`, still used for the
  Defender-exclusion/process-kill setup steps) stays a network-logon
  session and can't be used for the test binary itself — see below.

### Why the test binary needs a real interactive session

`network::tls::tests::test_tls_private_key_storage_and_legacy_migration`
round-trips a cert/key through the OS keyring. Under a plain WinRM remote
shell this failed **every time** (confirmed 3/3, not a flake) with a
specific, diagnosed cause: Windows Credential Manager (DPAPI-backed)
returns `ERROR_NO_SUCH_LOGON_SESSION` for any process whose logon session
isn't a real interactive one, and WinRM's remote shell always runs under a
network-logon session — no amount of retrying fixes it. `run_interactive()`
(above) works around this by executing the exe inside the VM's real
autologon desktop session instead, and the keyring test now passes cleanly
(confirmed 67/67, including this test, via a full `windows-test.sh` run).
This was never a bug in `get_or_create_tls_certificate` or the keyring
service, only in how the test binary was being launched.

## Shutting down

`vms/shutdown.sh` stops every running `os-next-*` VM (graceful ACPI shutdown,
falling back to a forced `virsh destroy` after 20s for one that doesn't
respond -- these are disposable test sandboxes, so a forced stop is fine).
Neither test script stops its VM when done, so run this when you're finished
testing rather than leaving them idling.

## Keyring test status on both VMs

`network::tls::tests::test_tls_private_key_storage_and_legacy_migration`
passes cleanly on both VMs now (see the Windows section above for what
that took). On the Linux VM it's `linux-test.sh`'s existing GNOME Keyring
unlock step that keeps it passing (confirmed 4/4 clean runs); if it ever
does fail on Linux, rerun once before assuming it's a real regression.
