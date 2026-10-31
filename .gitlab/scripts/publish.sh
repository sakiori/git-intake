#!/bin/sh
# Publishes a signed release: uploads the files in dist/ to the project's
# generic package registry, package git-intake, version vX.Y.Z (the tag), and
# then creates the GitLab Release of the tag with the changelog entry in
# release-notes.md and a link to every file. Each link's name is the file's
# name, which mise's GitLab backend matches, and its direct asset path is
# /binaries/<file>, which gives every file the permanent URL
# <project>/-/releases/vX.Y.Z/downloads/binaries/<file> that cargo-binstall
# uses. The release is created last, so it never lists a file that is not
# uploaded.
#
# A retried job uploads only the files the package lacks, and refuses a file
# the package holds with other content. Nothing is replaced: a release that
# exists already is left alone, and the registry refuses a second file of the
# same name once duplicates are disabled (RELEASING.md). A job retried after
# the release was created succeeds when the release links exactly the files
# it would have linked, and fails otherwise. Every response of the API goes
# to a file before it is parsed, so that a failed request stops the job
# instead of reading as an empty answer.
#
# usage: .gitlab/scripts/publish.sh   (in a tag pipeline, with jq on the PATH)
set -eu

tag=${CI_COMMIT_TAG:?not a tag pipeline}
: "${CI_API_V4_URL:?}" "${CI_PROJECT_ID:?}" "${CI_PROJECT_URL:?}" "${CI_JOB_TOKEN:?}" "${CI_COMMIT_SHA:?}"

project="$CI_API_V4_URL/projects/$CI_PROJECT_ID"
package="$project/packages/generic/git-intake/$tag"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Writes the response to <file>; on an HTTP error, shows it and fails.
api() { # <file> <curl arguments>...
  out=$1
  shift
  curl --fail-with-body --silent --show-error --header "JOB-TOKEN: $CI_JOB_TOKEN" --output "$out" "$@" || {
    status=$?
    cat "$out" >&2 2>/dev/null || true
    echo >&2
    return "$status"
  }
}

[ -s release-notes.md ] || { echo "release-notes.md is missing" >&2; exit 1; }
cd dist
set -- git-intake-"$tag"-*.tar.gz
if [ "$#" != 4 ] || [ ! -e "$1" ]; then
  echo "dist/ does not hold the four archives of $tag" >&2
  exit 1
fi
files=""
for archive in "$@"; do
  files="$files $archive $archive.sha256 $archive.sigstore.json"
done
files="$files SHA256SUMS SHA256SUMS.sigstore.json"
for file in $files; do
  [ -e "$file" ] || { echo "dist/ lacks $file" >&2; exit 1; }
done

# The files the package already holds, by name, with their SHA-256.
api "$tmp/packages.json" --get "$project/packages" --data package_type=generic \
  --data package_name=git-intake --data-urlencode "package_version=$tag"
id=$(jq -r --arg tag "$tag" '[.[] | select(.name == "git-intake" and .version == $tag)] | first | .id // empty' \
  "$tmp/packages.json")
held='[]'
if [ -n "$id" ]; then
  api "$tmp/files.json" --get "$project/packages/$id/package_files" --data per_page=100
  held=$(jq -c '[.[] | {name: .file_name, sha256: .file_sha256}]' "$tmp/files.json")
fi

links='[]'
for file in $files; do
  sha256sum "$file" > "$tmp/sum"
  sum=$(cut -d' ' -f1 "$tmp/sum")
  stored=$(printf '%s' "$held" | jq -r --arg name "$file" '[.[] | select(.name == $name) | .sha256] | unique | join(" ")')
  if [ -z "$stored" ]; then
    api "$tmp/upload.json" --upload-file "$file" "$package/$file"
    cat "$tmp/upload.json"
    echo
  elif [ "$stored" != "$sum" ]; then
    echo "the package $tag holds $file with other content; nothing is replaced" >&2
    exit 1
  else
    echo "$file is uploaded already"
  fi
  case "$file" in
    *.tar.gz) type=package ;;
    *) type=other ;;
  esac
  # jq builds the JSON, so no name or URL can break its quoting.
  links=$(printf '%s' "$links" | jq -c --arg name "$file" --arg url "$package/$file" --arg type "$type" \
    '. + [{name: $name, url: $url, direct_asset_path: ("/binaries/" + $name), link_type: $type}]')
done
cd ..

# A release of the tag that exists already, created by an earlier run of this
# job, is left alone if it links exactly these files, and is an error
# otherwise.
status=$(curl --silent --show-error --header "JOB-TOKEN: $CI_JOB_TOKEN" --output "$tmp/release.json" \
  --write-out '%{http_code}' "$project/releases/$tag") \
  || { echo "cannot tell whether the release $tag exists" >&2; exit 1; }
case "$status" in
  404)
    jq -n --arg tag "$tag" --arg sha "$CI_COMMIT_SHA" --rawfile notes release-notes.md --argjson links "$links" \
      '{tag_name: $tag, name: $tag, ref: $sha, description: $notes, assets: {links: $links}}' > "$tmp/release.json"
    api "$tmp/created.json" --request POST --header 'Content-Type: application/json' \
      --data-binary "@$tmp/release.json" "$project/releases"
    echo "released $tag"
    ;;
  200)
    # Every link's name, URL and type; its direct asset URL too where the
    # response carries one, which the API documents for the release links'
    # own endpoints.
    expected=$(printf '%s' "$links" | jq -c '[.[] | {name, url, link_type}] | sort_by(.name)')
    present=$(jq -c --arg base "$CI_PROJECT_URL/-/releases/$tag/downloads" --argjson links "$links" '
      [(.assets.links // [])[] | . as $link
        | if has("direct_asset_url") and $link.direct_asset_url != null
          then ([$links[] | select(.name == $link.name) | $base + .direct_asset_path] | first) as $direct
            | if $link.direct_asset_url == $direct then {name, url, link_type} else {name, url, link_type, direct_asset_url} end
          else {name, url, link_type} end]
      | sort_by(.name)' "$tmp/release.json")
    if [ "$present" != "$expected" ]; then
      echo "the release $tag exists with other links than its files'; nothing is changed. It links:" >&2
      jq -r '(.assets.links // [])[] | "\(.name) \(.url)"' "$tmp/release.json" >&2
      exit 1
    fi
    echo "the release $tag exists already with exactly these links"
    ;;
  *)
    echo "cannot tell whether the release $tag exists (HTTP $status)" >&2
    exit 1
    ;;
esac
printf '%s' "$links" | jq -r '.[].name'
