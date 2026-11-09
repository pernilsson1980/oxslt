#!/usr/bin/env bash
# Benchmarks oxslt against Debian's xsltproc (libxslt 1.1.35), both native
# code in the same container on the same machine, with no network.
#
#   scripts/bench.sh
#
# Needs docker (Colima), python3, and the fetched real stylesheets
# (scripts/fetch-real-stylesheets.sh). Results go to target/bench-results/:
# summary.txt (identical output? peak memory), json/ (hyperfine timings) and
# output/ (both tools' output for every workload).

set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
data=$here/target/bench-data
results=$here/target/bench-results
image=oxslt-bench

[[ -d $here/tests/real ]] || { echo "run scripts/fetch-real-stylesheets.sh first" >&2; exit 1; }
[[ -f $data/records-200k.xml ]] || python3 -I "$here/scripts/bench-data.py" "$data"
# A larger real input for Wireshark's Values.xsl, from the same pinned commit.
[[ -f $data/FIX50SP2.xml ]] || curl -sfL \
  https://raw.githubusercontent.com/wireshark/wireshark/1adccc3f1befa4b13e877827b7e765703d5c2ce7/fix/FIX50SP2.xml \
  -o "$data/FIX50SP2.xml"

# The host's architecture, so neither tool runs under emulation.
platform=linux/$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')
docker image inspect "$image" >/dev/null 2>&1 || docker build --platform "$platform" -t "$image" "$here/tests/bench"

rm -rf "$results"; mkdir -p "$results"
docker run --rm --platform "$platform" --network none \
  -v "$here":/src:ro -v "$data":/data:ro -v "$results":/out \
  -v "${CARGO_HOME:-$HOME/.cargo}/registry":/usr/local/cargo/registry:ro \
  -v oxslt-bench-target:/build \
  "$image" bash /src/tests/bench/run.sh
echo "results: $results/summary.md"
