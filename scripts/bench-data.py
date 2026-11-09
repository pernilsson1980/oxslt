#!/usr/bin/env python3
"""Generates the synthetic benchmark inputs (deterministic, seed 42).

    bench-data.py <out-dir>

records-200k.xml (~20 MB): the input for identity, html-report, sort and
recursion. records-5k.xml: the input for grouping, which is quadratic.
"""
import random
import sys
from pathlib import Path

WORDS = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau".split()


def write(path: Path, n: int, seed: int = 42) -> None:
    rnd = random.Random(seed)
    with path.open("w", encoding="utf-8") as f:
        f.write('<?xml version="1.0" encoding="UTF-8"?>\n<records>\n')
        for i in range(n):
            cat = f"c{rnd.randrange(50)}"
            amount = f"{rnd.uniform(0, 1000):.2f}"
            name = " ".join(rnd.choice(WORDS) for _ in range(2)).title()
            note = " ".join(rnd.choice(WORDS) for _ in range(rnd.randrange(3, 12)))
            f.write(f'  <r id="{i}" cat="{cat}"><name>{name}</name><amount>{amount}</amount><note>{note} &amp; more</note></r>\n')
        f.write("</records>\n")


out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
write(out / "records-200k.xml", 200_000)
write(out / "records-5k.xml", 5_000)
for p in sorted(out.glob("*.xml")):
    print(f"{p.name}: {p.stat().st_size / 1e6:.1f} MB")
