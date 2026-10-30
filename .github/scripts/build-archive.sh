#!/bin/sh
# Builds the release binary of git-intake for one target and packs it as
# dist/git-intake-v<version>-<target>.tar.gz, with a SHA-256 checksum file
# next to it, <version> being the package version in Cargo.toml. The archive
# holds exactly:
#
#   git-intake                         the binary
#   LICENSE                            its license
#   THIRD-PARTY-NOTICES.txt            the notices of the crates it is built from
#   man/man1/git-intake.1              the manual pages
#   man/man1/git-intake-<command>.1
#
# The xtask package generates the notices, with cargo-about, and the manual
# pages on the build host. Cargo.toml's [package.metadata.binstall] describes
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
cargo run --locked --quiet -p xtask -- notices "$stage/THIRD-PARTY-NOTICES.txt"
cargo run --locked --quiet -p xtask -- man "$stage/man/man1"

mkdir -p dist
archive="dist/$name.tar.gz"
# COPYFILE_DISABLE keeps macOS tar from adding AppleDouble files.
COPYFILE_DISABLE=1 tar -C "$stage" -czf "$archive" git-intake LICENSE THIRD-PARTY-NOTICES.txt man

# The layout check: the files above and nothing else, the main page and
# command pages among them, the notices' heading, and a binary that reports
# the version when the build host can run it.
listing=$(tar -tzf "$archive" | sed 's|/$||' | LC_ALL=C sort)
pages=$(cd "$stage" && find man -type f | LC_ALL=C sort)
expected=$(printf '%s\n' git-intake LICENSE THIRD-PARTY-NOTICES.txt man man/man1 "$pages" | LC_ALL=C sort)
if [ "$listing" != "$expected" ]; then
  echo "$archive does not have the release layout; it holds:" >&2
  echo "$listing" >&2
  exit 1
fi
for page in man/man1/git-intake.1 man/man1/git-intake-start.1 man/man1/git-intake-sync.1; do
  printf '%s\n' "$pages" | grep -Fqx "$page" || { echo "$archive lacks $page" >&2; exit 1; }
done
grep -Fq 'Third-party notices of git-intake' "$stage/THIRD-PARTY-NOTICES.txt" \
  || { echo "$archive holds no third-party notices" >&2; exit 1; }
case "$(uname -s)-$(uname -m)-$target" in
  Linux-x86_64-x86_64-unknown-linux-*|Linux-aarch64-aarch64-unknown-linux-*|Darwin-arm64-aarch64-apple-darwin|Darwin-x86_64-x86_64-apple-darwin)
    reported=$("$stage/git-intake" --version)
    [ "$reported" = "git-intake $version" ] || { echo "the binary reports '$reported', not version $version" >&2; exit 1; }
    ;;
esac

(cd dist && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256")
cat "$archive.sha256"
echo "$listing"
