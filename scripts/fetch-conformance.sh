#!/usr/bin/env bash
# Fetches the Xalan XSLT 1.0 conformance tests (Apache-2.0) at a pinned
# commit, classifies them as PoC or later, and records xsltproc's output for
# each inside an offline, read-only container.
#
#   scripts/fetch-conformance.sh
#
# Result, all git-ignored except passing.txt:
#   tests/conformance/xalan/          the tests (apache/xalan-test tests/conf)
#   tests/conformance/manifest.tsv    id, category, stylesheet, input, scope, reasons
#   tests/conformance/expected/<id>/  xsltproc's stdout, exit code and stderr
#
# xsltproc is the judge, not Xalan's own gold files: oxslt's goal is xsltproc
# compatibility. Needs curl, python3 and docker (oxslt-reference image).

set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
conf=$here/tests/conformance
sha=5120be9b3704a724ff0205b750d3cb409b62869b   # apache/xalan-test, 2025-08-08
image=oxslt-reference

rm -rf "$conf/xalan" "$conf/expected" "$conf/manifest.tsv"
mkdir -p "$conf/xalan" "$conf/expected"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "downloading apache/xalan-test@$sha"
curl -sfL "https://codeload.github.com/apache/xalan-test/tar.gz/$sha" -o "$tmp/xalan-test.tar.gz"
tar -xzf "$tmp/xalan-test.tar.gz" -C "$conf/xalan" --strip-components=3 "xalan-test-$sha/tests/conf"
echo "apache/xalan-test $sha tests/conf (Apache-2.0)" > "$conf/xalan/SOURCE"

python3 -I "$here/scripts/classify-conformance.py" "$conf/xalan" "$conf/manifest.tsv"

docker image inspect "$image" >/dev/null 2>&1 || docker build --platform linux/amd64 -t "$image" "$here/tests/reference"

echo "recording xsltproc output in an offline, read-only container (takes a few minutes)"
docker run --rm --platform linux/amd64 --network none --read-only --tmpfs /tmp \
  -v "$conf/xalan":/suite:ro -v "$conf/manifest.tsv":/manifest.tsv:ro -v "$conf/expected":/out \
  "$image" sh -c '
  n=0
  grep -v "^#" /manifest.tsv | while IFS="	" read -r id cat xsl xml scope reasons; do
    mkdir -p "/out/$id"
    cd "/suite/$cat"
    timeout 30 /usr/bin/xsltproc "$xsl" "$xml" > "/out/$id/stdout" 2> "/out/$id/stderr"
    echo $? > "/out/$id/exit"
    n=$((n+1)); [ $((n % 200)) -eq 0 ] && echo "  $n done"
  done; true' 2>&1 | grep -v 'WARNING: The requested image'

echo "exit codes recorded (count, code):"
cat "$conf"/expected/*/*/exit | sort | uniq -c | sort -rn
