"""WinRM helpers for vms/windows-test.sh. Not meant to be run directly.

Uses raw pywinrm Protocol (not the high-level Session) for pushing the test
binary, because PowerShell's -EncodedCommand has an argv length ceiling far
below the size of a debug test binary — chunks are streamed over the WS-Man
Send operation's stdin instead of being embedded in the command line.
"""
import sys, base64, time, os
import winrm

HOST = "127.0.0.1:15985"
USER = "tester"
PASSWORD = "TestVM123!"
CHUNK = 8 * 1024 * 1024


def new_session():
    return winrm.Session(HOST, auth=(USER, PASSWORD), transport="basic",
                          server_cert_validation="ignore",
                          read_timeout_sec=90, operation_timeout_sec=75)


def remote_size(s, remote_path):
    r = s.run_ps(f'if (Test-Path "{remote_path}") {{ (Get-Item "{remote_path}").Length }} else {{ 0 }}')
    out = r.std_out.decode().strip()
    return int(out) if out.isdigit() else -1


def push_file(local_path, remote_path):
    """Resumable chunked push. Always resyncs against the ACTUAL remote file
    size before writing the next chunk, rather than trusting a locally-held
    counter -- on this VM, Defender's real-time scanner can silently
    truncate/delete a growing unsigned .exe out from under a transfer that
    the client otherwise saw succeed, so the remote side is the only source
    of truth. (Excluding the target dir via Add-MpPreference -ExclusionPath
    before pushing avoids this in the first place; this is the safety net.)
    """
    total_size = os.path.getsize(local_path)
    proc_name = os.path.splitext(os.path.basename(remote_path))[0]
    for attempt in range(10):
        s = new_session()
        # Also kill orphaned powershell.exe hosts left over from a chunk-write
        # whose client-side connection died mid-command: each holds the
        # target file open via its still-live $fs handle. Exclude $PID (this
        # command is itself a powershell.exe).
        s.run_ps(f'Get-Process "{proc_name}" -ErrorAction SilentlyContinue | Stop-Process -Force; '
                 f'Get-Process powershell -ErrorAction SilentlyContinue | Where-Object {{ $_.Id -ne $PID }} | Stop-Process -Force')
        r = s.run_ps(f'if (Test-Path "{remote_path}") {{ Remove-Item "{remote_path}" -Force }}')
        if r.status_code == 0:
            break
        print(f"  cleanup attempt {attempt+1} failed (file likely still locked): {r.std_err.decode()[:200]}", flush=True)
        time.sleep(2)
    else:
        raise RuntimeError(f"could not clear {remote_path} for a fresh transfer: {r.std_err.decode()}")

    t0 = time.time()
    sent = 0
    stalls = 0
    MAX_STALLS = 60
    with open(local_path, "rb") as f:
        while sent < total_size:
            try:
                cur = remote_size(new_session(), remote_path)
            except Exception as e:
                print(f"  resync check failed: {type(e).__name__}: {e}", flush=True)
                cur = -1
            if cur < 0:
                time.sleep(2)
                stalls += 1
                if stalls > MAX_STALLS:
                    print("GIVING UP: too many failed resync attempts")
                    sys.exit(1)
                continue
            if cur != sent:
                print(f"  resync: remote has {cur/1024/1024:.1f}MB, expected {sent/1024/1024:.1f}MB -- adjusting", flush=True)
                sent = cur
                if sent >= total_size:
                    break

            f.seek(sent)
            chunk = f.read(CHUNK)
            expected_len = len(chunk)

            try:
                s2 = new_session()
                p = s2.protocol
                ps_script = (
                    f"$fs=[System.IO.File]::Open('{remote_path}',[System.IO.FileMode]::Append);"
                    "$stdin=[Console]::OpenStandardInput();"
                    "$ms=New-Object System.IO.MemoryStream;"
                    "$stdin.CopyTo($ms);"
                    "$b=$ms.ToArray();"
                    "$fs.Write($b,0,$b.Length);$fs.Close();"
                    "Write-Output $b.Length"
                )
                encoded = base64.b64encode(ps_script.encode("utf-16-le")).decode()
                shell_id = p.open_shell()
                command_id = p.run_command(shell_id, "powershell", ["-NoProfile", "-EncodedCommand", encoded], console_mode_stdin=True)
                p.send_command_input(shell_id, command_id, chunk, end=True)
                stdout, stderr, rc = p.get_command_output(shell_id, command_id)
                p.cleanup_command(shell_id, command_id)
                p.close_shell(shell_id)

                if rc == 0 and stdout.decode().strip() == str(expected_len):
                    sent += expected_len
                    stalls = 0
                    print(f"  progress: {sent/1024/1024:.1f} / {total_size/1024/1024:.1f} MB", flush=True)
                else:
                    print(f"  write at {sent} bad result rc={rc} out={stdout!r} err={stderr!r}", flush=True)
                    stalls += 1
            except Exception as e:
                print(f"  write at {sent} exception: {type(e).__name__}: {e}", flush=True)
                stalls += 1

            if stalls > MAX_STALLS:
                print("GIVING UP: too many failed write attempts without progress")
                sys.exit(1)
            if stalls:
                time.sleep(min(2 * stalls, 15))

    print(f"pushed {local_path} -> {remote_path} in {time.time()-t0:.1f}s")


def run(remote_cmd):
    """Run a command line on the VM (Windows cmd), stream stdout/stderr, return exit code."""
    s = new_session()
    r = s.run_cmd(remote_cmd)
    sys.stdout.write(r.std_out.decode(errors="replace"))
    sys.stderr.write(r.std_err.decode(errors="replace"))
    return r.status_code


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "push":
        push_file(sys.argv[2], sys.argv[3])
    elif cmd == "run":
        sys.exit(run(sys.argv[2]))
    else:
        print(f"unknown command {cmd}", file=sys.stderr)
        sys.exit(2)
