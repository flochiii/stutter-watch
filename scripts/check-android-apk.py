"""Check the packaged Android native ABI and 16 KiB load-segment alignment."""
import argparse
import json
from pathlib import Path
import struct
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk", type=Path)
    parser.add_argument("--abis", required=True)
    args = parser.parse_args()
    expected = set(args.abis.split(","))
    machines = {"armeabi-v7a": (1, 40), "arm64-v8a": (2, 183), "x86_64": (2, 62)}
    with zipfile.ZipFile(args.apk) as apk:
        native = [name for name in apk.namelist() if name.startswith("lib/") and name.endswith(".so")]
        assert {name.split("/")[1] for name in native} == expected, "Unexpected APK architectures"
        for abi in expected:
            assert f"lib/{abi}/libstutter_android.so" in native, "Missing recorder"
        for name in native:
            data = apk.read(name)
            abi = name.split("/")[1]
            elf_class, machine = machines[abi]
            assert data[:4] == b"\x7fELF" and data[4:6] == bytes([elf_class, 1]), name
            assert struct.unpack_from("<H", data, 18)[0] == machine, name
            if elf_class == 2:
                offset = struct.unpack_from("<Q", data, 32)[0]
                size, count = struct.unpack_from("<HH", data, 54)
                for index in range(count):
                    header = offset + size * index
                    if struct.unpack_from("<I", data, header)[0] == 1:
                        assert struct.unpack_from("<Q", data, header + 48)[0] >= 16384, f"4 KiB native library: {name}"
        assert "assets/mobile/en.json" in apk.namelist()
        assert "assets/LICENSE" in apk.namelist()
    print(json.dumps({"apk": args.apk.name, "abis": sorted(expected), "native_libraries": len(native), "page_alignment": "passed"}))


if __name__ == "__main__":
    main()
