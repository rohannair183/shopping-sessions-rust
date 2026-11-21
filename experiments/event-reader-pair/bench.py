"""Compare readers on the same warm input, one program at a time."""
import csv
import os
from pathlib import Path
import re
import subprocess
import sys

root = Path.cwd()
work = root / "runs/event-pair-9217386"
data = root / "data/practice-seed7"
runner = root / "experiments/report-benchmarks/time-release.sh"
order = [
    ("base","scan","grouped"), ("grouped","scan","base"),
    ("scan","grouped","base"), ("base","grouped","scan"),
    ("grouped","base","scan"), ("scan","base","grouped"),
]
fields = ["round","position","variant","reader","elapsed_s","event_read_s","click_read_s",
          "event_scan_and_route_s","setup_s","input_and_dedup_s","reports_write_s","peak_bytes","records"]
def get(pattern, log):
    m = re.search(pattern, log)
    if not m:
        raise ValueError(f"Missing {pattern}: {log[-700:]}")
    return m[1]

def trial(mode, variant, i, j, series=""):
    image = "shopping-events:9217386-" + variant
    name = f"{series}{mode}-{i}-{j}-{variant}"
    log = work / "logs" / (name + ".log")
    row = dict(round=i, position=j, variant=variant, reader=mode)
    if mode == "full":
        command = ["sh", str(runner), str(data), str(work/name)]
    else:
        command = ["docker","run","--rm","--platform","linux/amd64","--network","none","--read-only",
                   "--user","1000:1000","--cpuset-cpus","0-5","--memory","8g","--memory-swap","8g",
                   "--cap-drop","ALL","--security-opt","no-new-privileges","--pids-limit","4096",
                   "--tmpfs","/output:rw,size=1m,mode=1777","--mount",f"type=bind,src={data},dst=/data,readonly",
                   "-e","NCPU=6","-e","DATA_DIR=/data","-e","OUTPUT_DIR=/output","-e",f"READER_ONLY={mode}",image]
    with log.open("x") as out:
        subprocess.run(command, stdout=out,stderr=subprocess.STDOUT,env=dict(os.environ, IMAGE=image),check=True)
    body = log.read_text()
    if mode == "full":
        row["elapsed_s"] = int(get(r"elapsed_ns=(\d+)",body))/1e9
        row["peak_bytes"] = int(get(r"cgroup_peak_bytes=(\d+)",body))
        m=re.search(r"2242393 shopper rows, 28014 campaign rows; lookups ([\d.]+)s, input and dedup ([\d.]+)s, reports and write ([\d.]+)s",body)
        if not m: raise ValueError(f"Unexpected output counts in {log}")
        row.update(zip(("setup_s","input_and_dedup_s","reports_write_s"),map(float,m.groups())))
        for key in ("event_read_s","click_read_s","event_scan_and_route_s"):
            row[key] = float(get(key+r"=([\d.]+)",body))
    else:
        row["elapsed_s"] = float(get(mode+r"_read_s=([\d.]+)",body))
        row["records"] = int(get(r"records=(\d+)",body))
        expected = {"event":21582714,"click":26201714-21118908}
        if row["records"] != expected[mode]: raise ValueError(f"Unexpected records in {log}: {row}")
    return row

if __name__ == "__main__":
    phase = sys.argv[1]
    if phase == "combined":
        for pos,v in enumerate(("grouped","combined"),1):
            trial("full",v,0,pos,"stack-")
        with (work/"combined-times.csv").open("x") as dest:
            writer=csv.DictWriter(dest,fieldnames=fields,lineterminator="\n")
            writer.writeheader()
            seq=[("grouped","combined"),("combined","grouped"),("combined","grouped"),
                 ("grouped","combined"),("grouped","combined"),("combined","grouped")]
            for mode in ("full","event"):
                for i,variants in enumerate(seq,1):
                    for j,v in enumerate(variants,1):
                        row=trial(mode,v,i,j,"stack-")
                        writer.writerow(row)
                        dest.flush()
                        print(phase,mode,i,v,round(row["elapsed_s"],3),flush=True)
        sys.exit(0)
    for pos,v in enumerate(order[0],1):
        trial(phase if phase == "full" else "event",v,0,pos)
    if phase != "full": trial("click","base",0,4)
    with (work/(phase+"-times.csv")).open("x") as dest:
        writer=csv.DictWriter(dest,fieldnames=fields,lineterminator="\n")
        writer.writeheader()
        for i,variants in enumerate(order,1):
            runs=[(phase if phase == "full" else "event",v) for v in variants]
            if phase != "full": runs.insert((i-1)%4,("click","base"))
            for j,(mode,v) in enumerate(runs,1):
                row=trial(mode,v,i,j)
                writer.writerow(row)
                dest.flush()
                print(mode,i,v,round(row["elapsed_s"],3),flush=True)
