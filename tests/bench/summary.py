#!/usr/bin/env python3
"""Prints a markdown table from the benchmark results in a directory:
summary.txt (identical output, peak memory) and json/<name>.json (hyperfine).

    summary.py <results-dir> [name]

With a name, prints that workload's one-line timing instead.
"""
import json
import sys
from pathlib import Path

out = Path(sys.argv[1])


def timing(name):
    r = {x["command"]: x for x in json.loads((out / "json" / f"{name}.json").read_text())["results"]}
    return r["xsltproc"], r["oxslt"]


if len(sys.argv) > 2:
    x, o = timing(sys.argv[2])
    print(f"xsltproc {x['mean'] * 1000:8.1f} ms   oxslt {o['mean'] * 1000:8.1f} ms   speedup {x['mean'] / o['mean']:5.2f}x")
    sys.exit()

rows = [line.split("|") for line in (out / "summary.txt").read_text().splitlines()[1:]]
print("| Workload | Identical | xsltproc ms | oxslt ms | Speed (xsltproc time / oxslt time) | xsltproc MB | oxslt MB |")
print("|---|---|---:|---:|---:|---:|---:|")
for name, same, xp_kb, ox_kb in rows:
    x, o = timing(name)
    ms = lambda r: f"{r['mean'] * 1000:.1f} ± {r['stddev'] * 1000:.1f}"
    print(f"| {name} | {same} | {ms(x)} | {ms(o)} | {x['mean'] / o['mean']:.2f}x | {int(xp_kb) / 1024:.1f} | {int(ox_kb) / 1024:.1f} |")
