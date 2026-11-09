#!/usr/bin/env bash
# Records xsltproc's output and exit code for oxslt's own test cases
# (tests/cases/*), writing expected/stdout and expected/exit in each case.
#
#   scripts/record-expected.sh [case-dir...]
#
# Records with Debian's xsltproc in the oxslt-reference container, the
# reference for every expected result. macOS's xsltproc is also libxslt
# 1.1.35 but Apple's build differs: format-number() rounds 9.95 with "0.0"
# to 9.9, where Debian gives 10.0.

set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
image=oxslt-reference

if [[ -z ${OXSLT_IN_CONTAINER:-} ]]; then
  docker image inspect "$image" >/dev/null 2>&1 || docker build --platform linux/amd64 -t "$image" "$here/tests/reference"
  # Map case paths to their place in the container.
  args=()
  for c in "$@"; do args+=("/repo/tests/cases/$(basename "${c%/}")"); done
  docker run --rm --platform linux/amd64 --network none \
    -e OXSLT_IN_CONTAINER=1 -v "$here/scripts":/repo/scripts:ro -v "$here/tests/cases":/repo/tests/cases \
    "$image" bash /repo/scripts/record-expected.sh ${args[@]+"${args[@]}"} 2>&1 | grep -v 'WARNING: The requested image'
  exit "${PIPESTATUS[0]}"
fi

bin=/usr/bin/xsltproc
cases=("$@")
[[ ${#cases[@]} -gt 0 ]] || cases=("$here"/tests/cases/*/)

"$bin" --version | head -1
for case in "${cases[@]}"; do
  case=${case%/}
  args=() envs=()
  [[ -f $case/args ]] && while IFS= read -r a || [[ -n $a ]]; do [[ -n $a ]] && args+=("$a"); done < "$case/args"
  [[ -f $case/env ]] && while IFS= read -r e || [[ -n $e ]]; do
    [[ -n $e ]] || continue
    key=${e%%=*}
    [[ -z ${!key+x} ]] && envs+=("$e")
  done < "$case/env"
  mkdir -p "$case/expected"
  set +e
  (cd "$case" && env ${envs[@]+"${envs[@]}"} "$bin" ${args[@]+"${args[@]}"} style.xsl in.xml \
     > expected/stdout 2> /dev/null)
  echo $? > "$case/expected/exit"
  set -e
  printf '%-28s exit %s\n' "$(basename "$case")" "$(cat "$case/expected/exit")"
done
