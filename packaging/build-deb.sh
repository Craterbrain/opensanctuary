#!/bin/bash
# Builds both Linux release artifacts (docs/installer.md) inside the pinned
# Podman container defined by Dockerfile.linux-build, instead of directly on
# the host. This matters: a binary built directly on whatever machine
# happens to run this script links against *that machine's* glibc, which
# can easily be newer than what real Debian/Fedora users have (confirmed
# directly on a CachyOS dev machine: glibc 2.44 host produced a binary
# requiring GLIBC_2.39, new enough to fail to even start on Debian 12 stable
# or anything but the newest Fedora). The container pins that instead --
# see Dockerfile.linux-build's own comment for why it's Ubuntu 22.04
# specifically, not just "the oldest glibc available".
#
# Requires `podman` (or `docker` -- set CONTAINER_ENGINE=docker). No local
# Rust/bun/cargo-deb install needed; the container brings its own.
#
# Usage: packaging/build-deb.sh
# Output:
#   target/debian/opensanctuary_<version>-1_amd64.deb           (Debian/Ubuntu/Mint)
#   target/release-artifacts/opensanctuary-linux-x64-<version>.tar.gz  (everyone else)

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

ENGINE="${CONTAINER_ENGINE:-podman}"
IMAGE=os-next-linux-builder

echo "==> Building builder image ($ENGINE)..."
"$ENGINE" build -t "$IMAGE" -f packaging/Dockerfile.linux-build .

echo "==> Building inside the container..."
"$ENGINE" run --rm -v "$PWD":/src:Z -w /src "$IMAGE" packaging/build-deb-in-container.sh

DEB=$(ls -t target/debian/*.deb | head -1)
TARBALL=$(ls -t target/release-artifacts/*.tar.gz | head -1)
echo "==> Built: $DEB"
echo "==> Built: $TARBALL"
echo "$DEB"
echo "$TARBALL"
