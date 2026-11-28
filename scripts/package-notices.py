"""Dependency notices for a target-specific, frozen Cargo build."""
import json
from pathlib import Path
import shutil
import subprocess

def inventory(root, stage, target):
    def command(*args):
        return subprocess.check_output(args, cwd=root, text=True).strip()
    metadata = json.loads(command("cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target))
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    package_root = metadata["resolve"]["root"]
    included, pending = set(), [package_root]
    while pending:
        key = pending.pop()
        if key in included:
            continue
        included.add(key)
        for dep in nodes[key]["deps"]:
            if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]):
                pending.append(dep["pkg"])
    result = []
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] not in included or package["id"] == package_root:
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
        result.append({"name": package["name"], "version": package["version"], "license": package["license"],
                       "repository": package["repository"], "notices": [(folder / p.name).as_posix() for p in notices]})
    rust_docs = Path(command("rustc", "--print", "sysroot")) / "share/doc/rust"
    rust_folder = stage / "licenses/rust"
    rust_folder.mkdir(parents=True)
    shutil.copy2(rust_docs / "COPYRIGHT-library.html", rust_folder)
    shutil.copytree(rust_docs / "licenses", rust_folder / "licenses")
    shutil.copytree(root / "assets/fonts", stage / "licenses/fonts", ignore=shutil.ignore_patterns("*.otf"))
    (stage / "licenses/dependencies.json").write_text(json.dumps(result, indent=2)+"\n", encoding="utf-8")
    return result
