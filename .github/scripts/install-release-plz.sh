#!/bin/sh
# Installs the pinned release-plz binary into <dir> after verifying the
# SHA-256 checksum of its release archive, and adds <dir> to the job's PATH.
# release-plz runs in the jobs that hold the GitHub App's token and the OIDC
# token crates.io accepts for publishing, so nothing is installed unverified:
# there is no fallback to another source or to a build. A new release-plz
# version needs both values below; the checksum is the archive's SHA-256, as
# the release lists it and as a download of the archive shows it.
#
# usage: .github/scripts/install-release-plz.sh <dir>
set -eu

version=0.3.174
sha256=8240e953608d40e709d042e9ac0e206d518825de3d88a3da9ebeaf2a3ff57ea9
asset=release-plz-x86_64-unknown-linux-musl.tar.gz

dir=${1:?usage: install-release-plz.sh <dir>}
mkdir -p "$dir"
archive="$dir/$asset"
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 --output "$archive" \
  "https://github.com/release-plz/release-plz/releases/download/release-plz-v$version/$asset"
echo "$sha256  $archive" | sha256sum --check --strict -
tar -xzf "$archive" -C "$dir" release-plz
rm "$archive"
reported=$("$dir/release-plz" --version)
[ "$reported" = "release-plz $version" ] || { echo "the archive holds '$reported', not release-plz $version" >&2; exit 1; }
if [ -n "${GITHUB_PATH:-}" ]; then
  echo "$dir" >> "$GITHUB_PATH"
fi
echo "installed $reported"
