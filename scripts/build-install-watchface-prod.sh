#!/bin/sh
# Build one signed prod installer per device, with exact firmware selection.
# Usage: scripts/build-install-watchface-prod.sh [device-or-target ...]
# CANOPUS_DEVICE takes precedence over CANOPUS_TARGET; arguments override both.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CANOPUS=${CANOPUS_ROOT:-"$ROOT/../Canopus"}
SELECTION=${CANOPUS_DEVICE:-${CANOPUS_TARGET:-"xiaomi-band-10-pro xiaomi-band-11"}}
if [ "$#" -gt 0 ]; then SELECTION="$*"; fi
TARGET_IDS=""
for ITEM in $SELECTION; do
  case "$ITEM" in
    xiaomi-band-10-pro) EXPANDED="xiaomi-band-10-pro-3.101.036 xiaomi-band-10-pro-3.101.043" ;;
    xiaomi-band-11) EXPANDED="xiaomi-band-11-4.100.139" ;;
    xiaomi-band-10-pro-3.101.036|xiaomi-band-10-pro-3.101.043|xiaomi-band-11-4.100.139) EXPANDED="$ITEM" ;;
    *) echo "unsupported prod device/target: $ITEM" >&2; exit 1 ;;
  esac
  for TARGET_ID in $EXPANDED; do
    case " $TARGET_IDS " in *" $TARGET_ID "*) ;; *) TARGET_IDS="$TARGET_IDS $TARGET_ID" ;; esac
  done
done
WATCHFACE=${CANOPUS_WATCHFACE_OUT:-"$ROOT/watchfaces/bluetooth-audio-prod"}
PAYLOADS="$ROOT/build/bluetooth-audio-prod"
cargo fmt --manifest-path "$ROOT/Cargo.toml" --all -- --check
cargo test --manifest-path "$ROOT/Cargo.toml" -p canopus-bluetooth-audio-core --features production
for TARGET_ID in $TARGET_IDS; do
  "$ROOT/scripts/build-install-payload.sh" "$TARGET_ID" "$PAYLOADS/$TARGET_ID" production
done
set -- --product bluetooth-audio --payload-dir "$PAYLOADS" \
  --assets-dir "$ROOT/watchfaces/bluetooth-audio" --output-dir "$WATCHFACE"
for TARGET_ID in $TARGET_IDS; do set -- "$@" --target "$TARGET_ID"; done
python3 "$CANOPUS/scripts/build_module_installer_prod.py" "$@"
for TARGET_ID in $TARGET_IDS; do
  DEVICE=${TARGET_ID%-*}
  luac -p "$WATCHFACE/$DEVICE/main.lua"
done
python3 -m unittest discover -s "$CANOPUS/scripts/tests" -p test_module_installer_prod.py
echo "$WATCHFACE is ready to package (Band 11 device radio/display validation pending)"
