#!/bin/bash
# Shuts down every sandbox VM tied to this project (both currently:
# os-next-win10-test, os-next-mint-test). Run after a vms/linux-test.sh or
# vms/windows-test.sh session when you're done testing -- they don't stop
# themselves, so a VM left running just idles until something else needs it.
#
# Tries a graceful ACPI shutdown first (`virsh shutdown`), then falls back to
# a hard `virsh destroy` for any domain still running after the grace period
# -- these are disposable test sandboxes, not machines with state worth
# preserving, so a forced stop here is safe.
#
# Usage: vms/shutdown.sh

set -euo pipefail

CONNECT="qemu:///session"
GRACE_SECONDS=20

mapfile -t DOMAINS < <(virsh --connect "$CONNECT" list --name --state-running | grep '^os-next-' || true)

if [ ${#DOMAINS[@]} -eq 0 ]; then
  echo "==> No running os-next-* VMs."
  exit 0
fi

echo "==> Requesting graceful shutdown: ${DOMAINS[*]}"
for dom in "${DOMAINS[@]}"; do
  virsh --connect "$CONNECT" shutdown "$dom" || true
done

echo "==> Waiting up to ${GRACE_SECONDS}s for shutdown..."
for i in $(seq 1 "$GRACE_SECONDS"); do
  STILL_RUNNING=()
  for dom in "${DOMAINS[@]}"; do
    if virsh --connect "$CONNECT" domstate "$dom" 2>/dev/null | grep -q running; then
      STILL_RUNNING+=("$dom")
    fi
  done
  if [ ${#STILL_RUNNING[@]} -eq 0 ]; then
    break
  fi
  sleep 1
done

for dom in "${DOMAINS[@]}"; do
  if virsh --connect "$CONNECT" domstate "$dom" 2>/dev/null | grep -q running; then
    echo "==> $dom didn't shut down gracefully -- forcing off."
    virsh --connect "$CONNECT" destroy "$dom" || true
  else
    echo "==> $dom stopped."
  fi
done

echo "==> Done."
