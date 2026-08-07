#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CANOPUS=${CANOPUS_ROOT:-/Volumes/EXT0/Canopus}
OUT="$ROOT/build"
TRIPLE=thumbv8m.main-none-eabi
CC=${CC:-clang}
mkdir -p "$OUT"

# The module is cross-built on the nightly toolchain with two levers that keep
# the relocatable ELF small enough for the firmware loader and the 128KiB CMI1
# installer receipt bound:
#   - hashed symbol mangling (needs -Z unstable-options) shrinks the long
#     Rust symbol/section names that otherwise dominate .symtab/.strtab.
#   - function-sections=no merges the ~890 per-function sections into a handful,
#     which collapses .shstrtab and the section headers.
# RUSTFLAGS replaces the [target.*] flags in .cargo/config.toml, so panic=abort
# and target-cpu are repeated here.
NIGHTLY=${NIGHTLY_CARGO:-cargo +nightly}
LEAN_RUSTFLAGS="-C panic=abort -C target-cpu=cortex-m33 -Z unstable-options \
  -Z function-sections=no -C symbol-mangling-version=hashed"

cargo fmt --manifest-path "$ROOT/Cargo.toml" --all -- --check
cargo clippy --manifest-path "$ROOT/Cargo.toml" --workspace --all-targets -- -D warnings
cargo test --manifest-path "$ROOT/Cargo.toml" --workspace
# `$NIGHTLY` is intentionally unquoted: the default "cargo +nightly" must
# word-split so `+nightly` is passed to the rustup cargo proxy.
RUSTFLAGS="$LEAN_RUSTFLAGS" $NIGHTLY clippy \
  --manifest-path "$ROOT/Cargo.toml" --release --target "$TRIPLE" \
  -p canopus-bluetooth-audio-device --features device -- -D warnings
RUSTFLAGS="$LEAN_RUSTFLAGS" $NIGHTLY build \
  --manifest-path "$ROOT/Cargo.toml" --release --target "$TRIPLE" \
  -p canopus-bluetooth-audio-device --features device

"$CC" --target=arm-none-eabi -mcpu=cortex-m33 -mthumb -mfloat-abi=soft \
  -ffreestanding -fno-common -fno-builtin -fno-stack-protector \
  -fno-unwind-tables -fno-asynchronous-unwind-tables \
  -fdata-sections -ffunction-sections -Os -Wall -Wextra -Werror \
  -I"$CANOPUS/sdk/c" \
  -c "$ROOT/crates/bluetooth-audio-device/c_shim/canopus_ctor.c" \
  -o "$OUT/canopus_ctor.o"

ld.lld -r "$OUT/canopus_ctor.o" \
  "$ROOT/target/$TRIPLE/release/libcanopus_bluetooth_audio_device.a" \
  -o "$OUT/bluetooth-audio.elf"

# Drop the unconsumed thin-LTO bitcode (.llvmbc) and debug metadata; neither is
# part of the loaded image. objcopy cannot write in place, so write a temp.
OBJCOPY=${RUST_OBJCOPY:-$(command -v rust-objcopy || find "$HOME/.rustup" -name rust-objcopy 2>/dev/null | head -1)}
if [ -n "$OBJCOPY" ]; then
  "$OBJCOPY" --remove-section=.llvmbc --strip-debug \
    "$OUT/bluetooth-audio.elf" "$OUT/bluetooth-audio.elf.strip"
  mv "$OUT/bluetooth-audio.elf.strip" "$OUT/bluetooth-audio.elf"
fi

"$CANOPUS/target/debug/canopus" verify "$OUT/bluetooth-audio.elf" \
  --target xiaomi-band-10-pro-3.101.030 --targets-dir "$CANOPUS/targets"
"$ROOT/scripts/verify-device.sh" "$OUT/bluetooth-audio.elf"
