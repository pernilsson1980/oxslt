#!/usr/bin/env bash
# Runs inside the oxslt-bench container (see scripts/bench.sh). Builds oxslt,
# checks that oxslt and xsltproc give identical output on every workload,
# then measures wall time (hyperfine) and peak memory (GNU time) for both.
#
# Mounts: /src (the repo, read-only), /data (bench data), /build (cargo
# target dir), /out (results).

set -euo pipefail
cd /src
cargo build --release --offline --quiet --target-dir /build
OX=/build/release/oxslt
XP=/usr/bin/xsltproc
mkdir -p /out/json /out/output

# name | runs | stylesheet | input | extra args
workloads() {
  local real=/src/tests/real w=/src/tests/bench/workloads d=/data
  cat <<EOF
gnucash-iso|20|$real/gnucash-iso/style.xsl|$real/gnucash-iso/in.xml|
gimp-authors|20|$real/gimp-authors/style.xsl|$real/gimp-authors/in.xml|--nonet
darktable-prefs|20|$real/darktable-prefs/style.xsl|$real/darktable-prefs/in.xml|--nonet --stringparam HAVE_OPENCL 1
darktable-conf|20|$real/darktable-conf/style.xsl|$real/darktable-conf/in.xml|--nonet --stringparam HAVE_OPENCL 1
fluidsynth-html|20|$real/fluidsynth-html/style.xsl|$real/fluidsynth-html/in.xml|--nonet
wireshark-values|20|$real/wireshark-values/style.xsl|$real/wireshark-values/in.xml|--nonet
wireshark-hfdecl|20|$real/wireshark-hfdecl/style.xsl|$real/wireshark-hfdecl/in.xml|--nonet
wireshark-hflist|20|$real/wireshark-hflist/style.xsl|$real/wireshark-hflist/in.xml|--nonet
wireshark-values-FIX50SP2|10|$real/wireshark-values/style.xsl|$d/FIX50SP2.xml|--nonet
identity-27MB|5|$w/identity.xsl|$d/records-200k.xml|
html-report-27MB|5|$w/html-report.xsl|$d/records-200k.xml|
sort-27MB|5|$w/sort.xsl|$d/records-200k.xml|
recursion-27MB|5|$w/recursion.xsl|$d/records-200k.xml|
grouping-5k|5|$w/grouping.xsl|$d/records-5k.xml|
docbook-chunked|3|/src/tests/docbook-xsl/usr/share/xml/docbook/stylesheet/docbook-xsl/xhtml/chunk.xsl|$d/book.xml|-o /tmp/chunks-TOOL/
EOF
}

peak_kb() { # command... -> peak resident set size in KB
  /usr/bin/time -f '%M' -o /tmp/rss "$@" > /dev/null 2>&1 || true
  cat /tmp/rss
}

echo "name|identical|xsltproc_kb|oxslt_kb" > /out/summary.txt
# A workload writing files (`-o DIR/`, DocBook's chunker) names its
# directory with TOOL, replaced by each tool's name; the directories are
# compared too.
while IFS='|' read -r name runs style input extra; do
  xp_extra=${extra//TOOL/xsltproc} ox_extra=${extra//TOOL/oxslt}
  rm -rf /tmp/chunks-xsltproc /tmp/chunks-oxslt
  # shellcheck disable=SC2086
  "$XP" $xp_extra "$style" "$input" > "/out/output/$name.xsltproc" 2>/dev/null || true
  # shellcheck disable=SC2086
  "$OX" $ox_extra "$style" "$input" > "/out/output/$name.oxslt" 2>/dev/null || true
  same=yes
  cmp -s "/out/output/$name.xsltproc" "/out/output/$name.oxslt" || same=NO
  if [[ $extra == *TOOL* ]]; then
    diff -rq /tmp/chunks-xsltproc /tmp/chunks-oxslt > /dev/null 2>&1 || same=NO
  fi
  # shellcheck disable=SC2086
  xp_kb=$(peak_kb "$XP" $xp_extra "$style" "$input")
  # shellcheck disable=SC2086
  ox_kb=$(peak_kb "$OX" $ox_extra "$style" "$input")
  echo "$name|$same|$xp_kb|$ox_kb" >> /out/summary.txt
  printf '%-28s identical: %-3s  ' "$name" "$same"
  hyperfine -N --style none --warmup 2 --runs "$runs" --export-json "/out/json/$name.json" \
    -n xsltproc "$XP $xp_extra $style $input" -n oxslt "$OX $ox_extra $style $input" > /dev/null 2>&1
  python3 /src/tests/bench/summary.py /out "$name"
done < <(workloads)

echo
python3 /src/tests/bench/summary.py /out | tee /out/summary.md
