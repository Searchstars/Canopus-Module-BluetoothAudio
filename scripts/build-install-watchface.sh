#!/bin/sh
# Builds the Bluetooth audio module and stages it as a one-shot installer
# watchface (watchfaces/bluetooth-audio). Installing that watchface on the
# selected exact target stages the signed receipt and ELF and asks the
# resident Canopus supervisor (/dev/canopus) to register the module.
#
# Pipeline:
#   Rust staticlib + C ctor shim -> ld.lld -r -> ET_REL -> Canopus verifier
#   -> CMI1 Ed25519-signed receipt (dev key) -> watchfaces/bluetooth-audio/*
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CANOPUS=${CANOPUS_ROOT:-"$ROOT/../Canopus"}
TARGET_ID=${CANOPUS_TARGET:-xiaomi-band-10-pro-3.101.030}
TARGET_PROFILE="$ROOT/targets/$TARGET_ID.env"
[ -f "$TARGET_PROFILE" ] || {
  echo "error: unsupported module target: $TARGET_ID" >&2
  exit 1
}
. "$TARGET_PROFILE"
OUT=${CANOPUS_BUILD_OUT:-"$ROOT/build/$TARGET_ID"}
WATCHFACE="$ROOT/watchfaces/bluetooth-audio"
TRIPLE=$RUST_TARGET_TRIPLE
CC=${CC:-clang}
KEY=${MODULE_INSTALL_KEY:-"$CANOPUS/.canopus-local/module-installer-ed25519.pem"}
TOKEN=bluetooth_audio
LIFECYCLE=$(python3 - "$ROOT/Canopus.toml" <<'PY'
import pathlib, sys, tomllib
value = tomllib.loads(pathlib.Path(sys.argv[1]).read_text())["module"]["lifecycle"]
classes = {
    "removable": 0,
    "resident-after-activation": 1,
    "always-resident": 2,
    "patch-reboot-required": 3,
}
if value not in classes:
    raise SystemExit(f"unsupported module lifecycle: {value}")
print(classes[value])
PY
)

mkdir -p "$OUT" "$WATCHFACE"

# Same lean nightly build as build-device.sh: hashed mangling + merged
# function sections shrink the ELF while the MP3 decoder retains throughput.
NIGHTLY=${NIGHTLY_CARGO:-cargo +nightly}
LEAN_RUSTFLAGS="-C panic=abort -C target-cpu=$RUST_TARGET_CPU -Z unstable-options \
  -Z function-sections=no -C symbol-mangling-version=hashed \
  -Z location-detail=none -Z fmt-debug=none"

echo "[1/4] cross-build Rust staticlib (nightly, lean)"
# `$NIGHTLY` is intentionally unquoted: the default "cargo +nightly" must
# word-split so `+nightly` is passed to the rustup cargo proxy.
RUSTFLAGS="$LEAN_RUSTFLAGS" $NIGHTLY build \
  --manifest-path "$ROOT/Cargo.toml" --release --target "$TRIPLE" \
  -p canopus-bluetooth-audio-device --no-default-features \
  --features "$RUST_TARGET_FEATURE"

echo "[2/4] relocatable link + strip + Canopus verifier"
"$CC" --target=arm-none-eabi -mcpu="$RUST_TARGET_CPU" -mthumb -mfloat-abi=soft \
  -ffreestanding -fno-common -fno-builtin -fno-stack-protector \
  -fno-unwind-tables -fno-asynchronous-unwind-tables \
  -fdata-sections -ffunction-sections -Os -Wall -Wextra -Werror \
  -I"$CANOPUS/sdk/c" \
  -c "$ROOT/crates/bluetooth-audio-device/c_shim/canopus_ctor.c" \
  -o "$OUT/canopus_ctor.o"
"$ROOT/scripts/compile-sbc.sh" "$OUT" "$CC" "$RUST_TARGET_CPU"
"$ROOT/scripts/link-device.sh" \
  "$OUT" "$CC" "$RUST_TARGET_CPU" "$CANOPUS" "$TARGET_ID" "$ROOT" "$TRIPLE"
OBJCOPY=${RUST_OBJCOPY:-$(command -v rust-objcopy || find "$HOME/.rustup" -name rust-objcopy 2>/dev/null | head -1)}
if [ -n "$OBJCOPY" ]; then
  "$OBJCOPY" --remove-section=.llvmbc --strip-debug \
    "$OUT/bluetooth-audio.elf" "$OUT/bluetooth-audio.elf.strip"
  mv "$OUT/bluetooth-audio.elf.strip" "$OUT/bluetooth-audio.elf"
fi
"$CANOPUS/target/debug/canopus" verify "$OUT/bluetooth-audio.elf" \
  --target "$TARGET_ID" --targets-dir "$CANOPUS/targets"
"$ROOT/scripts/verify-device.sh" "$OUT/bluetooth-audio.elf" "$MODULE_MAX_SIZE"

echo "[3/4] sign CMI1 installer receipt"
[ -f "$KEY" ] || {
    echo "error: module installer key not found: $KEY" >&2
    exit 1
}
python3 "$CANOPUS/scripts/build-module-installer-receipt.py" \
  --module "$OUT/bluetooth-audio.elf" \
  --module-id "$TOKEN" \
  --version 1 \
  --lifecycle "$LIFECYCLE" \
  --private-key "$KEY" \
  --output "$OUT/receipt.bin"

echo "[4/4] strip test-only ID3 artwork, smoke-test watchface Lua, and stage payloads"
python3 - "$WATCHFACE/long_test_audio.bin" "$WATCHFACE/long_test_audio_stream.bin" <<'PY'
import pathlib, sys
source = pathlib.Path(sys.argv[1]).read_bytes()
if len(source) < 10 or source[:3] != b"ID3":
    raise SystemExit("long test audio is missing its ID3 header")
size = ((source[6] & 0x7f) << 21) | ((source[7] & 0x7f) << 14) | ((source[8] & 0x7f) << 7) | (source[9] & 0x7f)
offset = 10 + size
stream = source[offset:]
if len(stream) < 4096 or stream[0] != 0xff or stream[1] & 0xe0 != 0xe0:
    raise SystemExit("long test audio has no MPEG frame after ID3")
pathlib.Path(sys.argv[2]).write_bytes(stream)
PY
lua "$ROOT/scripts/smoke-watchface.lua" "$WATCHFACE/main.lua" >/dev/null
cp "$OUT/bluetooth-audio.elf" "$WATCHFACE/module.bin"
cp "$OUT/receipt.bin" "$WATCHFACE/receipt.bin"
python3 - "$WATCHFACE" "$LIFECYCLE" <<'EOF'
import hashlib, pathlib, struct, sys
watchface = pathlib.Path(sys.argv[1])
expected_lifecycle = int(sys.argv[2])
module = (watchface / "module.bin").read_bytes()
receipt = (watchface / "receipt.bin").read_bytes()
long_audio = (watchface / "long_test_audio.bin").read_bytes()
long_audio_stream = (watchface / "long_test_audio_stream.bin").read_bytes()
assert module[:4] == b"\x7fELF" and 512 <= len(module) <= 262144
assert receipt[:4] == b"CMI1" and len(receipt) == 256
assert long_audio[:3] == b"ID3" and len(long_audio) >= 4096
assert long_audio_stream[0] == 0xff and long_audio_stream[1] & 0xe0 == 0xe0
assert len(long_audio_stream) >= 4096
magic, version, header, flags, lifecycle, module_version, artifact_size, reserved = struct.unpack("<8I", receipt[:32])
assert magic == 0x31494D43 and header == 256
assert lifecycle == expected_lifecycle, lifecycle
assert artifact_size == len(module), (artifact_size, len(module))
expected = hashlib.sha256(module).digest()
actual = receipt[144:176]
assert actual == expected, (actual.hex(), expected.hex())
name = receipt[32:64].split(b"\0", 1)[0]
assert name == b"bluetooth_audio", name
print(f"watchface staged OK: module={len(module)}B receipt={len(receipt)}B "
      f"long_audio={len(long_audio)}B stream={len(long_audio_stream)}B sha256={expected.hex()}")
EOF
echo "watchfaces/bluetooth-audio is ready to install"
