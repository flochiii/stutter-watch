"""Native Linux desktop smoke test. Run inside dbus-run-session + xvfb-run.

Screenshots contain only data generated on the disposable CI machine.
"""
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = Path(os.environ.get("STUTTER_WATCH_TEST_OUTPUT", ROOT / "target/linux-evidence")).resolve()
EXE = Path(os.environ.get("STUTTER_WATCH_TEST_EXECUTABLE", ROOT / "target/release/stutter-watch")).resolve()

def run(*args, **kw):
    return subprocess.run([str(x) for x in args], check=True, **kw)

def main():
    OUT.mkdir(parents=True, exist_ok=True)
    config = OUT / "config.toml"
    base = [EXE, "--config", config]
    # Initial config creation is deliberately isolated from XDG user state.
    run(*base, "status", stdout=subprocess.DEVNULL)
    config.write_text(config.read_text().replace('language = "en"', 'language = "en"'))
    manager = subprocess.Popen(["openbox"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    gui = subprocess.Popen([str(x) for x in base], stdout=open(OUT / "gui.log", "w"), stderr=subprocess.STDOUT)
    window = None
    try:
        deadline = time.monotonic() + 35
        while time.monotonic() < deadline:
            result = subprocess.run(["xdotool", "search", "--name", "^Stutter Watch$"], capture_output=True, text=True)
            if result.returncode == 0 and result.stdout.strip():
                window = result.stdout.strip().splitlines()[-1]
                break
            if gui.poll() is not None:
                raise RuntimeError("Linux GUI exited before showing a window")
            time.sleep(.5)
        assert window, "No Linux app window appeared"
        run("xdotool", "windowactivate", "--sync", window)
        time.sleep(18)
        def click(x, y):
            run("xdotool", "mousemove", "--window", window, x, y, "click", "1")
            time.sleep(1)
        def screenshot(name):
            run("import", "-window", window, OUT / name)
        screenshot("linux-monitor.png")
        click(194, 34)
        screenshot("linux-data.png")
        click(434, 34)
        screenshot("linux-tray.png")
        run(*base, "mark")
        time.sleep(12)
        click(314, 34)
        screenshot("linux-incidents.png")
        # Closing the GUI must keep the independent recorder alive.
        run("xdotool", "windowactivate", "--sync", window, "key", "alt+F4")
        gui.wait(timeout=10)
        time.sleep(2)
        status = json.loads(subprocess.check_output([str(x) for x in base] + ["status"]))
        (OUT / "status.json").write_text(json.dumps(status, indent=2))
        run(*base, "report", "--hours", "1", stdout=subprocess.DEVNULL)
        csv = OUT / "synthetic-mangohud.csv"
        csv.write_text("fps,frametime,elapsed\n" + "".join(
            f"{1000/(130 if i==80 else 16.6):.3f},{130 if i==80 else 16.6},{i*100000000}\n" for i in range(200)))
        imported = subprocess.check_output([str(x) for x in base] + ["import-mangohud", str(csv)], text=True).strip()
        assert Path(imported).exists()
        # Reopening restores the existing history, rather than starting at zero.
        gui = subprocess.Popen([str(x) for x in base], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(5)
        windows = subprocess.check_output(["xdotool", "search", "--name", "^Stutter Watch$"], text=True).splitlines()
        window = windows[-1]
        screenshot("linux-history-restored.png")
        click(314, 34)
        screenshot("linux-mangohud.png")
        run("xdotool", "windowactivate", "--sync", window, "key", "alt+F4")
        gui.wait(timeout=10)
        run(*base, "shutdown")
        time.sleep(2)
        # Verify non-Latin glyphs without depending on machine-installed fonts.
        config.write_text(config.read_text().replace('language = "en"', 'language = "ja"'))
        gui = subprocess.Popen([str(x) for x in base], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(6)
        window = subprocess.check_output(["xdotool", "search", "--name", "^Stutter Watch$"], text=True).splitlines()[-1]
        click(194, 34)
        screenshot("linux-japanese.png")
        (OUT / "result.json").write_text(json.dumps({"desktop": "X11 / Xvfb / Mesa", "gui_close_keeps_recording": True,
            "history_reopened": True, "mangohud_import": True, "report": True, "screenshots": sorted(p.name for p in OUT.glob("*.png"))}, indent=2))
    finally:
        if gui.poll() is None:
            gui.terminate()
            gui.wait(timeout=10)
        subprocess.run([str(x) for x in base] + ["shutdown"], capture_output=True)
        manager.terminate()

if __name__ == "__main__":
    main()
