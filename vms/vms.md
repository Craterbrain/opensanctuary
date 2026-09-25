# Test VMs

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
the test binary locally with zig-as-mingw (`x86_64-pc-windows-gnu`, real
Windows SEH unwinding so `panic=unwind` test builds link), excludes the
target folder from Defender, pushes the exe + `WebView2Loader.dll` over
WinRM, runs it on the VM, streams results, and exits with the real test exit
code.

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

## Known flaky test on both VMs

`network::tls::tests::test_tls_private_key_storage_and_legacy_migration`
sometimes fails on a *fresh* VM/session: it round-trips a cert through the
OS keyring (GNOME Keyring / Windows Credential Manager), and a non-interactive
remote session (SSH or WinRM) doesn't always have that unlocked/consistent
the way a real interactive login does. Not a code bug — `linux-test.sh`
already unlocks the keyring before testing; if it recurs, rerun once more.
