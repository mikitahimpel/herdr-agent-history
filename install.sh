#!/bin/sh
# Agent History installer: downloads one pinned release archive, checks it
# against the SHA-256 written below, and puts three programs in ~/.local/bin.
#
#   curl -fsSL https://raw.githubusercontent.com/mikitahimpel/herdr-agent-history/main/install.sh | sh
#   ... | sh -s -- --dry-run          print what would happen; download and write nothing
#   ... | sh -s -- --tag v0.1.0-rc.1  install another release, checked against its published .sha256
#   ... | sh -s -- --prefix DIR       install into DIR (absolute) instead of ~/.local/bin
#
# It never uses sudo and never edits shell profiles or Herdr configuration.
# The only network access is the two release URLs printed before downloading.
set -eu

# Updated with every release. The hash is of the published archive.
pinned_tag=v0.1.0-rc.2
pinned_sha256=3050d9cbb13d1f3a13cb1bc6d50108a0d78606fd78a32507817b6d7cd403f04c

repo=mikitahimpel/herdr-agent-history
asset=agent-history-macos-arm64.tar.gz
binaries="agent-history agent-history-overlay agent-history-herdr"

fail() { echo "install.sh: $*" >&2; exit 1; }

# Everything runs from this function, called on the last line, so a download
# of this script cut off part way through runs nothing.
main() {
tag=$pinned_tag
prefix=${HOME:?HOME must be set}/.local/bin
dry_run=false
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run) dry_run=true ;;
    --tag) [ "$#" -ge 2 ] || fail "--tag needs a release tag"; tag=$2; shift ;;
    --prefix) [ "$#" -ge 2 ] || fail "--prefix needs a directory"; prefix=$2; shift ;;
    *) fail "unknown option: $1 (options: --dry-run, --tag TAG, --prefix DIR)" ;;
  esac
  shift
done
case "$tag" in v[0-9]*) ;; *) fail "--tag must be a release tag such as $pinned_tag" ;; esac
case "$tag" in *[!A-Za-z0-9._-]*) fail "--tag may contain only letters, digits and . _ -" ;; esac
case "$prefix" in /*) ;; *) fail "--prefix must be an absolute path" ;; esac
case "$prefix" in /|/usr|/usr/|/bin|/bin/|/sbin|/sbin/|/System|/System/) fail "refusing unsafe prefix $prefix" ;; esac

# The only build is for Apple Silicon. A Rosetta shell reports x86_64 but
# still runs arm64 programs natively.
if [ "$(uname -s)" != Darwin ]; then
  fail "Agent History is built only for macOS on Apple Silicon; this is $(uname -s)"
fi
if [ "$(uname -m)" != arm64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" != 1 ]; then
  fail "Agent History is built only for Apple Silicon Macs; this Mac is $(uname -m)"
fi

url=https://github.com/$repo/releases/download/$tag/$asset
if [ "$tag" = "$pinned_tag" ]; then
  expected=$pinned_sha256
  checksum_note="the SHA-256 pinned in this script"
else
  checksum_note="the SHA-256 published with $tag ($url.sha256)"
fi

echo "Agent History $tag for macOS (Apple Silicon)"
echo "  download  $url"
echo "  verify    against $checksum_note"
for name in $binaries; do
  if [ -e "$prefix/$name" ]; then
    echo "  replace   $prefix/$name (existing install, swapped by rename)"
  else
    echo "  create    $prefix/$name"
  fi
done
if $dry_run; then echo "Dry run: nothing downloaded or written."; exit 0; fi

work=$(mktemp -d "${TMPDIR:-/tmp}/agent-history-install.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM
fetch() { curl --proto '=https' --tlsv1.2 -fsSL -o "$2" "$1" || fail "download failed: $1"; }

if [ -z "${expected:-}" ]; then
  fetch "$url.sha256" "$work/$asset.sha256"
  expected=$(awk -v a="$asset" '$2 == a { print $1 }' "$work/$asset.sha256")
fi
case "$expected" in
  *[!0-9a-f]*|'') fail "no usable SHA-256 for $asset" ;;
esac
[ "${#expected}" -eq 64 ] || fail "no usable SHA-256 for $asset"
fetch "$url" "$work/$asset"
actual=$(shasum -a 256 "$work/$asset" | awk '{ print $1 }')
if [ "$actual" != "$expected" ]; then
  echo "install.sh: CHECKSUM MISMATCH for $asset" >&2
  echo "  expected $expected" >&2
  echo "  got      $actual" >&2
  fail "the download is corrupt or not the published release; nothing was installed"
fi
echo "Checksum verified: $actual"

tar -xzf "$work/$asset" -C "$work"
for name in $binaries; do
  [ -f "$work/agent-history/$name" ] || fail "the archive has no agent-history/$name; nothing was installed"
done

# Replace each directory entry by rename rather than writing through it. macOS
# checks a signature against the inode it already mapped, so overwriting a
# binary in place gets the next launch killed.
mkdir -p "$prefix"
for name in $binaries; do
  staged="$prefix/$name.install.$$"
  cp "$work/agent-history/$name" "$staged"
  chmod 755 "$staged"
  mv -f "$staged" "$prefix/$name"
done
echo "Installed $binaries in $prefix"
"$prefix/agent-history" --version

echo
case ":$PATH:" in
  *":$prefix:"*) ;;
  *)
    echo "$prefix is not on your PATH. Add it for new Terminal windows, then open one:"
    echo "  echo 'export PATH=\"$prefix:\$PATH\"' >> ~/.zshrc"
    echo ;;
esac
echo "Search your history:  agent-history browse"
echo "Resume from Herdr:    herdr plugin install $repo/plugin/agent-history --ref $tag"
echo "                      herdr plugin pane open --plugin agent-history --entrypoint search"
echo "Herdr starts the overlay with its server's PATH. If Herdr was already running"
echo "before $prefix was on PATH, the pane fails with 'No viable candidates found"
echo "in PATH': save your work, run 'herdr server stop' (closes every pane), and"
echo "start herdr again from a new Terminal window."
}

main "$@"
