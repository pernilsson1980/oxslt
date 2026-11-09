#!/usr/bin/env bash
# Drop-in testing: builds Debian packages with oxslt and xsltproc side by side.
#
#   scripts/dropin.sh [package ...]    (default: tests/dropin/packages.txt)
#
# 1. Builds a Linux oxslt in the oxslt-bench container.
# 2. Builds the oxslt-dropin image (network: build dependencies and sources).
# 3. Builds the packages offline, with a read-only root, through the comparing
#    wrapper (tests/dropin/xsltproc-wrapper.sh).
#
# Results in target/dropin/out: builds.tsv (one line per package), calls.tsv
# (one line per xsltproc call), diffs/<package>/<call>/ for each call where
# oxslt differed, logs/<package>.log. Needs docker (Colima).

set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
dropin=$here/target/dropin
platform=linux/$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')

mkdir -p "$dropin/bin" "$dropin/out"

# 1. oxslt for Linux, built from the working tree.
docker image inspect oxslt-bench >/dev/null 2>&1 \
  || docker build --platform "$platform" -t oxslt-bench "$here/tests/bench"
docker run --rm --platform "$platform" --network none \
  -v "$here":/src:ro -v "${CARGO_HOME:-$HOME/.cargo}/registry":/usr/local/cargo/registry:ro \
  -v oxslt-bench-target:/build -v "$dropin/bin":/bin-out \
  oxslt-bench bash -c 'cd /src && cargo build --release --offline --quiet --target-dir /build && cp /build/release/oxslt /bin-out/'

# 2. The image, rebuilt only when tests/dropin changes.
docker build --platform "$platform" -t oxslt-dropin "$here/tests/dropin"

# 3. The builds: offline, read-only root, fresh results. The build area is a
#    Docker volume, not a directory on the host: macOS's file system rejects
#    some file names in source packages and skews timestamps, which breaks
#    meson and automake.
rm -rf "$dropin/out" && mkdir -p "$dropin/out"
docker run --rm --platform "$platform" --network none --read-only \
  --tmpfs /tmp:exec,size=4g \
  -v "$dropin/bin/oxslt":/opt/oxslt/oxslt:ro \
  -v oxslt-dropin-work:/work -v "$dropin/out":/out \
  oxslt-dropin /dropin/run.sh "$@"

echo "results: $dropin/out (builds.tsv, calls.tsv, diffs/)"
