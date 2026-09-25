#!/bin/bash
# Fully automated native Linux test run on the Mint sandbox VM.
#
# Why this exists: the host and the VM have different glibc versions, so a
# binary built on the host will not reliably run on (or match the behavior
# of) a real end-user Linux machine. This VM gives a clean, disposable,
# real-root Linux environment to build and test natively in.
#
# Run this on demand when you need to validate Linux-specific behavior
# (e.g. keyring/DBus code, filesystem/permissions code) — not on every change.
#
# Usage: vms/linux-test.sh [cargo test args...]
#   vms/linux-test.sh                     # cargo test --lib
#   vms/linux-test.sh network::tls::      # run a specific test filter

set -euo pipefail

VM_HOME="$HOME/vms/mint"
DOM="os-next-mint-test"
SSH_PORT=12222
SSH_KEY="$VM_HOME/ssh_key"
REMOTE_DIR="/home/tester/os-next"
PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

SSH="ssh -i $SSH_KEY -p $SSH_PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"

echo "==> Ensuring $DOM is running..."
if ! virsh --connect qemu:///session domstate "$DOM" 2>/dev/null | grep -q running; then
  virsh --connect qemu:///session start "$DOM"
fi

echo "==> Waiting for SSH..."
for i in $(seq 1 60); do
  if $SSH -o ConnectTimeout=3 tester@127.0.0.1 true 2>/dev/null; then
    break
  fi
  sleep 2
  if [ "$i" -eq 60 ]; then
    echo "SSH never came up after 120s" >&2
    exit 1
  fi
done

echo "==> Syncing source to VM..."
rsync -az --delete \
  --exclude target --exclude .git --exclude node_modules \
  --exclude 'web/node_modules' --exclude 'web/dist' --exclude dist \
  --exclude '*.db' --exclude '*.db-shm' --exclude '*.db-wal' \
  --exclude '*.qcow2' --exclude '*.iso' \
  -e "ssh -i $SSH_KEY -p $SSH_PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR" \
  "$PROJECT_DIR/" "tester@127.0.0.1:$REMOTE_DIR/"

echo "==> Running cargo test on VM (native build, real glibc)..."
TEST_ARGS="$*"
if [ -z "$TEST_ARGS" ]; then
  TEST_ARGS="--lib"
fi

# Unlock the user's keyring for this SSH session (a plain SSH login doesn't
# get a graphical-login-driven unlocked gnome-keyring by default; several
# tests depend on the Secret Service API). Harmless if already unlocked.
$SSH tester@127.0.0.1 "echo -n 'TestVM123!' | gnome-keyring-daemon --replace --daemonize --unlock >/dev/null 2>&1; true"

$SSH tester@127.0.0.1 "cd $REMOTE_DIR && source \$HOME/.cargo/env && cargo test $TEST_ARGS 2>&1"
STATUS=$?

echo "==> Done (exit $STATUS)"
exit $STATUS
