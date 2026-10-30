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
# The xtask package generates the notices on the build host: those of the
# crates, with cargo-about, of the Rust toolchain's compiler-builtins, from its
# rust-src component, and for a musl target of the musl and the libc and
# compiler_rt that zig links; and it
# copies the toolchain's COPYRIGHT-library.html. It also generates the manual
# pages. Cargo.toml's [package.metadata.binstall] describes
# this layout to cargo-binstall. CI runs this script on every change, and the
# release workflow runs it for a release tag.
#
# usage: .github/scripts/build-archive.sh <target> <cargo | cargo-zigbuild>
set -eu

target=${1:?usage: build-archive.sh <target> <cargo | cargo-zigbuild>}
tool=${2:?usage: build-archive.sh <target> <cargo | cargo-zigbuild>}
case "$tool" in
  cargo) build=build ;;
  cargo-zigbuild) build=zigbuild ;;
  *) echo "unknown build tool: $tool" >&2; exit 1 ;;
esac

version=$(sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' Cargo.toml)
[ -n "$version" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }

cargo "$build" --release --locked --target "$target"

name="git-intake-v$version-$target"
stage="target/stage/$name"
rm -rf "$stage"
mkdir -p "$stage"
cp "target/$target/release/git-intake" LICENSE "$stage/"
cargo run --locked --quiet -p xtask -- notices "$stage" "$target"
cargo run --locked --quiet -p xtask -- man "$stage/man/man1"

mkdir -p dist
archive="dist/$name.tar.gz"
# COPYFILE_DISABLE keeps macOS tar from adding AppleDouble files.
COPYFILE_DISABLE=1 tar -C "$stage" -czf "$archive" \
  git-intake LICENSE THIRD-PARTY-NOTICES.txt RUST-LIBRARY-COPYRIGHT.html man

# The layout check: the files above and nothing else, the main page and
# command pages among them, every section of the notices the target needs,
# and a binary that reports the version when the build host can run it.
listing=$(tar -tzf "$archive" | sed 's|/$||' | LC_ALL=C sort)
pages=$(cd "$stage" && find man -type f | LC_ALL=C sort)
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
    grep -Eq '^musl libc [0-9]' "$notices" || { echo "the notices lack musl's" >&2; exit 1; }
    grep -Eq '^Zig [0-9]' "$notices" || { echo "the notices lack Zig's" >&2; exit 1; }
    ;;
esac
grep -Fq 'Copyright notices for The Rust Standard Library' "$stage/RUST-LIBRARY-COPYRIGHT.html" \
  || { echo "RUST-LIBRARY-COPYRIGHT.html is not the standard library's notices" >&2; exit 1; }
case "$(uname -s)-$(uname -m)-$target" in
  Linux-x86_64-x86_64-unknown-linux-*|Linux-aarch64-aarch64-unknown-linux-*|Darwin-arm64-aarch64-apple-darwin|Darwin-x86_64-x86_64-apple-darwin)
    reported=$("$stage/git-intake" --version)
    [ "$reported" = "git-intake $version" ] || { echo "the binary reports '$reported', not version $version" >&2; exit 1; }
    ;;
esac

(cd dist && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256")
cat "$archive.sha256"
echo "$listing"
