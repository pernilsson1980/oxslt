#!/usr/bin/env bash
# Differential testing of --html: the trees oxslt's HTML parser builds,
# compared with xsltproc's, in the offline oxslt-dropin image.
#
#   scripts/html-diff.sh corpus            HTML files from the image's docs and package sources
#   scripts/html-diff.sh fuzz [N] [SEED]   N generated, mostly broken documents (tests/html-diff/gen.py)
#
# Results in target/html-diff/out (diffs/<n>: the file and the first lines
# of the tree diff). Needs docker and the oxslt-dropin image (scripts/dropin.sh).
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
work=$here/target/html-diff
platform=linux/$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')
mkdir -p "$work/bin"
rm -rf "$work/out" && mkdir -p "$work/out"
docker run --rm --platform "$platform" --network none \
  -v "$here":/src:ro -v "${CARGO_HOME:-$HOME/.cargo}/registry":/usr/local/cargo/registry:ro \
  -v oxslt-bench-target:/build -v "$work/bin":/bin-out \
  oxslt-bench bash -c 'cd /src && cargo build --release --offline --quiet --target-dir /build && cp /build/release/oxslt /bin-out/'
cp "$here"/tests/html-diff/*.sh "$here"/tests/html-diff/gen.py "$here"/tests/html-dump.xsl "$work/"
docker run --rm --platform "$platform" --network none --read-only --tmpfs /tmp:exec,size=4g \
  -v "$work":/s:ro -v "$work/out":/out -v "$work/bin/oxslt":/opt/oxslt/oxslt:ro \
  oxslt-dropin bash /s/run.sh "$@"
