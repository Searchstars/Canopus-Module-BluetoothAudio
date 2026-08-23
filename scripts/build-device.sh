#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CANOPUS=${CANOPUS_ROOT:-"$ROOT/../Canopus"}
TARGET_ID=${CANOPUS_TARGET:-xiaomi-band-10-pro-3.101.036}
TARGET_PROFILE="$ROOT/targets/$TARGET_ID.env"
[ -f "$TARGET_PROFILE" ] || {
  echo "error: unsupported module target: $TARGET_ID" >&2
  exit 1
}
# Repository-owned profile: Rust feature, LLVM target, CPU, and loader bound.
. "$TARGET_PROFILE"
export CANOPUS_STATIC_CANDIDATE="${CANOPUS_STATIC_CANDIDATE:-0}"
OUT=${CANOPUS_BUILD_OUT:-"$ROOT/build/$TARGET_ID"}
TRIPLE=$RUST_TARGET_TRIPLE
CC=${CC:-clang}
mkdir -p "$OUT"

# The module is cross-built on the nightly toolchain with two levers that keep
# the relocatable ELF compact while allowing the decoder's hot loops to use a
# separate throughput-oriented package profile:
#   - hashed symbol mangling (needs -Z unstable-options) shrinks the long
#     Rust symbol/section names that otherwise dominate .symtab/.strtab.
#   - function-sections=no merges the ~890 per-function sections into a handful,
#     which collapses .shstrtab and the section headers.
# RUSTFLAGS replaces the [target.*] flags in .cargo/config.toml, so panic=abort
# and target-cpu are repeated here.
NIGHTLY=${NIGHTLY_CARGO:-cargo +nightly}
LEAN_RUSTFLAGS="-C panic=abort -C target-cpu=$RUST_TARGET_CPU -Z unstable-options \
  -Z function-sections=no -C symbol-mangling-version=hashed \
  -Z location-detail=none -Z fmt-debug=none"

cargo fmt --manifest-path "$ROOT/Cargo.toml" --all -- --check
cargo clippy --manifest-path "$ROOT/Cargo.toml" --workspace --all-targets -- -D warnings
cargo test --manifest-path "$ROOT/Cargo.toml" --workspace
# `$NIGHTLY` is intentionally unquoted: the default "cargo +nightly" must
# word-split so `+nightly` is passed to the rustup cargo proxy.
RUSTFLAGS="$LEAN_RUSTFLAGS" $NIGHTLY clippy \
  --manifest-path "$ROOT/Cargo.toml" --release --target "$TRIPLE" \
  -p canopus-bluetooth-audio-device --no-default-features \
  --features "$RUST_TARGET_FEATURE" -- -D warnings
RUSTFLAGS="$LEAN_RUSTFLAGS" $NIGHTLY build \
  --manifest-path "$ROOT/Cargo.toml" --release --target "$TRIPLE" \
  -p canopus-bluetooth-audio-device --no-default-features \
  --features "$RUST_TARGET_FEATURE"

"$CC" --target=arm-none-eabi -mcpu="$RUST_TARGET_CPU" -mthumb -mfloat-abi=soft \
  -ffreestanding -fno-common -fno-builtin -fno-stack-protector \
  -fno-unwind-tables -fno-asynchronous-unwind-tables \
  -fdata-sections -ffunction-sections -Os -Wall -Wextra -Werror \
  -I"$CANOPUS/sdk/c" \
  -DCANOPUS_STATIC_CANDIDATE="$CANOPUS_STATIC_CANDIDATE" \
  -c "$ROOT/crates/bluetooth-audio-device/c_shim/canopus_ctor.c" \
  -o "$OUT/canopus_ctor.o"
"$ROOT/scripts/compile-sbc.sh" "$OUT" "$CC" "$RUST_TARGET_CPU"

"$ROOT/scripts/link-device.sh" \
  "$OUT" "$CC" "$RUST_TARGET_CPU" "$CANOPUS" "$TARGET_ID" "$ROOT" "$TRIPLE"

# Drop the unconsumed thin-LTO bitcode (.llvmbc) and debug metadata; neither is
# part of the loaded image. objcopy cannot write in place, so write a temp.
OBJCOPY=${RUST_OBJCOPY:-$(command -v rust-objcopy || find "$HOME/.rustup" -name rust-objcopy 2>/dev/null | head -1)}
if [ -n "$OBJCOPY" ]; then
  "$OBJCOPY" --remove-section=.llvmbc --strip-debug \
    "$OUT/bluetooth-audio.elf" "$OUT/bluetooth-audio.elf.strip"
  mv "$OUT/bluetooth-audio.elf.strip" "$OUT/bluetooth-audio.elf"
fi

"$CANOPUS/target/debug/canopus" verify "$OUT/bluetooth-audio.elf" \
  --target "$TARGET_ID" --targets-dir "$CANOPUS/targets"
"$ROOT/scripts/verify-device.sh" "$OUT/bluetooth-audio.elf" "$MODULE_MAX_SIZE"
