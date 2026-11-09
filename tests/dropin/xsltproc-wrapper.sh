#!/bin/bash
# Installed as /usr/bin/xsltproc in the drop-in test image (the real one is
# diverted to /usr/bin/xsltproc.real). Each call runs oxslt first, then the
# real xsltproc, and compares exit codes, standard output and the -o file.
# The build always gets the real xsltproc's result, so a build finishes and
# reports every difference in one run.
#
# Results: one line per call in $DROPIN_OUT/calls.tsv; for each call that
# differs, a directory in $DROPIN_OUT/diffs/ with the command, both outputs
# and both stderrs.

REAL=/usr/bin/xsltproc.real
OX=/opt/oxslt/oxslt
OUT=${DROPIN_OUT:-/out}
PKG=${DROPIN_PKG:-unknown}

id="$(date +%s%N)-$$-$RANDOM"
d="$OUT/diffs/$PKG/$id"
mkdir -p "$d"

# The arguments for oxslt: the same, except that -o goes to a scratch file.
args=("$@")
oxargs=()
out=""
uses_stdin=0
i=0
while [ $i -lt ${#args[@]} ]; do
    a="${args[$i]}"
    case "$a" in
        -o|--output|-output)
            out="${args[$((i + 1))]}"
            oxargs+=("$a" "$d/ox-out")
            i=$((i + 2))
            continue
            ;;
        --param|-param|--stringparam|-stringparam)
            # name and value follow; never treat them as options or files
            oxargs+=("$a" "${args[$((i + 1))]}" "${args[$((i + 2))]}")
            i=$((i + 3))
            continue
            ;;
        -)
            uses_stdin=1
            ;;
    esac
    oxargs+=("$a")
    i=$((i + 1))
done

stdin=/dev/null
if [ $uses_stdin = 1 ]; then
    cat > "$d/stdin"
    stdin="$d/stdin"
fi

now() { date +%s%N; }

t0=$(now)
timeout 300 "$OX" "${oxargs[@]}" < "$stdin" > "$d/ox-stdout" 2> "$d/ox-stderr"
oxrc=$?
t1=$(now)
"$REAL" "$@" < "$stdin" > "$d/real-stdout" 2> "$d/real-stderr"
rc=$?
t2=$(now)

# Hand the build the real result.
cat "$d/real-stdout"
cat "$d/real-stderr" >&2

why=""
[ $oxrc != $rc ] && why="exit $oxrc, xsltproc $rc"
if [ -z "$why" ] && ! cmp -s "$d/ox-stdout" "$d/real-stdout"; then
    why="stdout differs"
fi
if [ -z "$why" ] && [ -n "$out" ]; then
    if [ -d "$out" ] || [ "${out%/}" != "$out" ]; then
        why="-o is a directory"
    elif [ -f "$out" ] && ! cmp -s "$out" "$d/ox-out"; then
        why="-o file differs"
    elif [ -f "$out" ] && [ ! -f "$d/ox-out" ]; then
        why="-o file missing"
    fi
fi

status=same
[ -n "$why" ] && status=DIFF
printf '%s\t%s\t%s\t%s\t%d\t%d\t%s\t%s\n' \
    "$PKG" "$status" "$why" "$PWD" $(((t1 - t0) / 1000000)) $(((t2 - t1) / 1000000)) \
    "$id" "$(printf '%q ' "$@")" >> "$OUT/calls.tsv"

if [ $status = same ]; then
    rm -rf "$d"
else
    printf '%s\n' "$PWD" > "$d/cwd"
    printf '%q ' "$@" > "$d/command"
    [ -n "$out" ] && [ -f "$out" ] && cp "$out" "$d/real-out"
fi
exit $rc
