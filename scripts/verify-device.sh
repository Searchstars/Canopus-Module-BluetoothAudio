#!/bin/sh
set -eu
ELF=${1:?usage: verify-device.sh path/to/module.elf}

# The firmware loader maps only SHF_ALLOC sections into module memory (the
# loaded image); the file also carries symtab/strtab/shstrtab/relocations that
# are never loaded. Measure the loaded image, not the file size.
LOADED=$(python3 - "$ELF" <<'PY'
import struct, sys
data = open(sys.argv[1], 'rb').read()
e_shoff = struct.unpack_from('<I', data, 0x20)[0]
e_shentsize = struct.unpack_from('<H', data, 0x2e)[0]
e_shnum = struct.unpack_from('<H', data, 0x30)[0]
loaded = 0
for i in range(e_shnum):
    off = e_shoff + i * e_shentsize
    flags = struct.unpack_from('<I', data, off + 0x8)[0]
    size = struct.unpack_from('<I', data, off + 0x14)[0]
    if flags & 0x2:  # SHF_ALLOC
        loaded += size
print(loaded)
PY
)

if [ "$LOADED" -gt 65536 ]; then
  echo "module exceeds target max_size: $LOADED > 65536 (loaded image)" >&2
  exit 1
fi

NM=${NM:-nm}
if "$NM" -u "$ELF" | grep -q .; then
  echo "module has undefined imports:" >&2
  "$NM" -u "$ELF" >&2
  exit 1
fi
file "$ELF"
echo "verified module loaded size: $LOADED bytes (limit 65536)"