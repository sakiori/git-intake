#!/bin/sh
# Signs a release's files in dist/ with cosign, keyless: the job's GitLab ID
# token (SIGSTORE_ID_TOKEN, audience sigstore) obtains a short-lived Sigstore
# certificate that names this project's CI configuration and the tag, and
# each signature is recorded in Sigstore's public transparency log. It writes
# SHA256SUMS over the archives, then <file>.sigstore.json, the Sigstore
# bundle, for every archive and for SHA256SUMS, and verifies every bundle
# against the identity users verify with, as the README shows.
#
# usage: .gitlab/scripts/sign.sh   (in a tag pipeline, with cosign on the PATH)
set -eu

tag=${CI_COMMIT_TAG:?not a tag pipeline}
: "${SIGSTORE_ID_TOKEN:?the job has no ID token for Sigstore}"
: "${CI_SERVER_URL:?}" "${CI_PROJECT_PATH:?}" "${CI_CONFIG_PATH:?}"

cd dist
set -- git-intake-"$tag"-*.tar.gz
[ "$#" = 4 ] && [ -e "$1" ] || { echo "dist/ does not hold the four archives of $tag" >&2; exit 1; }
for archive in "$@"; do
  [ -e "$archive.sha256" ] || { echo "$archive has no .sha256 next to it" >&2; exit 1; }
done
sha256sum --check --strict -- *.sha256
sha256sum -- "$@" > SHA256SUMS

identity="$CI_SERVER_URL/$CI_PROJECT_PATH//$CI_CONFIG_PATH@refs/tags/$tag"
for file in "$@" SHA256SUMS; do
  cosign sign-blob --yes --bundle "$file.sigstore.json" "$file"
  cosign verify-blob --bundle "$file.sigstore.json" \
    --certificate-identity "$identity" --certificate-oidc-issuer "$CI_SERVER_URL" "$file"
done
ls -l
