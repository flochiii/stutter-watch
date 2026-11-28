"""Run the app as a native Wayland client inside a nested Weston compositor.

The compositor uses Xvfb solely as its display; the app has no DISPLAY variable.
"""
import json
import os
from pathlib import Path
import subprocess
import time

ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/"target/linux-evidence/wayland"

def main():
    OUT.mkdir(parents=True,exist_ok=True)
    runtime=OUT/"runtime"
    runtime.mkdir(exist_ok=True,mode=0o700)
    runtime.chmod(0o700)
    env=os.environ.copy()
    env["XDG_RUNTIME_DIR"]=str(runtime)
    log=open(OUT/"weston.log","w")
    compositor=subprocess.Popen(["weston","--backend=x11-backend.so","--socket=sw-wayland","--width=1280","--height=960","--use-pixman","--idle-time=0","--no-config"],env=env,stdout=log,stderr=subprocess.STDOUT)
    gui=None
    base=[str(ROOT/"target/release/stutter-watch"),"--config",str(OUT/"config.toml")]
    try:
        deadline=time.monotonic()+20
        while not (runtime/"sw-wayland").exists():
            assert compositor.poll() is None,"Weston failed to start"
            assert time.monotonic()<deadline,"Wayland socket did not appear"
            time.sleep(.2)
        env.pop("DISPLAY",None)
        env["WAYLAND_DISPLAY"]="sw-wayland"
        gui=subprocess.Popen(base,env=env,stdout=open(OUT/"gui.log","w"),stderr=subprocess.STDOUT)
        time.sleep(18)
        assert gui.poll() is None,"Native Wayland GUI exited"
        # Only the compositor needs the parent X11 display. The screenshot is a
        # real nested Wayland desktop, not an X11 fallback for the application.
        subprocess.run(["import","-window","root",str(OUT/"linux-wayland.png")],check=True)
        subprocess.run(base+["mark"],env=env,check=True)
        subprocess.run(base+["shutdown"],env=env,check=True)
        (OUT/"result.json").write_text(json.dumps({"backend":"native Wayland / nested Weston / Mesa","application_display_env":False,"gui_alive_after_seconds":18,"manual_mark":True,"controlled_shutdown":True},indent=2))
    finally:
        if gui is not None and gui.poll() is None:
            gui.terminate()
            gui.wait(timeout=10)
        subprocess.run(base+["shutdown"],env=env,capture_output=True)
        compositor.terminate()
        compositor.wait(timeout=10)

if __name__=="__main__":main()
