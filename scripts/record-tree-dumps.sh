#!/usr/bin/env bash
# Records how libxml2 sees every XML file in the fetched corpus: runs
# tests/tree-dump.xsl with xsltproc on each file, inside the offline,
# read-only reference container, writing tests/tree-dumps/<path>.txt
# (git-ignored). tests/parse_corpus.rs compares oxslt's tree with these.
#
#   scripts/record-tree-dumps.sh
#
# Run it after scripts/fetch-conformance.sh and scripts/fetch-real-stylesheets.sh.

set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
image=oxslt-reference
out=$here/tests/tree-dumps
rm -rf "$out"; mkdir -p "$out"

docker image inspect "$image" >/dev/null 2>&1 || docker build --platform linux/amd64 -t "$image" "$here/tests/reference"

echo "recording tree dumps in an offline, read-only container"
docker run --rm --platform linux/amd64 --network none --read-only --tmpfs /tmp \
  -v "$here/tests":/tests:ro -v "$out":/out "$image" sh -c '
  cd /tests
  n=0
  find conformance/xalan real cases -type f \( -name "*.xml" -o -name "*.xsl" \) 2>/dev/null | sort | while read -r f; do
    mkdir -p "/out/$(dirname "$f")"
    # --nonet: never fetch DTDs. Without --novalid, libxml2 still loads local
    # DTDs, as xsltproc does by default; files where that matters show up as
    # differences.
    if timeout 30 /usr/bin/xsltproc --nonet /tests/tree-dump.xsl "$f" > "/out/$f.txt" 2>/dev/null; then :; else rm -f "/out/$f.txt"; fi
    n=$((n+1)); [ $((n % 500)) -eq 0 ] && echo "  $n done"
  done; true' 2>&1 | grep -v 'WARNING: The requested image'
echo "recorded $(find "$out" -name '*.txt' | wc -l | tr -d ' ') tree dumps"
