#!/usr/bin/env bash
# Fetches real DocBook documents for roadmap step 2's gate ("offline
# DocBook-style inputs parse correctly"), at pinned commits, into
# tests/docbook/ (git-ignored), then records libxml2's view of each
# (tests/tree-dump.xsl) in the offline reference container, with Debian's
# DocBook catalog. tests/parse_corpus.rs compares oxslt's trees with these.
#
#   scripts/fetch-docbook.sh
#
# Sources:
# - shadow's man pages: DocBook 4.5, with external entities declared in the
#   internal subset (login.defs.d/*.xml)
# - Linux-PAM's man pages and guides: DocBook 4.x

set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
out=$here/tests/docbook
image=oxslt-reference

SHADOW=shadow-maint/shadow@763e75e12be2b389f527502aa66dbca8ae7c8610
PAM=linux-pam/linux-pam@9ff73a5a0eda2baa5c14d468a746b828da4202af

rm -rf "$out"; mkdir -p "$out"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fetch() { # repo@sha dir-in-repo dest-name
  local repo=${1%@*} sha=${1#*@}
  curl -sfL "https://codeload.github.com/$repo/tar.gz/$sha" -o "$tmp/src.tar.gz"
  mkdir -p "$out/$3"
  tar -xzf "$tmp/src.tar.gz" -C "$out/$3" --strip-components=2 "${repo#*/}-$sha/$2"
  echo "$repo $sha $2" > "$out/$3/SOURCE"
}
fetch "$SHADOW" man shadow
fetch "$PAM" doc pam
# Keep only what parsing needs.
find "$out" -type f ! -name '*.xml' ! -name '*.ent' ! -name '*.dtd' ! -name SOURCE -delete
find "$out" -type d -empty -delete
echo "fetched $(find "$out" -name '*.xml' | wc -l | tr -d ' ') XML files"

docker image inspect "$image" >/dev/null 2>&1 || docker build --platform linux/amd64 -t "$image" "$here/tests/reference"
dumps=$here/tests/tree-dumps/docbook
rm -rf "$dumps"; mkdir -p "$dumps"
echo "recording libxml2's trees in an offline, read-only container"
docker run --rm --platform linux/amd64 --network none --read-only --tmpfs /tmp \
  -v "$here/tests":/tests:ro -v "$dumps":/out "$image" sh -c '
  cd /tests/docbook
  find . -name "*.xml" | sort | while read -r f; do
    mkdir -p "/out/$(dirname "$f")"
    # Documents that aren'"'"'t well-formed on their own (fragments, .xml
    # templates) get no dump; parse_corpus.rs skips them.
    if timeout 30 /usr/bin/xsltproc --nonet /tests/tree-dump.xsl "$f" > "/out/$f.txt" 2>/dev/null; then :; else rm -f "/out/$f.txt"; fi
  done; true' 2>&1 | grep -v 'WARNING: The requested image'
echo "recorded $(find "$dumps" -name '*.txt' | wc -l | tr -d ' ') trees"
