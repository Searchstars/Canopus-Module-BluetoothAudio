#!/bin/sh
# Builds the Bluetooth audio module and stages it as a one-shot installer
# watchface (watchfaces/bluetooth-audio). Installing that watchface on the
# xiaomi-band-10-pro-3.101.030 stages the signed receipt and ELF and asks the
# resident Canopus supervisor (/dev/canopus) to register the module.
#
# Pipeline:
#   Rust staticlib + C ctor shim -> ld.lld -r -> ET_REL -> Canopus verifier
#   -> CMI1 Ed25519-signed receipt (dev key) -> watchfaces/bluetooth-audio/*
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CANOPUS=${CANOPUS_ROOT:-/Volumes/EXT0/Canopus}
OUT="$ROOT/build"
WATCHFACE="$ROOT/watchfaces/bluetooth-audio"
TRIPLE=thumbv8m.main-none-eabi
CC=${CC:-clang}
KEY=${MODULE_INSTALL_KEY:-"$CANOPUS/.canopus-local/module-installer-ed25519.pem"}
TOKEN=bluetooth_audio

mkdir -p "$OUT" "$WATCHFACE"

# Same lean nightly build as build-device.sh: hashed mangling + merged
# function sections shrink the ELF below the firmware loader limit and the
# 128KiB CMI1 receipt bound.
NIGHTLY=${NIGHTLY_CARGO:-cargo +nightly}
LEAN_RUSTFLAGS="-C panic=abort -C target-cpu=cortex-m33 -Z unstable-options \
  -Z function-sections=no -C symbol-mangling-version=hashed"

echo "[1/4] cross-build Rust staticlib (nightly, lean)"
# `$NIGHTLY` is intentionally unquoted: the default "cargo +nightly" must
# word-split so `+nightly` is passed to the rustup cargo proxy.
RUSTFLAGS="$LEAN_RUSTFLAGS" $NIGHTLY build \
  --manifest-path "$ROOT/Cargo.toml" --release --target "$TRIPLE" \
  -p canopus-bluetooth-audio-device --features device

echo "[2/4] relocatable link + strip + Canopus verifier"
"$CC" --target=arm-none-eabi -mcpu=cortex-m33 -mthumb -mfloat-abi=soft \
  -ffreestanding -fno-common -fno-builtin -fno-stack-protector \
  -fno-unwind-tables -fno-asynchronous-unwind-tables \
  -fdata-sections -ffunction-sections -Os -Wall -Wextra -Werror \
  -c "$ROOT/crates/bluetooth-audio-device/c_shim/canopus_ctor.c" \
  -o "$OUT/canopus_ctor.o"
ld.lld -r "$OUT/canopus_ctor.o" \
  "$ROOT/target/$TRIPLE/release/libcanopus_bluetooth_audio_device.a" \
  -o "$OUT/bluetooth-audio.elf"
OBJCOPY=${RUST_OBJCOPY:-$(command -v rust-objcopy || find "$HOME/.rustup" -name rust-objcopy 2>/dev/null | head -1)}
if [ -n "$OBJCOPY" ]; then
  "$OBJCOPY" --remove-section=.llvmbc --strip-debug \
    "$OUT/bluetooth-audio.elf" "$OUT/bluetooth-audio.elf.strip"
  mv "$OUT/bluetooth-audio.elf.strip" "$OUT/bluetooth-audio.elf"
fi
"$CANOPUS/target/debug/canopus" verify "$OUT/bluetooth-audio.elf" \
  --target xiaomi-band-10-pro-3.101.030 --targets-dir "$CANOPUS/targets"

echo "[3/4] sign CMI1 installer receipt"
[ -f "$KEY" ] || {
    echo "error: module installer key not found: $KEY" >&2
    exit 1
}
python3 "$CANOPUS/scripts/build-module-installer-receipt.py" \
  --module "$OUT/bluetooth-audio.elf" \
  --module-id "$TOKEN" \
  --version 1 \
  --lifecycle 0 \
  --private-key "$KEY" \
  --output "$OUT/receipt.bin"

echo "[4/4] smoke-test watchface Lua and stage payloads"
lua "$ROOT/scripts/smoke-watchface.lua" >/dev/null
cp "$OUT/bluetooth-audio.elf" "$WATCHFACE/module.bin"
cp "$OUT/receipt.bin" "$WATCHFACE/receipt.bin"
python3 - "$WATCHFACE" <<'EOF'
import hashlib, pathlib, struct, sys
watchface = pathlib.Path(sys.argv[1])
module = (watchface / "module.bin").read_bytes()
receipt = (watchface / "receipt.bin").read_bytes()
assert module[:4] == b"\x7fELF" and 512 <= len(module) <= 131072
assert receipt[:4] == b"CMI1" and len(receipt) == 256
magic, version, header, flags, lifecycle, module_version, artifact_size, reserved = struct.unpack("<8I", receipt[:32])
assert magic == 0x31494D43 and header == 256
assert artifact_size == len(module), (artifact_size, len(module))
expected = hashlib.sha256(module).digest()
actual = receipt[144:176]
assert actual == expected, (actual.hex(), expected.hex())
name = receipt[32:64].split(b"\0", 1)[0]
assert name == b"bluetooth_audio", name
print(f"watchface staged OK: module={len(module)}B receipt={len(receipt)}B sha256={expected.hex()}")
EOF
echo "watchfaces/bluetooth-audio is ready to install"
