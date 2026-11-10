#!/bin/bash
# Runs inside the drop-in image: unpacks and builds each package with the
# comparing xsltproc wrapper. /work is the build area, /out the results.
#   run.sh [package ...]   (default: every package in packages.txt)
set -u
export DROPIN_OUT=/out
export HOME=/tmp
# Serial builds: the wrapper finds the files a call wrote by modification
# time, which another job running at the same time would confuse.
export DEB_BUILD_OPTIONS="nocheck parallel=1"
mkdir -p /out/logs
pkgs=("$@")
[ ${#pkgs[@]} = 0 ] && mapfile -t pkgs < <(grep -v '^#' /dropin/packages.txt)
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
done
