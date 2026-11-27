"""Build a Windows x64 portable archive from an explicit public-file allowlist.

Requires Python 3.11+, the locked Rust toolchain, MSVC Build Tools and Windows SDK.
Run from any directory. Output defaults to target/dist; existing assets are refused.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"


def command(*args, **kwargs):
    return subprocess.check_output(args, cwd=ROOT, text=True, **kwargs).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/dist")
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("Windows MSVC build tools are required")
    if subprocess.run(["git", "diff", "--quiet", "HEAD", "--"], cwd=ROOT).returncode:
        parser.error("Commit tracked source changes before packaging")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text("utf-8"))["package"]["version"]
    commit = command("git", "rev-parse", "HEAD")
    name = f"stutter-watch-{version}-windows-x64"
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"{name}.zip"
    checksum = output / f"{name}.zip.sha256"
    if archive.exists() or checksum.exists():
        parser.error("Release output already exists; use a fresh output directory")

    # An explicit target keeps these flags away from host proc-macro crates.
    env = os.environ.copy()
    env["CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS"] = "-C target-feature=+crt-static"
    subprocess.run(["cargo", "build", "--locked", "--release", "--target", TARGET],
                   cwd=ROOT, env=env, check=True)
    binary = ROOT / "target" / TARGET / "release/stutter-watch.exe"
    data = binary.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if (data[pe:pe + 4] != b"PE\0\0" or
            struct.unpack_from("<H", data, pe + 4)[0] != 0x8664 or
            struct.unpack_from("<H", data, pe + 24 + 68)[0] != 2):
        raise RuntimeError("Expected a Windows x64 GUI executable")

    metadata = json.loads(command("cargo", "metadata", "--locked", "--format-version", "1",
                                  "--filter-platform", TARGET))
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = metadata["resolve"]["root"]
    included = set()
    pending = [root]
    while pending:
        key = pending.pop()
        if key in included:
            continue
        included.add(key)
        for dep in nodes[key]["deps"]:
            if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]):
                pending.append(dep["pkg"])

    with tempfile.TemporaryDirectory(prefix="stutter-watch-package-") as temporary:
        stage = Path(temporary) / name
        stage.mkdir()
        shutil.copy2(binary, stage / "stutter-watch.exe")
        # Never copy a live config, recordings, internal plans, or workspace trees.
        for relative in ["LICENSE", "THIRD_PARTY_NOTICES.md", "config.example.toml",
                         "docs/GETTING_STARTED.md", "docs/DATA_AND_PRIVACY.md",
                         "docs/ARCHITECTURE.md", "docs/ROADMAP.md"]:
            destination = stage / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / relative, destination)
        shutil.copy2(ROOT / "docs/WINDOWS_PORTABLE.md", stage / "README.md")
        inventory = []
        for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
            if package["id"] not in included or package["id"] == root:
                continue
            source = Path(package["manifest_path"]).parent
            notices = [p for p in source.iterdir() if p.is_file() and p.name.lower().startswith(
                ("license", "licence", "copying", "notice", "copyright"))]
            if package.get("license_file"):
                notices.append(source / package["license_file"])
            notices = sorted(set(notices))
            if not package["license"] or not notices:
                raise RuntimeError(f"Review missing licensing for {package['name']}")
            folder = Path("licenses") / f"{package['name']}-{package['version']}"
            (stage / folder).mkdir(parents=True)
            for notice in notices:
                shutil.copy2(notice, stage / folder / notice.name)
            inventory.append({"name": package["name"], "version": package["version"],
                              "license": package["license"], "repository": package["repository"],
                              "notices": [(folder / p.name).as_posix() for p in notices]})
        rust_docs = Path(command("rustc", "--print", "sysroot")) / "share/doc/rust"
        rust_folder = stage / "licenses/rust"
        rust_folder.mkdir()
        shutil.copy2(rust_docs / "COPYRIGHT-library.html", rust_folder)
        shutil.copytree(rust_docs / "licenses", rust_folder / "licenses")
        (stage / "licenses/dependencies.json").write_text(
            json.dumps(inventory, indent=2) + "\n", encoding="utf-8")
        build = {"product": "Stutter Watch", "version": version, "commit": commit,
                 "target": TARGET, "rustc": command("rustc", "--version"),
                 "crt": "static", "exe_sha256": hashlib.sha256(data).hexdigest(),
                 "license": "PolyForm-Noncommercial-1.0.0",
                 "source": f"https://github.com/flochiii/stutter-watch/tree/{commit}"}
        (stage / "BUILD.json").write_text(json.dumps(build, indent=2) + "\n", encoding="utf-8")
        # Registry archives can carry Unix-epoch mtimes, outside ZIP's date range.
        temporary_archive = Path(temporary) / archive.name
        with zipfile.ZipFile(temporary_archive, "x", compression=zipfile.ZIP_DEFLATED,
                             compresslevel=9, strict_timestamps=False) as bundle:
            for path in sorted(stage.rglob("*")):
                if path.is_file():
                    bundle.write(path, path.relative_to(stage.parent).as_posix())
        with archive.open("xb") as destination, temporary_archive.open("rb") as source:
            shutil.copyfileobj(source, destination)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum.write_text(f"{digest}  {archive.name}\n", encoding="ascii")
    print(json.dumps({"archive": str(archive), "sha256": digest,
                      "dependency_count": len(inventory), "commit": commit}))


if __name__ == "__main__":
    main()
