#!/bin/sh
set -eu
OUT=${1:?usage: compile-sbc.sh output-dir cc cpu}
CC=${2:?usage: compile-sbc.sh output-dir cc cpu}
CPU=${3:?usage: compile-sbc.sh output-dir cc cpu}
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SBC="$ROOT/third_party/sbc"
COMMON="--target=arm-none-eabi -mcpu=$CPU -mthumb -mfloat-abi=soft \
  -DCANOPUS_FREESTANDING=1 -ffreestanding -fno-common -fno-builtin \
  -fno-stack-protector -fno-unwind-tables -fno-asynchronous-unwind-tables \
  -fdata-sections -ffunction-sections -O3 -Wall -Wextra -I$SBC"

# Intentional word splitting turns COMMON into compiler arguments.
# shellcheck disable=SC2086
"$CC" $COMMON -Wno-unused-parameter -Wno-ignored-attributes \
  -c "$SBC/sbc.c" -o "$OUT/sbc.o"
# shellcheck disable=SC2086
"$CC" $COMMON -Wno-unused-parameter -Wno-ignored-attributes \
  -c "$SBC/sbc_primitives.c" -o "$OUT/sbc_primitives.o"
# shellcheck disable=SC2086
"$CC" $COMMON -Werror \
  -c "$SBC/canopus_sbc_wrapper.c" -o "$OUT/canopus_sbc_wrapper.o"
