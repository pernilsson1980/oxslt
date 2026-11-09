#!/usr/bin/env bash
# Copies the reference container's DocBook XSL stylesheets (docbook-xsl
# 1.79.2+dfsg-2), DocBook DTDs and XML catalogs into tests/docbook-xsl/
# (git-ignored), with the catalogs' absolute paths rewritten to the copy.
# oxslt then resolves DocBook URLs to exactly the files the reference
# xsltproc uses:
#
#   scripts/fetch-docbook-xsl.sh
#   XML_CATALOG_FILES=$PWD/tests/docbook-xsl/etc/xml/catalog oxslt …
#
# tests/differential.rs sets that for cases whose env file names
# tests/docbook-xsl.

set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
out=$here/tests/docbook-xsl
image=oxslt-reference

docker image inspect "$image" >/dev/null 2>&1 || docker build --platform linux/amd64 -t "$image" "$here/tests/reference"
rm -rf "$out"; mkdir -p "$out"
docker run --rm --platform linux/amd64 --network none --read-only -v "$out":/out "$image" \
  sh -c 'mkdir -p /out/etc /out/usr/share && cp -rL /etc/xml /out/etc/ && cp -rL /usr/share/xml /out/usr/share/' \
  2>&1 | grep -v 'WARNING: The requested image' || true
# Point the catalogs at the copy.
grep -rlE '/(usr/share|etc)/xml' "$out/etc/xml" | while read -r f; do
  sed -i.bak -e "s#file:///usr/share/xml#file://$out/usr/share/xml#g" \
             -e "s#file:///etc/xml#file://$out/etc/xml#g" \
             -e "s#\"/usr/share/xml#\"$out/usr/share/xml#g" \
             -e "s#\"/etc/xml#\"$out/etc/xml#g" "$f"
  rm -f "$f.bak"
done
echo "copied $(du -sh "$out" | cut -f1) into tests/docbook-xsl"
