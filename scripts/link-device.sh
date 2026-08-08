#!/bin/sh
set -eu

OUT=${1:?usage: link-device.sh out cc cpu canopus target-id root triple}
CC=${2:?usage: link-device.sh out cc cpu canopus target-id root triple}
CPU=${3:?usage: link-device.sh out cc cpu canopus target-id root triple}
CANOPUS=${4:?usage: link-device.sh out cc cpu canopus target-id root triple}
TARGET_ID=${5:?usage: link-device.sh out cc cpu canopus target-id root triple}
ROOT=${6:?usage: link-device.sh out cc cpu canopus target-id root triple}
TRIPLE=${7:?usage: link-device.sh out cc cpu canopus target-id root triple}

PRELIM="$OUT/bluetooth-audio.prelim.elf"
CANDIDATE="$OUT/bluetooth-audio.codec-layout.elf"
FINAL="$OUT/bluetooth-audio.elf"
VERIFY_PRELIM="$OUT/codec-verifier-prelim.txt"
VERIFY_LAYOUT="$OUT/codec-verifier-layout.txt"
FIXUP_C="$OUT/codec-fixups.c"
FIXUP_O="$OUT/codec-fixups.o"
FIXUP_JSON="$OUT/codec-fixups.json"
RUSTLIB="$ROOT/target/$TRIPLE/release/libcanopus_bluetooth_audio_device.a"

link_base() {
  output=$1
  shift
  ld.lld -r --gc-sections -u canopus_module_descriptor \
    "$OUT/canopus_ctor.o" "$@" \
    "$OUT/sbc.o" "$OUT/sbc_primitives.o" \
    "$OUT/canopus_sbc_wrapper.o" "$RUSTLIB" -o "$output"
}

compile_fixups() {
  "$CC" --target=arm-none-eabi -mcpu="$CPU" -mthumb -mfloat-abi=soft \
    -ffreestanding -fno-common -fno-builtin -fno-stack-protector \
    -fno-unwind-tables -fno-asynchronous-unwind-tables \
    -fdata-sections -ffunction-sections -Os -Wall -Wextra -Werror \
    -c "$FIXUP_C" -o "$FIXUP_O"
}

# The codec's packed Huffman/transform tables naturally contain a few aligned
# words that alias this target's XIP range. Encode only verifier-reported words
# in the ELF and generate constructor fixups that restore them in modlib-owned
# RAM before Rust runs. The second layout pass accounts for the generated
# function and relocation table becoming live.
link_base "$PRELIM"
if "$CANOPUS/target/debug/canopus" verify "$PRELIM" \
    --target "$TARGET_ID" --targets-dir "$CANOPUS/targets" \
    >"$VERIFY_PRELIM" 2>&1; then
  echo "error: preliminary codec link unexpectedly had no words to encode" >&2
  exit 1
fi
python3 "$ROOT/scripts/encode-codec-words.py" generate \
  --verifier-output "$VERIFY_PRELIM" --output-c "$FIXUP_C" \
  --metadata "$FIXUP_JSON"
compile_fixups
link_base "$CANDIDATE" "$FIXUP_O"
if "$CANOPUS/target/debug/canopus" verify "$CANDIDATE" \
    --target "$TARGET_ID" --targets-dir "$CANOPUS/targets" \
    >"$VERIFY_LAYOUT" 2>&1; then
  echo "error: codec layout link unexpectedly had no words to encode" >&2
  exit 1
fi
python3 "$ROOT/scripts/encode-codec-words.py" generate \
  --verifier-output "$VERIFY_LAYOUT" --output-c "$FIXUP_C" \
  --metadata "$FIXUP_JSON"
compile_fixups
link_base "$FINAL" "$FIXUP_O"
python3 "$ROOT/scripts/encode-codec-words.py" patch \
  --elf "$FINAL" --metadata "$FIXUP_JSON"
