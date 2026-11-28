"""Build explicit-allowlist Linux packages from a committed source checkout."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
try:
    import tomllib
except ImportError:
    import tomli as tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-unknown-linux-gnu"
RUNTIME_URL = "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64"
RUNTIME_SHA = "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"

def run(*args, **kw):
    return subprocess.run([str(x) for x in args], cwd=ROOT, check=True, **kw)

def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/dist-linux")
    args = parser.parse_args()
    if command("uname", "-m") != "x86_64":
        parser.error("Build on Linux x86_64")
    run("git", "diff", "--exit-code", "HEAD", "--")
    commit = command("git", "rev-parse", "HEAD")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text("utf-8"))["package"]["version"]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        parser.error("Use a fresh output directory")
    run("cargo", "build", "--locked", "--release")
    run("git", "diff", "--exit-code", "HEAD", "--")
    assert commit == command("git", "rev-parse", "HEAD"), "Source changed during build"
    exe = ROOT / "target/release/stutter-watch"
    spec = importlib.util.spec_from_file_location("package_notices", ROOT / "scripts/package-notices.py")
    notices = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(notices)
    with tempfile.TemporaryDirectory(prefix="stutter-watch-linux-") as temporary:
        work = Path(temporary)
        package = work / "package"
        binary = package / "usr/bin/stutter-watch"
        binary.parent.mkdir(parents=True)
        shutil.copy2(exe, binary)
        binary.chmod(0o755)
        desktop = package / "usr/share/applications/io.github.flochiii.StutterWatch.desktop"
        desktop.parent.mkdir(parents=True)
        shutil.copy2(ROOT / "packaging/linux" / desktop.name, desktop)
        icon = package / "usr/share/icons/hicolor/scalable/apps/io.github.flochiii.StutterWatch.svg"
        icon.parent.mkdir(parents=True)
        shutil.copy2(ROOT / "assets/app.svg", icon)
        docs = package / "usr/share/doc/stutter-watch"
        docs.mkdir(parents=True)
        for path in ["LICENSE", "THIRD_PARTY_NOTICES.md", "docs/LINUX.md", "docs/DATA_AND_PRIVACY.md", "config.example.toml"]:
            shutil.copy2(ROOT / path, docs / Path(path).name)
        inventory = notices.inventory(ROOT, docs, TARGET)
        build = {"product": "Stutter Watch", "version": version, "commit": commit, "target": TARGET,
                 "rustc": command("rustc", "--version"), "exe_sha256": hashlib.sha256(exe.read_bytes()).hexdigest(),
                 "glibc_minimum": "2.35", "license": "PolyForm-Noncommercial-1.0.0",
                 "source": f"https://github.com/flochiii/stutter-watch/tree/{commit}", "dependencies": len(inventory)}
        (docs / "BUILD.json").write_text(json.dumps(build, indent=2)+"\n")
        # Set package modes explicitly: application runtime uses umask 077,
        # whereas distro payloads must be readable by all local users.
        for path in package.rglob("*"):
            path.chmod(0o755 if path.is_dir() or path == binary else 0o644)
        deb = work / "deb"
        shutil.copytree(package, deb)
        (deb / "DEBIAN").mkdir()
        (deb / "DEBIAN/control").write_text(
            f"Package: stutter-watch\nVersion: {version}\nArchitecture: amd64\nMaintainer: flochiii\n"
            "Section: utils\nPriority: optional\nHomepage: https://github.com/flochiii/stutter-watch\n"
            "Depends: libc6 (>= 2.35), libgcc-s1, libx11-6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libegl1, libgl1\n"
            "Description: Local desktop stutter recorder and incident charts\n"
            " Preserves selected performance history and protected bookmarks.\n")
        run("dpkg-deb", "--root-owner-group", "--build", deb, output / f"stutter-watch_{version}_amd64.deb")
        rpmdir = work / "rpm"
        for name in ["BUILD", "BUILDROOT", "RPMS", "SOURCES", "SPECS", "SRPMS"]:
            (rpmdir / name).mkdir(parents=True)
        rpm_spec = rpmdir / "SPECS/stutter-watch.spec"
        rpm_spec.write_text(f"""Name: stutter-watch
Version: {version}
Release: 1
Summary: Local desktop stutter recorder and incident charts
License: PolyForm-Noncommercial-1.0.0
URL: https://github.com/flochiii/stutter-watch
BuildArch: x86_64
Requires: glibc >= 2.35, libX11, libxkbcommon, libxkbcommon-x11, wayland-libs, libglvnd-egl, libglvnd-glx
%description
Preserves selected performance history and protected bookmarks locally.
%install
mkdir -p %{{buildroot}}
cp -a '{package}/.' %{{buildroot}}/
%files
/usr/bin/stutter-watch
/usr/share/applications/io.github.flochiii.StutterWatch.desktop
/usr/share/icons/hicolor/scalable/apps/io.github.flochiii.StutterWatch.svg
/usr/share/doc/stutter-watch
""")
        run("rpmbuild", "--define", f"_topdir {rpmdir}", "--define", "_build_id_links none", "--define", "__os_install_post %{nil}", "-bb", rpm_spec)
        shutil.copy2(next((rpmdir / "RPMS").rglob("*.rpm")), output)
        appdir = work / "StutterWatch.AppDir"
        shutil.copytree(package, appdir)
        shutil.copy2(desktop, appdir / desktop.name)
        shutil.copy2(icon, appdir / icon.name)
        (appdir / ".DirIcon").symlink_to(icon.name)
        (appdir / "AppRun").write_text('#!/bin/sh\nHERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\nexec "$HERE/usr/bin/stutter-watch" "$@"\n')
        (appdir / "AppRun").chmod(0o755)
        runtime = work / "runtime"
        urllib.request.urlretrieve(RUNTIME_URL, runtime)
        assert hashlib.sha256(runtime.read_bytes()).hexdigest() == RUNTIME_SHA, "AppImage runtime checksum mismatch"
        runtime_license = appdir / "usr/share/doc/stutter-watch/licenses/appimage-runtime"
        runtime_license.mkdir()
        urllib.request.urlretrieve("https://raw.githubusercontent.com/AppImage/type2-runtime/20251108/LICENSE", runtime_license / "LICENSE")
        (runtime_license / "SOURCE.txt").write_text("Unmodified type2-runtime 20251108.\nhttps://github.com/AppImage/type2-runtime/tree/20251108\n" + RUNTIME_URL + "\nSHA256 " + RUNTIME_SHA + "\n")
        squash = work / "payload.squashfs"
        run("mksquashfs", appdir, squash, "-root-owned", "-noappend", "-comp", "gzip", "-quiet")
        appimage = output / f"stutter-watch-{version}-linux-x64.AppImage"
        with appimage.open("xb") as destination:
            destination.write(runtime.read_bytes())
            with squash.open("rb") as source:
                shutil.copyfileobj(source, destination)
        appimage.chmod(0o755)
        # A conventional archive supports inspection and non-desktop workflows.
        with tarfile.open(output / f"stutter-watch-{version}-linux-x64.tar.gz", "w:gz") as archive:
            archive.add(package, arcname=f"stutter-watch-{version}")
    for path in sorted(output.iterdir()):
        if path.is_file() and not path.name.endswith(".sha256"):
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            path.with_name(path.name+".sha256").write_text(f"{digest}  {path.name}\n", encoding="ascii")
    print(json.dumps(build))

if __name__ == "__main__":
    main()
