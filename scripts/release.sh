#!/usr/bin/env bash
# Usage: scripts/release.sh <version>   (e.g. v0.2.0 or 0.2.0)
#
# Checks, bumps Cargo.toml, cross-builds every target with zig, then tags,
# pushes and publishes a GitHub release. Nothing leaves this machine until
# every local step has passed.
set -euo pipefail

TARGETS=(
    "x86_64-unknown-linux-gnu  x86_64-linux-gnu.2.17"
    "aarch64-unknown-linux-gnu aarch64-linux-gnu.2.17"
    "x86_64-apple-darwin       x86_64-macos.10.12-none"
    "aarch64-apple-darwin      aarch64-macos.11.0-none"
)
BRANCH=main
OUT=dist
TARGET_DIR=${CARGO_TARGET_DIR:-target}

die() { printf 'release: %s\n' "$*" >&2; exit 1; }
step() { printf '\n==> %s\n' "$*" >&2; }

[[ $# -eq 1 ]] || die "usage: make release v<major>.<minor>.<patch>[-<pre>]"
version=${1#v}
[[ $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?$ ]] \
    || die "'$1' is not a semantic version (e.g. v0.2.0)"
tag=v$version

cd "$(git rev-parse --show-toplevel)"

step "Preflight"
for tool in git gh cargo rustup zig cargo-zigbuild tar xz sha256sum; do
    command -v "$tool" >/dev/null || die "$tool is not on PATH (zig and cargo-zigbuild: run 'mise install')"
done
[[ $(git branch --show-current) == "$BRANCH" ]] || die "releases are cut from $BRANCH (on '$(git branch --show-current)')"
[[ -z $(git status --porcelain) ]] || die "the working tree has uncommitted changes"
git remote get-url origin >/dev/null 2>&1 || die "no 'origin' remote"
git fetch --quiet --tags origin
if git rev-parse --quiet --verify "origin/$BRANCH" >/dev/null; then
    git merge-base --is-ancestor "origin/$BRANCH" HEAD || die "$BRANCH is behind origin/$BRANCH (pull first)"
fi
git rev-parse --quiet --verify "refs/tags/$tag" >/dev/null && die "tag $tag already exists locally"
git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null && die "tag $tag already exists on origin"
gh auth status >/dev/null 2>&1 || die "gh is not logged in (run 'gh auth login')"
gh release view "$tag" >/dev/null 2>&1 && die "a GitHub release $tag already exists"

current=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
if [[ $current != "$version" ]]; then
    newest=$(printf '%s\n%s\n' "$current" "$version" | sort -V | tail -n1)
    [[ $newest == "$version" ]] || die "$tag is older than the current version $current"
fi

start=$(git rev-parse HEAD)
restore() { git checkout --quiet -- Cargo.toml Cargo.lock; }
trap restore EXIT

if [[ $current != "$version" ]]; then
    step "Version $current -> $version"
    sed -i "0,/^version = \".*\"$/s//version = \"$version\"/" Cargo.toml
    cargo metadata --quiet --offline --format-version 1 >/dev/null
fi

step "Checks"
WORK_REQUIRE_SHELLS=1 make check
make supply-chain

step "Build"
rustup target add "${TARGETS[@]%% *}" >/dev/null 2>&1 \
    || die "could not add the Rust targets (rustup target add ${TARGETS[*]%% *})"
linkers=$(mktemp -d)
trap 'restore; rm -rf "$linkers"' EXIT
rm -rf "$OUT" && mkdir -p "$OUT"
for entry in "${TARGETS[@]}"; do
    read -r target zig_target <<<"$entry"
    # cargo-zigbuild drops any OS version from rustc's link line, so the
    # versioned zig target goes through a linker wrapper instead.
    linker=$linkers/cc-$target
    printf '#!/bin/sh\nexec cargo-zigbuild zig cc -- -g -fno-sanitize=all -target %s "$@"\n' "$zig_target" >"$linker"
    chmod +x "$linker"
    env_name=CARGO_TARGET_$(tr 'a-z-' 'A-Z_' <<<"$target")_LINKER
    env "$env_name=$linker" cargo build --quiet --locked --profile dist --target "$target"

    name=work-$target
    mkdir -p "$OUT/$name"
    cp "$TARGET_DIR/$target/dist/work" LICENSE README.md "$OUT/$name/"
    tar -cJf "$OUT/$name.tar.xz" -C "$OUT" "$name"
    rm -rf "${OUT:?}/$name"
    (cd "$OUT" && sha256sum "$name.tar.xz" >"$name.tar.xz.sha256")
    printf '  %s\n' "$OUT/$name.tar.xz" >&2
done
cp scripts/install.sh "$OUT/work-installer.sh"

host=$(uname -m)-unknown-linux-gnu
if [[ $(uname -s) == Linux && -x $TARGET_DIR/$host/dist/work ]]; then
    got=$("$TARGET_DIR/$host/dist/work" --version)
    [[ $got == "work $version" ]] || die "the built binary reports '$got', expected 'work $version'"
fi

step "Tag and push $tag"
if [[ $current != "$version" ]]; then
    git commit --quiet -m "Release $tag" -- Cargo.toml Cargo.lock
fi
trap 'rm -rf "$linkers"' EXIT
git tag -a "$tag" -m "work $tag"
if ! git push --quiet --atomic origin "HEAD:refs/heads/$BRANCH" "refs/tags/$tag"; then
    git tag -d "$tag" >/dev/null
    git reset --quiet --keep "$start"
    die "push failed; the local tag and release commit were removed"
fi

step "GitHub release $tag"
release_args=(--verify-tag --title "work $tag" --generate-notes)
[[ $version == *-* ]] && release_args+=(--prerelease)
if ! gh release create "$tag" "${release_args[@]}" "$OUT"/*; then
    die "$tag is pushed but the release failed; retry with: gh release create $tag ${release_args[*]} $OUT/*"
fi
printf '\nReleased %s\n' "$tag" >&2
