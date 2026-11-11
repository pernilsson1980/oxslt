#!/usr/bin/env python3
"""Writes random, mostly broken HTML documents for scripts/html-diff.sh."""
import os, random, sys

out, count, seed = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
rnd = random.Random(seed)
tags = ("verylongname" * 10 + " html head body title meta link script style p div span a b i u em strong table tr td th "
        "thead tbody tfoot caption ul ol li dl dt dd h1 h2 h3 pre br hr img input select option "
        "textarea form label button font center blockquote frameset frame noframes iframe object "
        "param map area base basefont col colgroup abbr acronym address applet big small tt sub sup "
        "q s strike ins del noscript fieldset legend optgroup isindex menu dir embed x foo svg:rect "
        "xml:foo a:b:c P DIV TD Html").split()
attrs = "id name class href src checked selected nowrap title style type charset http-equiv content xmlns onclick value".split()
ents = ["&amp;", "&lt;", "&nbsp;", "&copy;", "&eacute;", "&foo;", "&amp", "&#65;", "&#x41;", "&#0;",
        "&#xD800;", "&#1114112;", "&#;", "&;", "& ", "&#x;", "&hellip;", "&Alpha;", "&euro;", "&apos;", "&quot"]
texts = [" " * 1200, "x" * 1500, "a" * 999 + " b", "\r", "hello", " ", "\n", "  \n  ", "a<b", "x>y", "\t", "caf\xe9".encode("latin-1").decode("latin-1"),
         "naïve", "日本", "]]>", "--", "\r\n", "<", "</", "</ p>", "< b>", "<3"]

def attr():
    a = rnd.choice(attrs)
    r = rnd.random()
    if r < 0.2: return a
    v = rnd.choice(["x", "a b", "1", "", "UTF-8", "text/html; charset=ISO-8859-1",
                    "text/html; charset=utf-8", "windows-1252", "café", "&amp;x", rnd.choice(ents)])
    q = rnd.choice(['"', "'", ""])
    if q == "" and (" " in v or v == ""): v = "v"
    sp = rnd.choice(["=", " = ", "="])
    return f"{a}{sp}{q}{v}{q}"

def node(depth):
    r = rnd.random()
    if r < 0.003:
        return "<div>" * 300 + "deep" + "</div>" * 10
    if r < 0.01:
        return rnd.choice(["</html>", "</body>", "</head>", "<html lang=x>", "<head>", "<body class=b>",
                           "</html><p>after", "</body></html><!-- end -->  <p>x"])
    if r < 0.02:
        return rnd.choice(['<meta http-equiv="Content-Type" content="text/html; charset=windows-1251">',
                           '<meta charset="iso-8859-2">', '<meta charset=utf-8>', '<meta charset="bogus">',
                           '<meta http-equiv=content-type content="text/html;charset = koi8-r">'])
    if depth > 6 or r < 0.3:
        return rnd.choice(texts + ents) if rnd.random() < 0.8 else rnd.choice(
            ["<!-- c -->", "<!--x--!>", "<!---->", "<?pi data?>", "<?php echo 1 ?>", "<!DOCTYPE html>", "<!-->"])
    t = rnd.choice(tags)
    s = "<" + t + "".join(" " + attr() for _ in range(rnd.randint(0, 3)))
    s += rnd.choice([">", ">", ">", "/>", " >"])
    if t.lower() in ("script", "style"):
        s += rnd.choice(["if (a<b) x();", "</notend", "a</b>c", "<!-- x -->", "x", "y" * 2100, "</ x", "<\\/"])
    s += "".join(node(depth + 1) for _ in range(rnd.randint(0, 4)))
    r = rnd.random()
    if r < 0.6: s += f"</{t}>"
    elif r < 0.7: s += f"</{rnd.choice(tags)}>"
    return s

os.makedirs(out, exist_ok=True)
for i in range(count):
    head = rnd.choice(["", "", '<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01//EN">\n',
                       '<!DOCTYPE html PUBLIC "-//W3C//DTD HTML 4.0 Transitional//EN" "http://www.w3.org/TR/REC-html40/loose.dtd">',
                       "<?xml version='1.0'?>", "  \n", "<!-- lead -->"])
    body = head + "".join(node(0) for _ in range(rnd.randint(1, 6)))
    enc = rnd.choice(["utf-8", "utf-8", "latin-1", "latin-1-raw", "utf-16"])
    if enc == "utf-16":
        data = b"\xff\xfe" + body.encode("utf-16-le")
    elif enc == "utf-8":
        data = body.encode("utf-8")
    else:
        data = body.encode("latin-1", errors="replace")
    if rnd.random() < 0.05:
        data = data.replace(b"x", b"\x00", 1)
    with open(os.path.join(out, f"{i:05}.html"), "wb") as f:
        f.write(data)
