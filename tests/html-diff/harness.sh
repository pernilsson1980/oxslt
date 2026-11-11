#!/bin/bash
# Runs inside the oxslt-dropin image: compares the trees xsltproc's and
# oxslt's --html parsers build (tests/html-dump.xsl) for every file listed
# in $1. Differences go to /out/diffs/<n>.
list=$1; out=/out; mkdir -p $out/diffs
n=0; same=0; diff=0
while IFS= read -r f; do
  n=$((n+1))
  xsltproc.real --html /s/html-dump.xsl "$f" > /tmp/r.out 2>/dev/null; rr=$?
  timeout 20 /opt/oxslt/oxslt --html /s/html-dump.xsl "$f" > /tmp/o.out 2>/dev/null; ro=$?
  if [ $rr = $ro ] && cmp -s /tmp/r.out /tmp/o.out; then same=$((same+1)); else
    diff=$((diff+1)); { echo "$f rc=$rr/$ro"; diff /tmp/r.out /tmp/o.out | head -20; } > $out/diffs/$n
  fi
done < "$list"
echo "files $n same $same diff $diff"
