"""Reproduce the bundled OFL CJK subsets (requires fonttools).

Inputs are downloaded only when this maintainer script is run. The application
uses the committed subsets offline; no system font discovery is required.
"""
import hashlib
import json
from pathlib import Path
import urllib.request
from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[1]
COMMIT = "f8d157532fbfaeda587e826d4cd5b21a49186f7c"
SOURCES = {
    "sc": ("SimplifiedChinese", "2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b"),
    "jp": ("Japanese", "68a3fc98800b2a27b371f2fb79991daf3633bd89309d4ffaa6946fd587f375b5"),
}

def main():
    cache = ROOT / ".verification/tools/fonts"
    dest = ROOT / "assets/fonts"
    cache.mkdir(parents=True, exist_ok=True)
    dest.mkdir(parents=True, exist_ok=True)
    text = "".join(p.read_text("utf-8") for p in sorted((ROOT / "locales").glob("*.json")))
    text += (ROOT / "src/i18n.rs").read_text("utf-8")
    text += "".join(chr(c) for c in range(32, 127)) + "…—–−•ⓘ↑↓²³°µμ×≥≤"
    for code, (directory, digest) in SOURCES.items():
        source = cache / f"NotoSansCJK{code}-Regular.otf"
        url = f"https://raw.githubusercontent.com/notofonts/noto-cjk/{COMMIT}/Sans/OTF/{directory}/{source.name}"
        if not source.exists():
            urllib.request.urlretrieve(url, source)
        if hashlib.sha256(source.read_bytes()).hexdigest() != digest:
            raise RuntimeError("Upstream font checksum mismatch")
        font = TTFont(source, recalcTimestamp=False)
        options = subset.Options()
        options.recalc_timestamp = False
        options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
        options.name_legacy = True
        options.name_languages = ["*"]
        worker = subset.Subsetter(options=options)
        worker.populate(text=text)
        worker.subset(font)
        family = f"Stutter Watch UI CJK {code.upper()}"
        for record in font["name"].names:
            if record.nameID in (1, 3, 4, 6):
                value = family.replace(" ", "") if record.nameID == 6 else family
                record.string = value.encode(record.getEncoding())
        font.save(dest / f"ui-cjk-{code}.otf")
    license_file = cache / "OFL.txt"
    if not license_file.exists():
        urllib.request.urlretrieve(f"https://raw.githubusercontent.com/notofonts/noto-cjk/{COMMIT}/Sans/LICENSE", license_file)
    (dest / "OFL.txt").write_bytes(license_file.read_bytes())
    (dest / "README.md").write_text(
        f"# UI font subsets\n\nDerived from Noto Sans CJK SC and JP at `{COMMIT}`.\n"
        "Copyright and terms are in [OFL.txt](OFL.txt). Modified font families are named Stutter Watch UI CJK.\n\n"
        "Reproduce with `python scripts/subset-fonts.py` after installing fonttools. "
        "Only characters needed by the shipped UI catalogs are included.\n\n"
        + "\n".join(f"- `{code}` upstream SHA256: `{data[1]}`" for code, data in SOURCES.items()) + "\n",
        encoding="utf-8")

if __name__ == "__main__":
    main()
