#!/usr/bin/env bash
# Fetches real-world stylesheets at pinned commits into tests/real/ and
# records xsltproc's output for each, inside an offline, read-only container.
#
#   scripts/fetch-real-stylesheets.sh
#
# The files are GPL/LGPL-licensed, so tests/real/ is git-ignored and never
# committed. Needs curl and docker; builds the oxslt-reference image if missing.
# They were chosen as small-to-medium real build stylesheets from Debian
# packages, each run as its project's build runs it.

set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
real=$here/tests/real
image=oxslt-reference

DARKTABLE=darktable-org/darktable@65eda86705b11bfa67be0cbe9c7f08b76f4cfb42
GNUCASH=Gnucash/gnucash@8aacd82fa8266e0724437848b0aa069ac7b4e257
FLUIDSYNTH=FluidSynth/fluidsynth@8210cb1bdc42718db5d68c2bc5aaea99ed434472
WIRESHARK=wireshark/wireshark@1adccc3f1befa4b13e877827b7e765703d5c2ce7
GIMP=GNOME/gimp@20e6eefb4d2c1b031495cc9eed11809ee396ef06

get() { # repo@sha path dest
  local repo=${1%@*} sha=${1#*@}
  curl -sfL "https://raw.githubusercontent.com/$repo/$sha/$2" -o "$3" || { echo "failed: $repo $2" >&2; exit 1; }
}

# case <name> <repo@sha> <stylesheet> <input> [extra file]... -- [xsltproc args]...
case_() {
  local name=$1 src=$2 style=$3 input=$4; shift 4
  local dir=$real/$name; mkdir -p "$dir/expected"
  get "$src" "$style" "$dir/style.xsl"
  get "$src" "$input" "$dir/in.xml"
  while [[ $# -gt 0 && $1 != -- ]]; do get "$src" "$1" "$dir/$(basename "$1")"; shift; done
  [[ ${1:-} == -- ]] && shift
  printf '%s\n' "$@" > "$dir/args"
  [[ -s $dir/args && $(cat "$dir/args") != "" ]] || rm -f "$dir/args"
  echo "${src%@*} ${src#*@} $style $input" > "$dir/SOURCE"
}

rm -rf "$real"; mkdir -p "$real"
echo "fetching into $real"

# First milestone.
case_ gnucash-iso      "$GNUCASH"    libgnucash/engine/iso-currencies-to-c.xsl libgnucash/engine/iso-4217-currencies.xml
# The PoC's real stylesheet. The input is a .in file with one build-time placeholder.
case_ darktable-prefs  "$DARKTABLE"  tools/generate_prefs.xsl data/darktableconfig.xml.in data/darktableconfig.dtd -- --nonet --stringparam HAVE_OPENCL 1
case_ darktable-conf   "$DARKTABLE"  tools/generate_darktablerc_conf.xsl data/darktableconfig.xml.in data/darktableconfig.dtd -- --nonet --stringparam HAVE_OPENCL 1
for d in darktable-prefs darktable-conf; do
  sed -i.bak 's/@DARKTABLECONFIG_IOP_ENTRIES@//' "$real/$d/in.xml" && rm "$real/$d/in.xml.bak"
done
case_ fluidsynth-html  "$FLUIDSYNTH" doc/fluidsettings.xsl doc/fluidsettings.xml -- --nonet
case_ wireshark-values "$WIRESHARK"  fix/Values.xsl fix/FIX44.xml -- --nonet
case_ wireshark-hfdecl "$WIRESHARK"  fix/hfDecl.xsl fix/FIX44.xml -- --nonet
case_ wireshark-hflist "$WIRESHARK"  fix/hfList.xsl fix/FIX44.xml -- --nonet
# authors.xml declares its own default namespace; its DTD's #FIXED xmlns only repeats it.
case_ gimp-authors     "$GIMP"       authors.xsl authors.xml authors.dtd -- --nonet

docker image inspect "$image" >/dev/null 2>&1 || docker build --platform linux/amd64 -t "$image" "$here/tests/reference"

echo "recording xsltproc output in an offline, read-only container"
docker run --rm --platform linux/amd64 --network none --read-only --tmpfs /tmp \
  -v "$real":/real:ro -v "$real":/out "$image" sh -c '
  for dir in /real/*/; do
    name=$(basename "$dir")
    cd "$dir"
    if [ -f args ]; then set -- $(cat args); else set --; fi
    /usr/bin/xsltproc "$@" style.xsl in.xml > /out/$name/expected/stdout 2>/dev/null
    echo $? > /out/$name/expected/exit
    printf "%-18s exit %s  %8d bytes\n" "$name" "$(cat /out/$name/expected/exit)" "$(wc -c < /out/$name/expected/stdout)"
  done' 2>&1 | grep -v 'WARNING: The requested image'
