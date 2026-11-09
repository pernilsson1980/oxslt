#!/usr/bin/env python3
"""Classifies Xalan conformance tests as PoC scope or later, by the features
they use, and writes a tab-separated manifest.

    classify-conformance.py <suite-dir> <manifest-out>

A test is <category>/<name>.xsl with a matching <name>.xml. "Later" follows the
PoC plan's list: xsl:key, xsl:number, import precedence, document(),
EXSLT and other extensions, locale-aware sorting, plus anything that needs
DTD processing (roadmap step 2). Detection is textual, so it's a heuristic:
good enough to size the subset, not a parser.

Manifest columns: id, category, stylesheet, input, scope, reasons.
"""

import re
import sys
from pathlib import Path

XSLT_NS = "http://www.w3.org/1999/XSL/Transform"

CHECKS = [
    ("key", re.compile(r"<xsl:key\b|(?<![\w.:-])key\s*\(")),
    ("number", re.compile(r"<xsl:number\b")),
    ("import", re.compile(r"<xsl:(import|apply-imports)\b")),
    ("document", re.compile(r"(?<![\w.:-])document\s*\(")),
    ("extension-elements", re.compile(r"extension-element-prefixes\s*=")),
    ("locale-sort", re.compile(r"<xsl:sort\b[^>]*\blang\s*=")),
    # Not generate-id(): the lookbehind rejects a preceding name character.
    ("dtd-functions", re.compile(r"(?<![\w.:-])(id|unparsed-entity-uri)\s*\(")),
]
# prefix:name( inside an attribute value or text: an extension function call.
# Axis steps (child::node()) don't match, because of the second colon.
PREFIXED_CALL = re.compile(r"(?<![\w:.-])([A-Za-z_][\w.-]*):([A-Za-z_][\w.-]*)\s*\(")
INCLUDE = re.compile(r"<xsl:include\b[^>]*\bhref\s*=\s*[\"']([^\"']+)[\"']")
DOCTYPE = re.compile(r"<!DOCTYPE\b")


def read(path: Path) -> str:
    return path.read_bytes().decode("utf-8", errors="replace")


def stylesheet_texts(path: Path, seen: set) -> list:
    """The stylesheet plus everything it includes, recursively."""
    if path in seen or not path.is_file():
        return []
    seen.add(path)
    text = read(path)
    texts = [text]
    for href in INCLUDE.findall(text):
        if "://" not in href:
            texts += stylesheet_texts((path.parent / href).resolve(), seen)
    return texts


XSLT_PREFIX = re.compile(r"xmlns:([\w.-]+)\s*=\s*[\"']" + re.escape(XSLT_NS) + r"[\"']")


def normalise_xslt_prefix(text: str) -> str:
    """Rewrites any prefix bound to the XSLT namespace as xsl:, so the
    checks below see <xsl:key> even in a file that writes <xslt:key>."""
    for prefix in set(XSLT_PREFIX.findall(text)):
        if prefix != "xsl":
            text = re.sub(r"(</?)" + re.escape(prefix) + r":", r"\1xsl:", text)
    return text


def reasons_for(xsl: Path, xml: Path) -> list:
    reasons = set()
    for text in stylesheet_texts(xsl.resolve(), set()):
        body = normalise_xslt_prefix(re.sub(r"<!--.*?-->", "", text, flags=re.S))
        for name, rx in CHECKS:
            if rx.search(body):
                reasons.add(name)
        for prefix, _ in PREFIXED_CALL.findall(body):
            if prefix not in ("xsl", "xmlns", "http", "https", "file"):
                reasons.add("extension-functions")
        if DOCTYPE.search(body):
            reasons.add("dtd")
    if DOCTYPE.search(read(xml)):
        reasons.add("dtd")
    return sorted(reasons)


def main() -> None:
    suite, out = Path(sys.argv[1]), Path(sys.argv[2])
    rows = []
    for xsl in sorted(suite.glob("*/*.xsl")):
        xml = xsl.with_suffix(".xml")
        if not xml.is_file():
            continue  # a helper imported or included by other tests
        reasons = reasons_for(xsl, xml)
        scope = "later" if reasons else "poc"
        cat = xsl.parent.name
        rows.append((f"{cat}/{xsl.stem}", cat, xsl.name, xml.name, scope, ",".join(reasons)))
    with out.open("w") as f:
        f.write("# id\tcategory\tstylesheet\tinput\tscope\treasons\n")
        for row in rows:
            f.write("\t".join(row) + "\n")
    poc = sum(1 for r in rows if r[4] == "poc")
    print(f"{len(rows)} tests: {poc} PoC, {len(rows) - poc} later")


if __name__ == "__main__":
    main()
