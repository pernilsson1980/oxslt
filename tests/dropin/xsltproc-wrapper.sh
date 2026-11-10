#!/bin/bash
# Installed as /usr/bin/xsltproc in the drop-in test image (the real one is
# diverted to /usr/bin/xsltproc.real). Each call runs oxslt, then the real
# xsltproc, with the same arguments, and compares exit codes, standard
# output and every file each one wrote: the -o file and the files
# exsl:document wrote. The build always gets the real xsltproc's result and
# files, so a build finishes and reports every difference in one run.
#
# Written files are found by comparing snapshots (path, modification time,
# size, inode) of the working directory and the -o file's directory taken
# before and after each run, not by time alone: file times come from a
# coarse clock, which put files the build wrote just before a call into it.
# Builds run serially (run.sh), so only this call writes in between.
#
# Results: one line per call in $DROPIN_OUT/calls.tsv; for each call that
# differs, a directory in $DROPIN_OUT/diffs/ with the command, both standard
# outputs and stderrs, and both sets of written files (ox-files/,
# real-files/, by path).

REAL=/usr/bin/xsltproc.real
OX=/opt/oxslt/oxslt
OUT=${DROPIN_OUT:-/out}
PKG=${DROPIN_PKG:-unknown}

id="$(date +%s%N)-$$-$RANDOM"
d="$OUT/diffs/$PKG/$id"
mkdir -p "$d"

# Where written files can be: the working directory and the -o directory.
args=("$@")
roots=("$PWD")
uses_stdin=0
i=0
while [ $i -lt ${#args[@]} ]; do
    case "${args[$i]}" in
        -o|--output|-output)
            o="${args[$((i + 1))]}"
            case "$o" in
                */) roots+=("$o") ;;
                */*) roots+=("$(dirname "$o")") ;;
            esac
            i=$((i + 2)); continue ;;
        --param|-param|--stringparam|-stringparam) i=$((i + 3)); continue ;;
        -) uses_stdin=1 ;;
    esac
    i=$((i + 1))
done

stdin=/dev/null
if [ $uses_stdin = 1 ]; then
    cat > "$d/stdin"
    stdin="$d/stdin"
fi

# Every file under the roots: path, modification time, size, inode.
snapshot() {
    for r in "${roots[@]}"; do
        [ -d "$r" ] && find "$(cd "$r" && pwd)" -xdev -type f -printf '%p\t%T@\t%s\t%i\n' 2>/dev/null
    done | grep -v "^$OUT/" | sort -u
}

# The files new or changed between two snapshots, as paths.
written_between() {
    comm -13 "$1" "$2" | cut -f1 | sort -u
}

now() { date +%s%N; }

snapshot > "$d/before-ox"
t0=$(now)
timeout 300 "$OX" "$@" < "$stdin" > "$d/ox-stdout" 2> "$d/ox-stderr"
oxrc=$?
t1=$(now)
snapshot > "$d/after-ox"
written_between "$d/before-ox" "$d/after-ox" > "$d/ox-written"
# oxslt's files are kept on the container's /tmp for the comparison: the
# results directory may be on a case-insensitive file system (macOS), where
# PAM.7 and pam.7 are one file.
oxfiles="/tmp/ox-files-$id"
while IFS= read -r f; do
    mkdir -p "$oxfiles$(dirname "$f")"
    cp -p "$f" "$oxfiles$f"
done < "$d/ox-written"

t2=$(now)
"$REAL" "$@" < "$stdin" > "$d/real-stdout" 2> "$d/real-stderr"
rc=$?
t3=$(now)
snapshot > "$d/after-real"
written_between "$d/after-ox" "$d/after-real" > "$d/real-written"

# Hand the build the real result.
cat "$d/real-stdout"
cat "$d/real-stderr" >&2

why=""
[ $oxrc != $rc ] && why="exit $oxrc, xsltproc $rc"
if [ -z "$why" ] && ! cmp -s "$d/ox-stdout" "$d/real-stdout"; then
    why="stdout differs"
fi
if [ -z "$why" ] && ! cmp -s "$d/ox-written" "$d/real-written"; then
    why="different files written"
fi
if [ -z "$why" ]; then
    while IFS= read -r f; do
        if ! cmp -s "$f" "$oxfiles$f"; then
            why="written file differs: $f"
            break
        fi
    done < "$d/real-written"
fi

status=same
[ -n "$why" ] && status=DIFF
printf '%s\t%s\t%s\t%s\t%d\t%d\t%s\t%s\n' \
    "$PKG" "$status" "$why" "$PWD" $(((t1 - t0) / 1000000)) $(((t3 - t2) / 1000000)) \
    "$id" "$(printf '%q ' "$@")" >> "$OUT/calls.tsv"

if [ $status = same ]; then
    rm -rf "$d" "$oxfiles"
else
    mkdir -p "$d/ox-files"
    cp -a "$oxfiles/." "$d/ox-files/" 2>/dev/null
    rm -rf "$oxfiles"
    printf '%s\n' "$PWD" > "$d/cwd"
    printf '%q ' "$@" > "$d/command"
    while IFS= read -r f; do
        mkdir -p "$d/real-files$(dirname "$f")"
        cp "$f" "$d/real-files$f"
    done < "$d/real-written"
fi
exit $rc
