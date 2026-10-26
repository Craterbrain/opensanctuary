#!/bin/sh
# Fails if the YouVersion key (YVP_APP_KEY) can be found as plain text in any file of the given release
# archives (.tar.gz, .zip, .deb). It is masked in the binary (build.rs, src/builtin_key_mask.rs), so
# `strings` and a text search must not find it. Prints no part of the key.
#   YVP_APP_KEY=... packaging/check-key-not-plain.sh target/release-artifacts/*.tar.gz ...
set -eu
if [ -z "${YVP_APP_KEY:-}" ]; then
  echo "YVP_APP_KEY is not set: nothing to check."
  exit 0
fi
bad=0
for archive in "$@"; do
  dir=$(mktemp -d)
  case "$archive" in
    *.tar.gz) tar -xzf "$archive" -C "$dir" ;;
    *.zip)    unzip -q "$archive" -d "$dir" ;;
    *.deb)    dpkg-deb -x "$archive" "$dir" ;;
    *) echo "unknown archive type: $archive"; exit 2 ;;
  esac
  if grep -raqF -- "$YVP_APP_KEY" "$dir"; then
    echo "::error::the YouVersion key is readable as plain text inside $archive"
    bad=1
  else
    echo "ok: no plain-text key in $archive"
  fi
  rm -rf "$dir"
done
exit $bad
