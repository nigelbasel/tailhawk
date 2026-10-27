#!/usr/bin/env bash
# Publishes one GitHub Release for a green master build: the two executables, a zip of each,
# and their SHA-256 sums, under the tag `v<version>` where <version> is the stamp the build legs
# read back out of the exe. Run by `.github/workflows/ci.yml`'s `release` job and by nobody else.
#
# Inputs, all environment:
#   VERSION            the build's version stamp, e.g. 2026.10.27.22732
#   DIST               a directory holding x64/tailhawk.exe and arm64/tailhawk.exe
#   GITHUB_REPOSITORY  owner/repo
#   GITHUB_SHA         the commit the build is of
#   GITHUB_TOKEN       the workflow's own token, `contents: write`
#
# `--dry-run` does everything but talk to GitHub, and prints what it would have said. That is the
# only way this script can be exercised on a developer machine, so it is kept honest: the dry run
# builds the real assets into DIST and prints the real request body.
#
# `curl` against the REST API, never `gh`: the project reads its CI the same way and for the same
# reason, and a release needs nothing a token and two endpoints do not give.
set -euo pipefail

dry=0
[ "${1:-}" = "--dry-run" ] && dry=1

: "${VERSION:?VERSION is required}"
: "${DIST:?DIST is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
: "${GITHUB_SHA:?GITHUB_SHA is required}"
if [ "$dry" -eq 0 ]; then
  : "${GITHUB_TOKEN:?GITHUB_TOKEN is required}"
fi

tag="v$VERSION"
short="${GITHUB_SHA:0:7}"
out="$DIST/release"
rm -rf "$out"
mkdir -p "$out"

# The assets, named for what a reader types: the architecture and nothing else. The workflow's
# artefacts say `-unsigned`; the release body says it instead, once, where it is read.
for arch in x64 arm64; do
  exe="$DIST/$arch/tailhawk.exe"
  [ -f "$exe" ] || { echo "missing $exe" >&2; exit 1; }
  cp "$exe" "$out/tailhawk-$arch.exe"
  # python's zipfile rather than `zip`: the runner has both, a developer's Git Bash has no `zip`
  # and usually a python3 on PATH, and the dry run is only worth having if it runs where a
  # developer is. Deflated, because the module's command line stores and a stored zip of an exe
  # is the exe with a header on it.
  (cd "$out" && python3 -c "
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], 'w', zipfile.ZIP_DEFLATED) as z:
    z.write(sys.argv[2])
" "tailhawk-$arch.zip" "tailhawk-$arch.exe")
done
(cd "$out" && sha256sum tailhawk-x64.exe tailhawk-arm64.exe tailhawk-x64.zip tailhawk-arm64.zip > SHA256SUMS.txt)

body="Tailhawk $VERSION, built from $short on master by CI after every check passed.

Download the newest build at a fixed address:
https://github.com/$GITHUB_REPOSITORY/releases/latest/download/tailhawk-x64.exe
(ARM64: https://github.com/$GITHUB_REPOSITORY/releases/latest/download/tailhawk-arm64.exe)

The executable is unsigned, so SmartScreen may ask once on first run; the SHA-256 of every file
is in SHA256SUMS.txt. Copy and run: no installer and no runtime to install."

# **Created as a draft, filled, then published — in that order.** A release published first and
# filled second is *latest* for the seconds before its assets exist, and for ever if an upload
# fails: the tag is pinned to the commit, so a re-run cannot create it again. A draft has no tag
# and is not latest, so a failed run leaves a draft to delete, not a broken download address.
request=$(python3 - "$tag" "$GITHUB_SHA" "$VERSION" "$body" <<'PY'
import json, sys
tag, sha, version, body = sys.argv[1:5]
print(json.dumps({
    "tag_name": tag,
    "target_commitish": sha,
    "name": f"Tailhawk {version}",
    "body": body,
    "draft": True,
    "prerelease": False,
}))
PY
)

echo "release $tag at $short with:"
(cd "$out" && ls -l tailhawk-x64.exe tailhawk-arm64.exe tailhawk-x64.zip tailhawk-arm64.zip SHA256SUMS.txt)
cat "$out/SHA256SUMS.txt"

if [ "$dry" -eq 1 ]; then
  echo "--- dry run: would POST to https://api.github.com/repos/$GITHUB_REPOSITORY/releases, then publish:"
  echo "$request"
  exit 0
fi

api="https://api.github.com/repos/$GITHUB_REPOSITORY/releases"
auth=(-H "Authorization: Bearer $GITHUB_TOKEN" -H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28")

# `id` and `draft` of the release already carrying this tag, if any: a published one means a
# re-run of a workflow that already did its job; a draft is a failed run's leftover.
field() { python3 -c 'import json,sys
try:
    print(json.load(sys.stdin).get(sys.argv[1], ""))
except Exception:
    print("")' "$1"; }
existing=$(curl -sS "$api?per_page=50" "${auth[@]}" | python3 -c '
import json, sys
tag = sys.argv[1]
try:
    for r in json.load(sys.stdin):
        if r.get("tag_name") == tag:
            print(f"{r.get(\"id\")} {str(r.get(\"draft\")).lower()}")
            break
except Exception:
    pass' "$tag")
if [ -n "$existing" ]; then
  existing_id=${existing%% *}
  existing_draft=${existing##* }
  if [ "$existing_draft" = "false" ]; then
    echo "already released: https://github.com/$GITHUB_REPOSITORY/releases/tag/$tag"
    exit 0
  fi
  echo "deleting the draft $existing_id a previous run left behind"
  code=$(curl -sS -o /dev/null -w '%{http_code}' -X DELETE "$api/$existing_id" "${auth[@]}")
  [ "$code" = "204" ] || { echo "could not delete draft $existing_id: HTTP $code" >&2; exit 1; }
fi

code=$(curl -sS -o /tmp/create.json -w '%{http_code}' -X POST "$api" "${auth[@]}" -d "$request")
release_id=$(field id < /tmp/create.json)
if [ "$code" != "201" ] || [ -z "$release_id" ]; then
  echo "the draft release was not created: HTTP $code" >&2
  cat /tmp/create.json >&2
  exit 1
fi
echo "created draft release $release_id"

for asset in tailhawk-x64.exe tailhawk-arm64.exe tailhawk-x64.zip tailhawk-arm64.zip SHA256SUMS.txt; do
  type="application/octet-stream"
  [ "$asset" = "SHA256SUMS.txt" ] && type="text/plain"
  code=$(curl -sS -o /tmp/upload.json -w '%{http_code}' -X POST \
    "https://uploads.github.com/repos/$GITHUB_REPOSITORY/releases/$release_id/assets?name=$asset" \
    "${auth[@]}" -H "Content-Type: $type" --data-binary "@$out/$asset")
  if [ "$code" != "201" ]; then
    echo "uploading $asset failed with HTTP $code; the draft $release_id is left for the next run to replace" >&2
    cat /tmp/upload.json >&2
    exit 1
  fi
  echo "uploaded $asset"
done

code=$(curl -sS -o /tmp/publish.json -w '%{http_code}' -X PATCH "$api/$release_id" "${auth[@]}" \
  -d '{"draft": false, "make_latest": "true"}')
if [ "$code" != "200" ]; then
  echo "publishing release $release_id failed with HTTP $code; it is still a draft" >&2
  cat /tmp/publish.json >&2
  exit 1
fi
echo "published https://github.com/$GITHUB_REPOSITORY/releases/tag/$tag"
