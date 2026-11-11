#!/bin/bash
# Inside the container: run.sh corpus | run.sh fuzz COUNT SEED
case $1 in
  corpus)
    mkdir -p /tmp/src && cd /tmp/src
    for t in /dropin/src/*.orig.tar.* /dropin/src/*.debian.tar.*; do
      tar xf "$t" --wildcards '*.htm' '*.html' '*.HTM' '*.html.in' '*.xhtml' 2>/dev/null
    done
    find /tmp/src /usr/share/doc -type f \( -iname '*.htm' -o -iname '*.html' -o -name '*.html.in' -o -name '*.xhtml' \) | sort > /tmp/list ;;
  fuzz)
    python3 /s/gen.py /tmp/gen "${2:-2000}" "${3:-1}" && ls /tmp/gen/*.html > /tmp/list ;;
esac
/s/harness.sh /tmp/list
