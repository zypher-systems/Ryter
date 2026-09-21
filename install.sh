#!/bin/sh
# Install Ryter from its GitHub releases.
#
#   curl -fsSL https://raw.githubusercontent.com/zypher-systems/ryter/main/install.sh | sh
#
# Environment:
#   RYTER_VERSION      a release tag like v0.2.0 (default: the latest release)
#   RYTER_INSTALL_DIR  where the binary goes (default: ~/.local/bin)
#   RYTER_DOWNLOAD_URL where release files are (default: GitHub; for mirrors)
#
# Linux (x86_64, arm64; static, any distro) and macOS (Apple Silicon, Intel).
# Every download is checked against the release's SHA256SUMS before install.

set -eu

REPO="zypher-systems/ryter"
DIR="${RYTER_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
die() { printf 'ryter install: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "needs $1"; }

need uname
need tar
need mkdir
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q "$1" -O "$2"; }
else
    die "needs curl or wget"
fi

os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Linux) sys="unknown-linux-musl" ;;
    Darwin) sys="apple-darwin" ;;
    *) die "no prebuilt binary for $os yet; build from source: cargo install --git https://github.com/$REPO ryter-cli" ;;
esac
case "$arch" in
    x86_64 | amd64) cpu="x86_64" ;;
    arm64 | aarch64) cpu="aarch64" ;;
    *) die "no prebuilt binary for $arch yet; build from source: cargo install --git https://github.com/$REPO ryter-cli" ;;
esac
target="$cpu-$sys"
asset="ryter-$target.tar.gz"

if [ -n "${RYTER_DOWNLOAD_URL:-}" ]; then
    base="$RYTER_DOWNLOAD_URL"
elif [ -n "${RYTER_VERSION:-}" ]; then
    base="https://github.com/$REPO/releases/download/$RYTER_VERSION"
else
    base="https://github.com/$REPO/releases/latest/download"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading $asset ${RYTER_VERSION:-(latest)}"
fetch "$base/$asset" "$tmp/$asset" || die "download failed: $base/$asset"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "download failed: $base/SHA256SUMS"

want=$(grep " $asset\$" "$tmp/SHA256SUMS" | cut -d ' ' -f 1)
[ -n "$want" ] || die "$asset is not listed in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
    got=$(sha256sum "$tmp/$asset" | cut -d ' ' -f 1)
elif command -v shasum >/dev/null 2>&1; then
    got=$(shasum -a 256 "$tmp/$asset" | cut -d ' ' -f 1)
else
    die "needs sha256sum or shasum to verify the download"
fi
[ "$want" = "$got" ] || die "checksum mismatch for $asset: refusing to install"

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$DIR"
# Replace atomically, so a running ryter keeps working until it exits.
cp "$tmp/ryter-$target/ryter" "$DIR/.ryter.new"
chmod 755 "$DIR/.ryter.new"
mv "$DIR/.ryter.new" "$DIR/ryter"

say "installed $("$DIR/ryter" --version 2>/dev/null || echo ryter) to $DIR/ryter"
case ":$PATH:" in
    *":$DIR:"*) ;;
    *) say "note: $DIR is not on your PATH. Add it, e.g. to ~/.profile:"
       say "  export PATH=\"$DIR:\$PATH\"" ;;
esac
say "next: cd into a project and run: ryter"
