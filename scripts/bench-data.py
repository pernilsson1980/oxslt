#!/usr/bin/env python3
"""Generates the synthetic benchmark inputs (deterministic, seed 42).

    bench-data.py <out-dir>

records-200k.xml (~20 MB): the input for identity, html-report, sort and
recursion. records-5k.xml: the input for grouping, which is quadratic.
book.xml: a DocBook 4 book for DocBook XSL's chunked XHTML, with the
numbered figures and tables, cross-references and index terms that make
large manuals (GIMP's, Csound's) slow.
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


def book(path: Path, chapters: int, sections: int, seed: int = 42) -> None:
    rnd = random.Random(seed)
    words = lambda n: " ".join(rnd.choice(WORDS) for _ in range(n))
    ids = [f"s{c}-{s}" for c in range(chapters) for s in range(sections)]
    with path.open("w", encoding="utf-8") as f:
        f.write('<?xml version="1.0" encoding="UTF-8"?>\n<book lang="en"><title>Benchmark manual</title>\n')
        for c in range(chapters):
            f.write(f'<chapter id="c{c}"><title>{words(3).title()}</title>\n')
            for s in range(sections):
                f.write(f'<sect1 id="s{c}-{s}"><title>{words(4).title()}</title>\n')
                for p in range(4):
                    target = rnd.choice(ids)
                    f.write(f'<para><indexterm><primary>{rnd.choice(WORDS)}</primary></indexterm>'
                            f'{words(25)} See <xref linkend="{target}"/> and <xref linkend="f{target}"/>.</para>\n')
                f.write(f'<figure id="fs{c}-{s}"><title>{words(3)}</title><mediaobject><imageobject>'
                        f'<imagedata fileref="images/{c}-{s}.png"/></imageobject></mediaobject></figure>\n')
                f.write(f'<table id="ts{c}-{s}"><title>{words(3)}</title><tgroup cols="2"><tbody>'
                        + "".join(f'<row><entry>{words(2)}</entry><entry>{words(3)}</entry></row>' for _ in range(5))
                        + '</tbody></tgroup></table>\n')
                f.write('</sect1>\n')
            f.write('</chapter>\n')
        f.write('<index/>\n</book>\n')


out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
write(out / "records-200k.xml", 200_000)
write(out / "records-5k.xml", 5_000)
book(out / "book.xml", 30, 12)
for p in sorted(out.glob("*.xml")):
    print(f"{p.name}: {p.stat().st_size / 1e6:.1f} MB")
