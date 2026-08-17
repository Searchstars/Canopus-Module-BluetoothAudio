#!/bin/sh
# Builds the Bluetooth audio module and stages the single-target development
# installer watchface, including its long-MP3 diagnostic resource.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TARGET_ID=${CANOPUS_TARGET:-xiaomi-band-10-pro-3.101.030}
OUT=${CANOPUS_BUILD_OUT:-"$ROOT/build/$TARGET_ID"}
WATCHFACE="$ROOT/watchfaces/bluetooth-audio"
mkdir -p "$OUT" "$WATCHFACE"

"$ROOT/scripts/build-install-payload.sh" "$TARGET_ID" "$OUT"

echo "[watchface] strip test-only ID3 artwork and stage development payloads"
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
python3 - "$OUT" "$WATCHFACE" "$TARGET_ID" <<'PY'
import hashlib, pathlib, struct, sys
out = pathlib.Path(sys.argv[1])
watchface = pathlib.Path(sys.argv[2])
expected_target = sys.argv[3]
source_module = (out / "bluetooth-audio.elf").read_bytes()
source_receipt = (out / "receipt.bin").read_bytes()
module = (watchface / "module.bin").read_bytes()
receipt = (watchface / "receipt.bin").read_bytes()
long_audio = (watchface / "long_test_audio.bin").read_bytes()
long_audio_stream = (watchface / "long_test_audio_stream.bin").read_bytes()
appicon = (watchface / "appicon_headphones.bin").read_bytes()
assert module == source_module and receipt == source_receipt
assert module[:4] == b"\x7fELF" and 512 <= len(module) <= 262144
assert receipt[:4] == b"CMI1" and len(receipt) == 256
assert len(appicon) == 54768 and appicon[:4] == b"\x19\x10\0\0"
icon_width, icon_height, icon_stride, icon_reserved = struct.unpack_from("<4H", appicon, 4)
assert (icon_width, icon_height, icon_stride, icon_reserved) == (117, 117, 468, 0)
assert len(appicon) == 12 + icon_height * icon_stride
assert long_audio[:3] == b"ID3" and len(long_audio) >= 4096
assert long_audio_stream[0] == 0xff and long_audio_stream[1] & 0xe0 == 0xe0
assert len(long_audio_stream) >= 4096
artifact_size = struct.unpack_from("<I", receipt, 24)[0]
assert artifact_size == len(module), (artifact_size, len(module))
expected = hashlib.sha256(module).digest()
actual = receipt[144:176]
assert actual == expected, (actual.hex(), expected.hex())
name = receipt[32:64].split(b"\0", 1)[0]
target_id = receipt[64:112].split(b"\0", 1)[0].decode("ascii")
assert name == b"bluetooth_audio", name
assert target_id == expected_target, (target_id, expected_target)
print(
    f"watchface staged OK: target={target_id} module={len(module)}B receipt={len(receipt)}B "
    f"long_audio={len(long_audio)}B stream={len(long_audio_stream)}B sha256={expected.hex()}"
)
PY
echo "watchfaces/bluetooth-audio is ready to install"
