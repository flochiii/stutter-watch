"""Measure a native recorder for 30 minutes without a desktop or network I/O."""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("executable",type=Path)
    p.add_argument("output",type=Path)
    p.add_argument("--seconds",type=int,default=1800)
    args=p.parse_args()
    args.output.mkdir(parents=True,exist_ok=True)
    exe=args.executable.resolve()
    config=(args.output/"config.toml").resolve()
    start=time.monotonic()
    process=subprocess.Popen([str(exe),"--config",str(config),"--self-test",str(args.seconds),"--no-tray"],stdout=open(args.output/"recorder.log","w"),stderr=subprocess.STDOUT)
    measurements=[]
    previous=None
    try:
        while process.poll() is None:
            try:
                stat=Path(f"/proc/{process.pid}/stat").read_text().rsplit(")",1)[1].split()
                ticks=int(stat[11])+int(stat[12])
                now=time.monotonic()
                rss=int(stat[21])*os.sysconf("SC_PAGE_SIZE")
                cpu=None if previous is None else (ticks-previous[1])/os.sysconf("SC_CLK_TCK")/(now-previous[0])*100
                measurements.append({"seconds":round(now-start,2),"rss_bytes":rss,"cpu_one_core_pct":cpu,"fds":len(list(Path(f"/proc/{process.pid}/fd").iterdir()))})
                previous=(now,ticks)
            except FileNotFoundError:
                break
            time.sleep(5)
        assert process.wait(timeout=30)==0,"Recorder failed its self-test"
        receipt=json.loads((config.parent/"logs/self-test-last.json").read_text())
        assert receipt["malformed_records"]==0 and receipt["omitted_jobs"]==0 and not receipt["recording_error"]
        warm=[m for m in measurements if m["seconds"]>=min(600,args.seconds/3)]
        summary={"duration_seconds":round(time.monotonic()-start,2),"mode":"recorder, no GUI/tray; default sources; disposable CI VM",
                 "cpu_definition":"100% equals one logical CPU", "sample_count":len(measurements),"self_test":receipt,
                 "peak_rss_bytes":max(m["rss_bytes"] for m in measurements),
                 "warm_median_rss_bytes":statistics.median(m["rss_bytes"] for m in warm),
                 "warm_median_cpu_one_core_pct":statistics.median(m["cpu_one_core_pct"] for m in warm if m["cpu_one_core_pct"] is not None),
                 "warm_peak_fds":max(m["fds"] for m in warm)}
        (args.output/"resources.json").write_text(json.dumps({"summary":summary,"measurements":measurements},indent=2))
        print(json.dumps(summary,indent=2))
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=30)

if __name__=="__main__":
    main()
