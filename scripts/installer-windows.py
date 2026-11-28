"""Create a per-user installer from the verified portable package."""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
INNO_SHA256 = "a549929319f9120716677c685535ca4671e4b937b5fdc87e2585f32d8eb30e8c"
INNO_URL = "https://github.com/jrsoftware/issrc/releases/download/is-6_7_4/innosetup-6.7.4.exe"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    args = parser.parse_args()
    output = args.archive.resolve().parent
    tools = ROOT / ".verification/tools/inno-6.7.4"
    compiler = tools / "compiler/ISCC.exe"
    if not compiler.exists():
        tools.mkdir(parents=True, exist_ok=True)
        download = tools / "innosetup-6.7.4.exe"
        if not download.exists():
            urllib.request.urlretrieve(INNO_URL, download)
        if hashlib.sha256(download.read_bytes()).hexdigest() != INNO_SHA256:
            raise RuntimeError("Inno Setup compiler download checksum mismatch")
        subprocess.run([str(download), "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART",
                        "/CURRENTUSER", "/NOICONS", f"/DIR={compiler.parent}"], check=True)
    with tempfile.TemporaryDirectory(prefix="stutter-watch-installer-") as temporary:
        with zipfile.ZipFile(args.archive) as archive:
            archive.extractall(temporary)
        package = next(Path(temporary).glob("*/BUILD.json")).parent
        import json
        build = json.loads((package / "BUILD.json").read_text("utf-8"))
        if hashlib.sha256((package / "stutter-watch.exe").read_bytes()).hexdigest() != build["exe_sha256"]:
            raise RuntimeError("Portable executable checksum mismatch")
        # The setup stub retains Inno's notices, with a copy in the installed docs.
        (package / "licenses/installer").mkdir()
        for source in compiler.parent.glob("license*"):
            if source.is_file():
                import shutil
                shutil.copy2(source, package / "licenses/installer" / source.name)
        subprocess.run([str(compiler), f"/DAppVersion={build['version']}",
                        f"/DPackageDir={package}", f"/DOutputDir={output}",
                        str(ROOT / "packaging/windows/setup.iss")], check=True)
    installer = output / f"stutter-watch-{build['version']}-windows-x64-setup.exe"
    digest = hashlib.sha256(installer.read_bytes()).hexdigest()
    installer.with_suffix(".exe.sha256").write_text(f"{digest}  {installer.name}\n", encoding="ascii")
    print(f"Created {installer.name} SHA256 {digest}")


if __name__ == "__main__":
    main()
