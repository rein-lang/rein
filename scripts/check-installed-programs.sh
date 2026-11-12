#!/usr/bin/env sh
# Build rein from this checkout, then run its check on every installed program.
# The pre-push hook runs this, so a checker change that breaks an installed program blocks the push.
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
cargo build -q -p rein
rein="$root/target/debug/rein"

list="$(mktemp)"
trap 'rm -f "$list"' EXIT
"$rein" list -l > "$list"

if [ ! -s "$list" ]; then
  echo "no installed programs; nothing to check"
  exit 0
fi

failed=""
# Columns: scope version name path description. The path has no spaces in the default layout.
while read -r _scope _version name dir _rest; do
  code=0
  out="$("$rein" check "$dir" 2>&1)" || code=$?
  echo "$name: $(printf '%s\n' "$out" | tail -n 1)"
  if [ "$code" -ne 0 ]; then
    printf '%s\n' "$out"
    failed="$failed $name"
  fi
done < "$list"

if [ -n "$failed" ]; then
  echo "installed programs fail the check:$failed" >&2
  echo "a checker change breaks an installed program; fix the program first or make the rule a warning" >&2
  exit 1
fi
