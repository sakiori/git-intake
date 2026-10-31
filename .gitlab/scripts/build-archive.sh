#!/bin/sh
# Builds the release binary of git-intake for one target and packs it as
# dist/git-intake-v<version>-<target>.tar.gz, with a SHA-256 checksum file
# next to it, <version> being the package version in Cargo.toml. The archive
# holds exactly:
#
#   git-intake                         the binary
#   LICENSE                            its license
#   THIRD-PARTY-NOTICES.txt            the notices of the code it links
#   RUST-LIBRARY-COPYRIGHT.html        the Rust standard library's notices
#   man/man1/git-intake.1              the manual pages
#   man/man1/git-intake-<command>.1
#
# The Linux targets are built with cargo-zigbuild, which links them with zig,
# statically against the musl zig ships; any host with zig and cargo-zigbuild
# on the PATH builds them. The macOS targets are built on macOS with cargo
# alone, which links them with Apple's toolchain of the Xcode Command Line
# Tools against the system's libraries and frameworks, Security among them,
# through which the tool verifies TLS certificates; an Apple silicon Mac
# builds both. MACOSX_DEPLOYMENT_TARGET is set here, so that a build by hand
# and the pipeline's agree on the oldest macOS they run on. The xtask package
# generates the notices on the build host: those of the crates, with
# cargo-about, of the Rust toolchain's compiler-builtins, from its rust-src
# component, and for a Linux target of the zig distribution and the musl it
# links; and it copies the toolchain's COPYRIGHT-library.html. It also
# generates the manual pages and packs the archive. Cargo.toml's
# [package.metadata.binstall] describes this layout to cargo-binstall. The
# pipeline runs this script for the Linux targets in merge requests that can
# change the build, and for every target for a release tag, the macOS
# targets in a fresh macOS virtual machine on the project's macOS runner.
#
# usage: .gitlab/scripts/build-archive.sh <target>
set -eu

target=${1:?usage: build-archive.sh <target>}
case "$target" in
  x86_64-unknown-linux-musl|aarch64-unknown-linux-musl) ;;
  x86_64-apple-darwin|aarch64-apple-darwin)
    [ "$(uname -s)" = Darwin ] || { echo "$target is built on macOS, with the Xcode Command Line Tools" >&2; exit 1; }
    # The oldest macOS both binaries run on: the first release for Apple
    # silicon, and Rust's minimum for aarch64-apple-darwin.
    MACOSX_DEPLOYMENT_TARGET=11.0
    export MACOSX_DEPLOYMENT_TARGET
    ;;
  *) echo "not a release target: $target" >&2; exit 1 ;;
esac

version=$(sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' Cargo.toml)
[ -n "$version" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }

# rustc writes paths of the build host into the binary, in panic locations
# and the like. They are remapped: the workspace to git-intake, the
# toolchain's sysroot to rust, and CARGO_HOME to cargo, with its registry
# sources to registry and its git checkouts to git. rustc applies the last
# remapping that matches, so the more specific ones come later. Cargo's
# trim-paths would do this, but it is not stable. Apple's linker also hashes
# the paths of the objects it links, which lie in the workspace, into the
# binary's UUID before the binary is stripped of them, so for a macOS target
# it is told to drop the workspace from them (-oso_prefix), and the same
# commit gives the same binary in any directory. The flags are added to
# CARGO_ENCODED_RUSTFLAGS, or to RUSTFLAGS, when one is set, since either
# overrides the rustflags of Cargo's configuration; otherwise they are given
# as build.rustflags with --config, which adds to the configuration's.
# cargo-zigbuild passes all of them on to cargo. xtask check-archive fails on
# any host path left in the archive; on macOS it also looks for the active
# developer directory and the SDK, which Apple's toolchain links with.
workspace=$(pwd -P)
sysroot=$(rustc --print sysroot)
cargo_home=${CARGO_HOME:-$HOME/.cargo}
sep=$(printf '\037')
flags=""
toml=""
add_flag() { # <rustc flag>
  flags="${flags:+$flags$sep}$1"
  quoted=$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')
  toml="${toml:+$toml, }\"$quoted\""
}
for remap in "$workspace=git-intake" "$sysroot=rust" "$cargo_home=cargo" \
  "$cargo_home/registry/src=registry" "$cargo_home/git/checkouts=git"; do
  add_flag "--remap-path-prefix=$remap"
done
case "$target" in
  *-apple-darwin)
    # -Xlinker passes the path whole, whatever it holds.
    for arg in -Xlinker -oso_prefix -Xlinker "$workspace/"; do
      add_flag "-Clink-arg=$arg"
    done
    ;;
esac
set -- --release --locked --target "$target"
if [ -n "${CARGO_ENCODED_RUSTFLAGS+set}" ]; then
  CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS:+$CARGO_ENCODED_RUSTFLAGS$sep}$flags"
  export CARGO_ENCODED_RUSTFLAGS
elif [ -n "${RUSTFLAGS+set}" ]; then
  # Cargo splits RUSTFLAGS at whitespace.
  CARGO_ENCODED_RUSTFLAGS=$(printf '%s' "$RUSTFLAGS" | tr -s ' \t\n' '\037' | sed "s/^$sep//; s/$sep\$//")
  CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS:+$CARGO_ENCODED_RUSTFLAGS$sep}$flags"
  export CARGO_ENCODED_RUSTFLAGS
  unset RUSTFLAGS
else
  set -- "$@" --config "build.rustflags = [$toml]"
fi
case "$target" in
  *-linux-musl) cargo zigbuild "$@" ;;
  *-apple-darwin)
    tools="$target needs the Xcode Command Line Tools (xcode-select --install)"
    developer=$(xcode-select -p) || { echo "$tools" >&2; exit 1; }
    sdk=$(xcrun --show-sdk-path) || { echo "$tools" >&2; exit 1; }
    cargo build "$@"
    ;;
esac

name="git-intake-v$version-$target"
stage="target/stage/$name"
rm -rf "$stage"
mkdir -p "$stage"
cp "target/$target/release/git-intake" LICENSE "$stage/"
cargo run --locked --quiet -p xtask -- notices "$stage" "$target"
cargo run --locked --quiet -p xtask -- man "$stage/man/man1"

mkdir -p dist
archive="dist/$name.tar.gz"
# xtask pack packs the archive deterministically, so that it carries nothing
# of the build host and the same build gives the same bytes on either host:
# GNU tar headers without PAX records, names sorted, owner and group 0
# without names, every modification time the commit's time, modes 0755 and
# 0644, no extended attributes, and gzip without a file name or time. xtask
# check-archive reads the raw headers back and scans every file for host
# paths.
epoch=$(git log -1 --format=%ct)
cargo run --locked --quiet -p xtask -- pack "$stage" "$archive" "$epoch"
set -- "$workspace" "$sysroot" "$cargo_home" "$HOME/"
case "$target" in
  *-apple-darwin) set -- "$@" "$developer" "$sdk" ;;
esac
cargo run --locked --quiet -p xtask -- check-archive "$archive" "$epoch" "$@"

# The layout check: the files above and nothing else, the main page and
# command pages among them, every section of the notices the target needs,
# and a binary that reports the version when the build host can run it.
# tar and find write to files first, since a pipe would hide their failure.
tar -tzf "$archive" > "$stage.listing"
listing=$(sed 's|/$||' "$stage.listing" | LC_ALL=C sort)
(cd "$stage" && find man -type f) > "$stage.pages"
pages=$(LC_ALL=C sort "$stage.pages")
rm "$stage.listing" "$stage.pages"
expected=$(printf '%s\n' git-intake LICENSE THIRD-PARTY-NOTICES.txt RUST-LIBRARY-COPYRIGHT.html \
  man man/man1 "$pages" | LC_ALL=C sort)
if [ "$listing" != "$expected" ]; then
  echo "$archive does not have the release layout; it holds:" >&2
  echo "$listing" >&2
  exit 1
fi
for page in man/man1/git-intake.1 man/man1/git-intake-start.1 man/man1/git-intake-sync.1; do
  printf '%s\n' "$pages" | grep -Fqx "$page" || { echo "$archive lacks $page" >&2; exit 1; }
done
notices="$stage/THIRD-PARTY-NOTICES.txt"
for heading in 'Third-party notices of git-intake' 'The Rust standard library and compiler-builtins' 'libm of compiler-builtins'; do
  grep -Fq "$heading" "$notices" || { echo "the notices lack '$heading'" >&2; exit 1; }
done
case "$target" in
  *-linux-musl)
    grep -Eq '^Zig [0-9]' "$notices" || { echo "the notices lack Zig's" >&2; exit 1; }
    grep -Eq '^musl libc [0-9]' "$notices" || { echo "the notices lack musl's" >&2; exit 1; }
    ;;
esac
grep -Fq 'Copyright notices for The Rust Standard Library' "$stage/RUST-LIBRARY-COPYRIGHT.html" \
  || { echo "RUST-LIBRARY-COPYRIGHT.html is not the standard library's notices" >&2; exit 1; }
# An Apple silicon Mac runs the x86_64 macOS binary through Rosetta when
# Rosetta is installed, as it is in the pipeline's macOS image; without it,
# the check is skipped.
run=""
case "$(uname -s)-$(uname -m)-$target" in
  Linux-x86_64-x86_64-unknown-linux-*|Linux-aarch64-aarch64-unknown-linux-*|Darwin-arm64-aarch64-apple-darwin)
    run=native
    ;;
  Darwin-arm64-x86_64-apple-darwin)
    if arch -x86_64 /usr/bin/true 2>/dev/null; then
      run=rosetta
    else
      echo "the $target binary is not run: Rosetta is not installed (softwareupdate --install-rosetta)"
    fi
    ;;
  *) echo "the $target binary is not run: this host cannot run it" ;;
esac
if [ -n "$run" ]; then
  # The binary as the archive holds it.
  unpacked="target/stage/$name.unpacked"
  rm -rf "$unpacked"
  mkdir -p "$unpacked"
  tar -xzf "$archive" -C "$unpacked" git-intake
  if [ "$run" = rosetta ]; then
    reported=$(arch -x86_64 "$unpacked/git-intake" --version)
  else
    reported=$("$unpacked/git-intake" --version)
  fi
  rm -rf "$unpacked"
  [ "$reported" = "git-intake $version" ] || { echo "the binary reports '$reported', not version $version" >&2; exit 1; }
  echo "the $target binary reports version $version"
fi

case "$(uname -s)" in
  Darwin) sha256() { shasum -a 256 "$@"; } ;;
  *) sha256() { sha256sum "$@"; } ;;
esac
(cd dist && sha256 "$name.tar.gz" > "$name.tar.gz.sha256")
cat "$archive.sha256"
echo "$listing"
