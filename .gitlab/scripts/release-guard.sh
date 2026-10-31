#!/bin/sh
# Checks, before anything of a release is built, that the pipeline's tag is
# one to release: a protected tag vX.Y.Z naming this pipeline's commit, a
# commit on the default branch, whose Cargo.toml holds version X.Y.Z and whose
# CHANGELOG.md has its entry; and that no GitLab Release of the tag exists, so
# that a published release is never changed. It writes the release notes, the
# changelog entry, to release-notes.md for the publish job.
#
# usage: .gitlab/scripts/release-guard.sh   (in a tag pipeline)
set -eu

tag=${CI_COMMIT_TAG:?not a tag pipeline}
: "${CI_COMMIT_SHA:?}" "${CI_DEFAULT_BRANCH:?}" "${CI_API_V4_URL:?}" "${CI_PROJECT_ID:?}" "${CI_JOB_TOKEN:?}"

if ! printf '%s\n' "$tag" | grep -Eqx 'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)'; then
  echo "$tag is not a release tag vX.Y.Z" >&2
  exit 1
fi
if [ "${CI_COMMIT_REF_PROTECTED:-}" != true ]; then
  echo "$tag is not a protected tag; only a protected tag releases" >&2
  exit 1
fi

git fetch --quiet --no-tags origin "+refs/tags/$tag:refs/tags/$tag" \
  "+refs/heads/$CI_DEFAULT_BRANCH:refs/remotes/origin/$CI_DEFAULT_BRANCH"
tagged=$(git rev-parse --verify --quiet "refs/tags/$tag^{commit}" || true)
if [ "$tagged" != "$CI_COMMIT_SHA" ]; then
  echo "$tag points at ${tagged:-nothing}, not at this pipeline's commit $CI_COMMIT_SHA" >&2
  exit 1
fi
# Only a commit on the default branch is released: it went through review and
# the merge request pipeline.
if ! git merge-base --is-ancestor "$CI_COMMIT_SHA" "refs/remotes/origin/$CI_DEFAULT_BRANCH"; then
  echo "the commit of $tag is not on $CI_DEFAULT_BRANCH" >&2
  exit 1
fi

version=${tag#v}
committed=$(sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' Cargo.toml)
if [ "$committed" != "$version" ]; then
  echo "Cargo.toml at $tag holds version $committed, not $version" >&2
  exit 1
fi

# The changelog entry is the section under the heading that names the version.
awk -v v="$version" '
  /^## / { if (found) exit; if (index($0, "[" v "]") || $2 == v) { found = 1; next } }
  found { print }
' CHANGELOG.md > release-notes.md
if ! grep -q '[^[:space:]]' release-notes.md; then
  echo "CHANGELOG.md has no entry for $version" >&2
  exit 1
fi

status=$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' \
  --header "JOB-TOKEN: $CI_JOB_TOKEN" "$CI_API_V4_URL/projects/$CI_PROJECT_ID/releases/$tag")
case "$status" in
  404) ;;
  200) echo "the release $tag exists already; a published release is never changed" >&2; exit 1 ;;
  *) echo "cannot tell whether the release $tag exists (HTTP $status)" >&2; exit 1 ;;
esac
echo "releasing $tag from $CI_COMMIT_SHA"
