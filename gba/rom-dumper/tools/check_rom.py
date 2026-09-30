"""Validate a devkitPro multiboot image, optionally padding it to 16 bytes."""

import argparse
from pathlib import Path
import struct
import zlib


def check(data: bytes) -> None:
    if not 400 <= len(data) <= 256 * 1024 or len(data) % 16:
        raise ValueError("multiboot size must be 400..262144 bytes, aligned to 16")
    if data[0xB2] != 0x96:
        raise ValueError("missing GBA header fixed byte")
    if zlib.crc32(data[4:0xA0]) != 0xD0BEB55E:
        raise ValueError("invalid GBA logo; run devkitPro gbafix")
    if (sum(data[0xA0:0xBE]) + 0x19) & 0xFF:
        raise ValueError("invalid GBA header checksum")
    for offset in (0, 0xC0):
        instruction = struct.unpack_from("<I", data, offset)[0]
        if instruction >> 24 != 0xEA:
            raise ValueError(f"missing ARM entry branch at {offset:#x}")
        displacement = instruction & 0xFFFFFF
        if displacement & 0x800000:
            displacement -= 0x1000000
        target = offset + 8 + displacement * 4
        if not 0xC0 <= target < len(data):
            raise ValueError(f"entry branch at {offset:#x} leaves the image")
    if data[0xC4:0xC6] != b"\0\0":
        raise ValueError("BIOS boot-mode/client-ID bytes must be reserved")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pad", action="store_true")
    parser.add_argument("rom", type=Path)
    args = parser.parse_args()
    data = args.rom.read_bytes()
    if args.pad:
        size = (max(400, len(data)) + 15) & ~15
        data += bytes(size - len(data))
    try:
        check(data)
    except ValueError as error:
        parser.exit(1, f"{args.rom}: {error}\n")
    if args.pad:
        args.rom.write_bytes(data)
    print(f"{args.rom}: valid multiboot layout, {len(data)} bytes")


if __name__ == "__main__":
    main()
