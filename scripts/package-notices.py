"""Dependency notices for a target-specific, frozen Cargo build."""
import json
from pathlib import Path
import shutil
import subprocess
import hashlib

def inventory(root, stage, target):
    def command(*args):
        return subprocess.check_output(args, cwd=root, text=True).strip()
    metadata = json.loads(command("cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target))
    # Cargo metadata also lists resolved optional crates which were not compiled.
    active = set()
    for line in command("cargo", "tree", "--locked", "--target", target, "--edges", "normal,build", "--prefix", "none", "--format", "{p}").splitlines():
        parts=line.split()
        if len(parts)>=2: active.add((parts[0],parts[1].removeprefix("v")))
    upstream_path=root / "packaging/licenses/upstream.json"
    upstream=json.loads(upstream_path.read_text("utf-8")) if upstream_path.exists() else []
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
        if (package["name"],package["version"]) not in active: continue
        source = Path(package["manifest_path"]).parent
        notices = [p for p in source.rglob("*") if p.is_file() and (p.name.lower().startswith(
            ("license", "licence", "copying", "notice", "copyright", "unlicense", "ofl", "ufl"))
            or (package["name"]=="epaint_default_fonts" and p.suffix==".txt"))]
        if package.get("license_file"):
            notices.append(source / package["license_file"])
        notices = sorted(set(notices))
        extra=[]
        for entry in upstream:
            if (entry["name"],entry["version"]) != (package["name"],package["version"]):continue
            for file in entry["files"]:
                path=root / "packaging/licenses" / f"{entry['name']}-{entry['version']}" / file["name"]
                if hashlib.sha256(path.read_bytes()).hexdigest()!=file["sha256"]:raise RuntimeError("Upstream notice checksum mismatch")
                extra.append(path)
        if not package["license"] or not notices+extra:
            raise RuntimeError(f"Review missing licensing for {package['name']}")
        folder = Path("licenses") / f"{package['name']}-{package['version']}"
        (stage / folder).mkdir(parents=True)
        for notice in notices:
            destination=stage / folder / notice.relative_to(source)
            destination.parent.mkdir(parents=True,exist_ok=True)
            shutil.copy2(notice,destination)
        for notice in extra:shutil.copy2(notice,stage / folder / notice.name)
        result.append({"name": package["name"], "version": package["version"], "license": package["license"],
                       "repository": package["repository"], "notices": [(folder / p.relative_to(source)).as_posix() for p in notices]+[(folder / p.name).as_posix() for p in extra]})
    rust_docs = Path(command("rustc", "--print", "sysroot")) / "share/doc/rust"
    rust_folder = stage / "licenses/rust"
    rust_folder.mkdir(parents=True)
    shutil.copy2(rust_docs / "COPYRIGHT-library.html", rust_folder)
    shutil.copytree(rust_docs / "licenses", rust_folder / "licenses")
    shutil.copytree(root / "assets/fonts", stage / "licenses/fonts", ignore=shutil.ignore_patterns("*.otf", "*.ttf"))
    (stage / "licenses/dependencies.json").write_text(json.dumps(result, indent=2)+"\n", encoding="utf-8")
    return result
