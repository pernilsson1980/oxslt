#!/usr/bin/env bash
# Roadmap step 3's gate: DocBook XSL's single-page HTML stylesheet
# (html/docbook.xsl, docbook-xsl 1.79.2) over every DocBook document in
# tests/docbook/ (scripts/fetch-docbook.sh). Records xsltproc's output and
# exit code in the offline, read-only container, into
# tests/docbook-html/<path>.html and .exit (git-ignored);
# tests/docbook_html.rs compares oxslt's with them.
#
#   scripts/fetch-docbook.sh && scripts/fetch-docbook-xsl.sh
#   scripts/record-docbook-html.sh

set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
out=$here/tests/docbook-html
image=oxslt-reference

docker image inspect "$image" >/dev/null 2>&1 || docker build --platform linux/amd64 -t "$image" "$here/tests/reference"
rm -rf "$out"; mkdir -p "$out"
docker run --rm --platform linux/amd64 --network none --read-only --tmpfs /tmp \
  -v "$here/tests/docbook":/docbook:ro -v "$out":/out "$image" sh -c '
  xsl=/usr/share/xml/docbook/stylesheet/docbook-xsl/html/docbook.xsl
  cd /docbook
  find . -name "*.xml" | sort | while read -r f; do
    mkdir -p "/out/$(dirname "$f")"
    (cd "$(dirname "$f")" && timeout 120 /usr/bin/xsltproc --nonet "$xsl" "$(basename "$f")" > "/out/$f.html" 2>/dev/null)
    echo $? > "/out/$f.exit"
  done' 2>&1 | grep -v 'WARNING: The requested image' || true
echo "recorded $(find "$out" -name '*.exit' | wc -l | tr -d ' ') results, $(grep -L '^0$' $(find "$out" -name '*.exit') | wc -l | tr -d ' ') with a non-zero exit"
