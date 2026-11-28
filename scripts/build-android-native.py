"""Build the shared recorder for explicit Android ABIs using the pinned NDK."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "armeabi-v7a": ("armv7-linux-androideabi", "armv7a-linux-androideabi29"),
    "arm64-v8a": ("aarch64-linux-android", "aarch64-linux-android29"),
    "x86_64": ("x86_64-linux-android", "x86_64-linux-android29"),
}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--abis", default="armeabi-v7a,arm64-v8a")
    args = parser.parse_args()
    sdk = Path(os.environ.get("ANDROID_HOME") or os.environ["ANDROID_SDK_ROOT"])
    ndk = sdk / "ndk/28.2.13676358"
    host = "windows-x86_64" if os.name == "nt" else "linux-x86_64"
    clang = ndk / "toolchains/llvm/prebuilt" / host / "bin" / ("clang.exe" if os.name == "nt" else "clang")
    if not clang.is_file():
        parser.error("Install Android NDK 28.2.13676358")
    for abi in args.abis.split(","):
        if abi not in TARGETS:
            parser.error("Unsupported Android ABI")
        target, triple = TARGETS[abi]
        subprocess.run(["rustup", "target", "add", target], check=True)
        env = os.environ.copy()
        key = target.upper().replace("-", "_")
        env.pop("RUSTFLAGS", None)
        env.pop("CARGO_ENCODED_RUSTFLAGS", None)
        env[f"CARGO_TARGET_{key}_LINKER"] = str(clang)
        env[f"CARGO_TARGET_{key}_RUSTFLAGS"] = f"-C link-arg=--target={triple} -C link-arg=-Wl,-z,max-page-size=16384"
        subprocess.run(["cargo", "build", "--locked", "--release", "-p", "stutter-watch-android", "--target", target], cwd=ROOT, env=env, check=True)
        dest = ROOT / "android/app/build/generated/jniLibs" / abi
        dest.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / "target" / target / "release/libstutter_android.so", dest)

if __name__ == "__main__":
    main()
