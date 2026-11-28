"""Validate source identity for tagged releases and explicitly marked candidates."""
import json
import os
from pathlib import Path
import subprocess

root=Path(__file__).resolve().parents[1]
metadata=json.loads(subprocess.check_output(["cargo","metadata","--locked","--no-deps","--format-version","1"],cwd=root))
version=next(p["version"] for p in metadata["packages"] if p["name"]=="stutter-watch")
candidate=os.environ.get("CANDIDATE", "false")=="true"
if os.environ.get("GITHUB_EVENT_NAME")=="push" and os.environ.get("GITHUB_REF_NAME")!="v"+version:
    raise RuntimeError("Tag must match the Cargo package version")
if not candidate:
    subprocess.run(["git","merge-base","--is-ancestor","HEAD","origin/main"],cwd=root,check=True)
if not (root/f"docs/releases/v{version}.md").is_file():
    raise RuntimeError("Release notes are missing")
print(json.dumps({"version":version,"candidate":candidate,"commit":subprocess.check_output(["git","rev-parse","HEAD"],cwd=root,text=True).strip()}))
