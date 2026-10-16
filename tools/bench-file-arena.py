#!/usr/bin/env python3
"""Compare lifetime prototype modes on newline-separated public source manifests.

Build: cargo build --release -p tsrs_frontend_arena --example measure
Run: python3 tools/bench-file-arena.py --out results.json name=/absolute/manifest.txt ...
This measures eager parse/bind/publication, not project resolution or type checking.
"""
import argparse
import json
import platform
import statistics
import subprocess
from pathlib import Path

MODES = ["tls", "region", "retained", "direct", "published-tls", "published", "region-drop", "published-drop"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifests", nargs="+", help="project=/absolute/manifest.txt")
    parser.add_argument("--out", required=True)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--binary", default="target/release/examples/measure")
    args = parser.parse_args()
    raw = []
    for project, manifest in (item.split("=", 1) for item in args.manifests):
        for run in range(-1, args.runs):
            modes = MODES if run % 2 else list(reversed(MODES))
            for mode in modes:
                result = json.loads(subprocess.check_output([args.binary, mode, manifest], text=True))
                if run >= 0:
                    raw.append({"project": project, "run": run, **result})
            print(f"{project}: {'warmup' if run < 0 else run + 1}", flush=True)
        rows = [r for r in raw if r["project"] == project]
        assert len({(r["checksum"], r["diagnostics"]) for r in rows}) == 1, "front-end outputs differ"
    summary = []
    for project in dict.fromkeys(r["project"] for r in raw):
        for mode in MODES:
            rows = [r for r in raw if r["project"] == project and r["mode"] == mode]
            summary.append({"project": project, "mode": mode, **{
                key: statistics.median(r[key] for r in rows)
                for key in rows[0] if key not in {"project", "mode", "run"}
            }})
    output = {"platform": platform.platform(), "runs": args.runs, "warmups": 1,
              "instruction_counter": "macOS proc_pid_rusage v4 delta; zero on other platforms (use bench/count.py on Linux)",
              "summary": summary, "raw": raw}
    Path(args.out).write_text(json.dumps(output, indent=2) + "\n")


if __name__ == "__main__":
    main()
