# Contributing

The goal is the same output bytes and exit codes as Debian bookworm's
`xsltproc` (libxml2 2.9.14, libxslt 1.1.35, libexslt 0.8.20), so most work
starts from a difference between the two.

## Reporting a difference

Open an issue with the stylesheet, the input and the command line, and what
`xsltproc` and oxslt each print and exit with. Small cases are best.

## Changing the code

- Check `xsltproc`'s behaviour in its source or by running it, and match
  it, quirks included; comments name the C function a port follows.
  `CLAUDE.md` records the behaviour and decisions so far.
- Add a test case to `tests/cases/` with `style.xsl` and `in.xml` (and
  `args` or `env` if needed), and record `xsltproc`'s output with
  `scripts/record-expected.sh`.
- Run `xsltproc` on stylesheets only inside the offline container the
  scripts use, never directly: some of its extensions write files and
  fetch from the network.
- Before sending a change: `cargo fmt`, `cargo clippy --all-targets -- -D
  warnings` and `cargo test`.

Contributions are dual-licensed as described in the README.
