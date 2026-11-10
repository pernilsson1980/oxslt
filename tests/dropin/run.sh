#!/bin/bash
# Runs inside the drop-in image: unpacks and builds each package with the
# comparing xsltproc wrapper. /work is the build area, /out the results.
#   run.sh [package ...]   (default: every package in packages.txt)
#   run.sh --list packages-2.txt [package ...]
# Each package's build tree is deleted after its build; logs, results and
# differences stay in /out. DROPIN_KEEP=1 keeps the trees.
set -u
export DROPIN_OUT=/out
export HOME=/tmp
# Serial builds: the wrapper finds the files a call wrote by comparing
# snapshots, which another job running at the same time would confuse.
export DEB_BUILD_OPTIONS="nocheck parallel=1"
mkdir -p /out/logs
list=packages.txt
if [ "${1:-}" = --list ]; then list=$2; shift 2; fi
pkgs=("$@")
[ ${#pkgs[@]} = 0 ] && mapfile -t pkgs < <(grep -v '^#' "/dropin/$list")
for p in "${pkgs[@]}"; do
    if grep -qx "$p" /dropin/build-dep-failed /dropin/source-failed; then
        printf '%s\tskipped (dependencies or source unavailable)\n' "$p" >> /out/builds.tsv
        continue
    fi
    rm -rf "/work/$p"
    mkdir -p "/work/$p"
    dsc=$(ls /dropin/src/${p}_*.dsc | head -1)
    dpkg-source -x "$dsc" "/work/$p/src" > "/out/logs/$p.log" 2>&1
    start=$(date +%s)
    ( cd "/work/$p/src" && DROPIN_PKG="$p" dpkg-buildpackage -b -uc -us ) >> "/out/logs/$p.log" 2>&1
    rc=$?
    printf '%s\tbuild exit %d\t%ds\n' "$p" $rc $(( $(date +%s) - start )) >> /out/builds.tsv
    [ "${DROPIN_KEEP:-0}" = 1 ] || rm -rf "/work/$p"
done
