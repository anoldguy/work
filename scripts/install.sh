#!/bin/sh
# Installs work from a GitHub release:
#
#   curl -LsSf https://github.com/anoldguy/work/releases/latest/download/work-installer.sh | sh
#
# WORK_VERSION picks a release (default: latest); WORK_INSTALL_DIR picks the
# directory (default: ~/.local/bin).
set -eu

repo=anoldguy/work
version=${WORK_VERSION:-latest}
dir=${WORK_INSTALL_DIR:-$HOME/.local/bin}

die() { printf 'work-installer: %s\n' "$*" >&2; exit 1; }

case $(uname -s) in
    Linux) os=unknown-linux-gnu ;;
    Darwin) os=apple-darwin ;;
    *) die "unsupported OS: $(uname -s) (Linux and macOS only)" ;;
esac
case $(uname -m) in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) die "unsupported CPU: $(uname -m)" ;;
esac
# A shell running under Rosetta reports x86_64 on Apple silicon.
if [ "$os" = apple-darwin ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = 1 ]; then
    arch=aarch64
fi

for tool in curl tar; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is required"
done
if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
    die "sha256sum or shasum is required"
fi

if [ "$version" = latest ]; then
    base=https://github.com/$repo/releases/latest/download
else
    base=https://github.com/$repo/releases/download/v${version#v}
fi
name=work-$arch-$os
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

printf 'Downloading %s\n' "$base/$name.tar.xz" >&2
curl --proto '=https' --tlsv1.2 -fsSL -o "$tmp/$name.tar.xz" "$base/$name.tar.xz" \
    || die "download failed"
curl --proto '=https' --tlsv1.2 -fsSL -o "$tmp/$name.tar.xz.sha256" "$base/$name.tar.xz.sha256" \
    || die "checksum download failed"
expected=$(cut -d' ' -f1 "$tmp/$name.tar.xz.sha256")
[ "$(sha256 "$tmp/$name.tar.xz")" = "$expected" ] || die "checksum mismatch for $name.tar.xz"

tar -xJf "$tmp/$name.tar.xz" -C "$tmp" || die "could not unpack $name.tar.xz (is xz installed?)"
mkdir -p "$dir"
cp "$tmp/$name/work" "$dir/.work.$$"
chmod 755 "$dir/.work.$$"
mv -f "$dir/.work.$$" "$dir/work"
printf 'Installed %s to %s\n' "$("$dir/work" --version)" "$dir/work" >&2

case ":$PATH:" in
    *":$dir:"*) ;;
    *) printf '%s is not on your PATH; add it before using work.\n' "$dir" >&2 ;;
esac
printf 'Next: add shell integration (eval "$(work activate bash)", or zsh/fish), then run work setup.\n' >&2
