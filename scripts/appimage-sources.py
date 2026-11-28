"""Stage notices and corresponding runtime/library source for AppImage distribution."""
import hashlib
import json
from pathlib import Path
import shutil
import tarfile
import tempfile
import urllib.request

ROOT=Path(__file__).resolve().parents[1]

def stage_notices(destination, output, version):
    source=ROOT/"packaging/appimage"
    manifest=json.loads((source/"upstream.json").read_text("utf-8"))
    destination.mkdir(parents=True)
    for entry in manifest["notices"]:
        path=source/entry["name"]
        if hashlib.sha256(path.read_bytes()).hexdigest()!=entry["sha256"]:
            raise RuntimeError("AppImage notice checksum mismatch")
        shutil.copy2(path,destination/path.name)
    shutil.copy2(source/"upstream.json",destination/"upstream.json")
    name=f"stutter-watch-{version}-appimage-runtime-sources.tar.gz"
    archive_path=output/name
    with tempfile.TemporaryDirectory(prefix="stutter-watch-runtime-sources-") as temporary:
        stage=Path(temporary)
        for entry in manifest["sources"]:
            path=stage/entry["name"]
            urllib.request.urlretrieve(entry["url"],path)
            if hashlib.sha256(path.read_bytes()).hexdigest()!=entry["sha256"]:
                raise RuntimeError("AppImage corresponding-source checksum mismatch")
        shutil.copy2(source/"README.md",stage/"README.md")
        shutil.copy2(source/"upstream.json",stage/"upstream.json")
        for entry in manifest["notices"]:shutil.copy2(source/entry["name"],stage/entry["name"])
        with tarfile.open(archive_path,"w:gz") as archive:
            for path in sorted(stage.iterdir()):archive.add(path,arcname=path.name)
    digest=hashlib.sha256(archive_path.read_bytes()).hexdigest()
    (destination/"SOURCE.txt").write_text(
        "Unmodified AppImage type2-runtime 20251108.\n"
        "Runtime binary SHA256: "+manifest["runtime_binary_sha256"]+"\n"
        "Corresponding runtime, patched libfuse and squashfuse source/build recipes are distributed beside the AppImage:\n"
        +name+"\nSHA256: "+digest+"\n"
        "The original runtime archive includes the libfuse mount.c patch and Docker/build instructions.\n"
        "Other bundled native-library notices are included here. Stutter Watch is a separately executed application.\n",
        encoding="utf-8")
    return manifest["runtime_binary_sha256"]
