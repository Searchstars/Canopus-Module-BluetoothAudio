#!/bin/sh
# Build every target included by Canopus.toml, or the target IDs passed as args.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

if [ "$#" -eq 0 ]; then
  set -- $(python3 - "$ROOT/Canopus.toml" <<'PY'
import pathlib, sys, tomllib
manifest = tomllib.loads(pathlib.Path(sys.argv[1]).read_text())
for target in manifest["targets"].get("include", []):
    print(target)
PY
  )
fi

[ "$#" -gt 0 ] || {
  echo "error: no targets selected" >&2
  exit 1
}

for target in "$@"; do
  echo "==> build target: $target"
  CANOPUS_TARGET="$target" "$ROOT/scripts/build-device.sh"
done
