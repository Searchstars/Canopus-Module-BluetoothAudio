#!/bin/sh
# Builds one production installer watchface containing exact payload pairs for
# Xiaomi Band 10 Pro firmware 3.101.030 and 3.101.036.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CANOPUS=${CANOPUS_ROOT:-"$ROOT/../Canopus"}
WATCHFACE="$ROOT/watchfaces/bluetooth-audio-prod"
mkdir -p "$WATCHFACE"

set -- \
  xiaomi-band-10-pro-3.101.030 \
  xiaomi-band-10-pro-3.101.036
for TARGET_ID do
  rm -f "$WATCHFACE/bluetooth-audio-$TARGET_ID.cmi"
done

cargo fmt --manifest-path "$ROOT/Cargo.toml" --all -- --check
cargo test --manifest-path "$ROOT/Cargo.toml" \
  -p canopus-bluetooth-audio-core --features production
lua "$ROOT/scripts/smoke-watchface-prod.lua" "$WATCHFACE/main.lua" >/dev/null

for TARGET_ID do
  OUT="$ROOT/build/bluetooth-audio-prod/$TARGET_ID"
  "$ROOT/scripts/build-install-payload.sh" "$TARGET_ID" "$OUT" production
  STEM="$WATCHFACE/bluetooth-audio-$TARGET_ID"
  cp "$OUT/bluetooth-audio.elf" "$STEM.bin"
  cp "$OUT/receipt.bin" "$STEM.cmi.bin"
done

python3 - "$ROOT" "$CANOPUS" "$WATCHFACE" \
  xiaomi-band-10-pro-3.101.030 \
  xiaomi-band-10-pro-3.101.036 <<'PY'
import hashlib, pathlib, struct, sys, tomllib
root = pathlib.Path(sys.argv[1])
canopus = pathlib.Path(sys.argv[2])
watchface = pathlib.Path(sys.argv[3])
targets = sys.argv[4:]
expected_files = set()
module_digests = set()
for target in targets:
    stem = watchface / f"bluetooth-audio-{target}"
    module_path = pathlib.Path(str(stem) + ".bin")
    receipt_path = pathlib.Path(str(stem) + ".cmi.bin")
    expected_files.update((module_path, receipt_path))
    module = module_path.read_bytes()
    receipt = receipt_path.read_bytes()
    assert len(module) >= 512 and len(module) <= 262144
    assert module[:7] == b"\x7fELF\x01\x01\x01"
    assert struct.unpack_from("<HH", module, 16) == (1, 40)
    for removed_label in (b"Play test tone", b"Play long MP3", b"Decode long MP3 only"):
        assert removed_label not in module, (target, removed_label)
    assert len(receipt) == 256 and receipt[:4] == b"CMI1"
    magic, version, header, _flags, lifecycle, module_version, artifact_size, _reserved = struct.unpack(
        "<8I", receipt[:32]
    )
    assert magic == 0x31494D43 and version == 1 and header == 256
    assert lifecycle in range(4) and module_version == 1
    assert artifact_size == len(module), (target, artifact_size, len(module))
    module_id = receipt[32:64].split(b"\0", 1)[0]
    receipt_target = receipt[64:112].split(b"\0", 1)[0].decode("ascii")
    receipt_firmware = receipt[112:144].hex()
    profile = tomllib.loads((canopus / "targets" / target / "target.toml").read_text())
    assert profile["target_id"] == target
    assert module_id == b"bluetooth_audio"
    assert receipt_target == target, (receipt_target, target)
    assert receipt_firmware == profile["firmware_sha256"]
    module_digest = hashlib.sha256(module).digest()
    assert receipt[144:176] == module_digest
    module_digests.add(module_digest)
assert len(module_digests) == len(targets), "target payloads must not be identical"
appicon = (watchface / "appicon_headphones.bin").read_bytes()
assert len(appicon) == 54768 and appicon[:4] == b"\x19\x10\0\0"
icon_width, icon_height, icon_stride, icon_reserved = struct.unpack_from("<4H", appicon, 4)
assert (icon_width, icon_height, icon_stride, icon_reserved) == (117, 117, 468, 0)
assert len(appicon) == 12 + icon_height * icon_stride
assert appicon == (root / "watchfaces" / "bluetooth-audio" / "appicon_headphones.bin").read_bytes()
actual_files = set(watchface.glob("bluetooth-audio-*.bin"))
assert actual_files == expected_files, (sorted(map(str, actual_files)), sorted(map(str, expected_files)))
assert not list(watchface.glob("bluetooth-audio-*.cmi"))
assert not (watchface / "module.bin").exists()
assert not (watchface / "receipt.bin").exists()
for path in watchface.rglob("*"):
    assert path.name not in {"long_test_audio.bin", "long_test_audio_stream.bin"}
print("production watchface staged OK: " + ", ".join(targets))
PY

echo "watchfaces/bluetooth-audio-prod is ready to install"
