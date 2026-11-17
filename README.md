# oxslt

oxslt (pronounced "ox-ess-ell-tee") is a memory-safe replacement for the `xsltproc` command, written in Rust, for XSLT 1.0. The goal is a drop-in binary: a build system or distribution should be able to swap it in for `xsltproc` and get the same output bytes and the same exit codes, without changing anything else.

**Status: early.** oxslt is tested extensively against `xsltproc` but hasn't been used in production yet; please report any difference you find. In drop-in testing, the builds of 375 Debian packages ran oxslt beside `xsltproc` (`tests/dropin`, `scripts/dropin.sh`), Debian's own DocBook guides, Erlang's and libvirt's documentation among them. Of their 8,015 `xsltproc` calls, all but 18 gave identical output, written files included: 17 sort by `xsl:sort`'s `lang` with locales installed, which oxslt ignores, and in one (csound-manual's chunked HTML) 2 of 1,751 pages have a different "previous" link, where libxml2's evaluation of `[last()]` on a union depends on earlier evaluations (xsltproc's own links there disagree). (15 of the builds fail for reasons that have nothing to do with XSLT.)

## What works

Compatibility is measured against Debian bookworm's `xsltproc` (libxml2 2.9.14, libxslt 1.1.35, libexslt 0.8.20): oxslt is expected to give byte-identical output and the same exit code.

- **XSLT 1.0 and XPath 1.0**, including imports and import precedence, keys, `xsl:number`, `document()`, `format-number()` and the xml, html and text output methods.
- **EXSLT:** common (`exsl:node-set`, `exsl:object-type`, `exsl:document`), functions (`func:function`, `func:result`), sets, strings, math, dates and times (durations included), and `dyn:evaluate`.
- **XInclude** (`--xinclude`) and libxml2's XHTML serializer.
- **HTML input** (`--html`): a port of libxml2's HTML parser, with its implied elements, auto-closing and encoding detection.
- **DTDs and XML catalogs:** internal and external subsets, entities, default attributes and IDs, with catalogs from `XML_CATALOG_FILES` (or `SGML_CATALOG_FILES` with `--catalogs`).
- **Conformance:** 1,691 of the 1,692 tests in Apache's xalan-test `conf` suite (the Lotus/Xalan contribution to the OASIS XSLT suite) give output identical to `xsltproc`'s.
- **DocBook:** DocBook XSL's single-page HTML stylesheet (`html/docbook.xsl`) gives output identical to `xsltproc`'s for 237 DocBook 4.5 and 5 documents.
- **Real build stylesheets** from GnuCash, GIMP, darktable, FluidSynth and Wireshark give identical output.

### Not supported yet

- Extension elements other than `exsl:document`, `xt:document`, `saxon:output`, `xsl:document` and EXSLT's `func:function`/`func:result`, such as `redirect:write`.
- EXSLT's `dyn:map` and crypto functions, and saxon's `expression`, `eval` and `evaluate`. `function-available()` reports them as available, as `xsltproc` does, but calling one is an error.
- `xsltproc`'s `--debug`.
- EBCDIC encodings.
- `xsl:sort`'s `lang` attribute is ignored.

The smaller known differences from `xsltproc` are listed in [CLAUDE.md](CLAUDE.md#known-differences-from-xsltproc). [ARCHITECTURE.md](ARCHITECTURE.md) is a map of the code.

## Usage

Install it from crates.io (this needs Rust 1.88 or later):

```sh
cargo install oxslt
oxslt [options] stylesheet.xsl input.xml [input.xml ...]
```

Or build it from a clone of this repository with `cargo build --release`; the binary is `target/release/oxslt`.

As with `xsltproc`, `-` in place of a file name reads it from standard input.

The options are `xsltproc`'s, and are read as it reads them (every argument after the stylesheet is an input):

| Option | |
|---|---|
| `-o`, `--output file` | Write to `file` instead of standard output, making missing directories |
| `--param name value` | Pass a parameter as an XPath expression |
| `--stringparam name value` | Pass a parameter as a string |
| `--nonet` | Accepted; oxslt never uses the network anyway |
| `--novalid` | Don't load the external DTD or apply its default attributes |
| `--nodtdattr` | Load the external DTD but don't apply default attributes |
| `--encoding name` | Read the input documents in this encoding |
| `--xinclude` | Process XIncludes in the input documents (and in `document()`'s) |
| `--xincludestyle` | Process XIncludes in the stylesheet |
| `--html` | Parse the input documents as HTML |
| `--catalogs` | Use the catalogs in `SGML_CATALOG_FILES` instead of `XML_CATALOG_FILES` |
| `--nowrite`, `--nomkdir` | Refuse to write files with `exsl:document`, or to create directories |
| `--writesubtree path` | Allow `exsl:document` to write only below `path` |
| `--path paths` | Directories to search for DTDs and entities |
| `--noout` | Don't write the result |
| `--maxdepth n`, `--maxvars n` | libxslt's recursion limits (3,000 nested levels, 15,000 variables) |
| `--maxparserdepth n`, `--huge` | libxml2's parser limits |
| `--version`, `--dumpextensions` | Print what `xsltproc` prints |

`--verbose`, `--timing`, `--profile`, `--load-trace`, `--repeat`, `--nodict` and `--seed-rand` are accepted and change nothing; `--debug` isn't supported.

Exit codes match those documented in the `xsltproc` man page.

### Using it as `xsltproc`

oxslt behaves as `xsltproc`, so a build can use it under that name, for example with a symlink earlier in `PATH`:

```sh
ln -s "$(command -v oxslt)" ~/bin/xsltproc
```

On Debian-based systems, `update-alternatives` can do the same system-wide. Like `xsltproc`, it writes the files a stylesheet's `exsl:document` names; see Security below.

## Performance

On native arm64 Linux, oxslt is faster than `xsltproc` on all 15 benchmark workloads and uses less memory on all of them. The four that best stand for real use (measured 2026-11-15):

| Workload | | `xsltproc` | oxslt | Speed | Memory (`xsltproc` / oxslt) |
|---|---|---:|---:|---:|---:|
| docbook-chunked | DocBook XSL's chunker on a 400-page book | 11.4 s | 8.0 s | 1.42× | 57 / 45 MB |
| darktable-prefs | a real build stylesheet; mostly start-up and parsing | 10.2 ms | 8.0 ms | 1.27× | 7.8 / 4.8 MB |
| wireshark-values-FIX50SP2 | a real build stylesheet on its largest input | 31.9 ms | 9.8 ms | 3.27× | 13.3 / 6.5 MB |
| html-report-27MB | a 27 MB document to an HTML report | 2.59 s | 1.18 s | 2.20× | 718 / 390 MB |

In drop-in testing, GIMP's manual built about twice as fast and Csound's 1,751 pages about 1.2 times. All 15 workloads, the methods and the history are in [tests/bench/RESULTS.md](tests/bench/RESULTS.md); `scripts/bench.sh` reruns the benchmarks.

## Security

oxslt treats stylesheets and documents as untrusted input.

- There's no `unsafe` code: the crate is built with `#![forbid(unsafe_code)]`.
- **No network access at all**, with or without `--nonet`.
- **Files oxslt writes:** the one given with `-o`, and, as `xsltproc` does, whatever a stylesheet's `exsl:document` names, anywhere the user may write. `--nowrite`, `--nomkdir` and `--writesubtree` restrict that, as in `xsltproc`; use them for untrusted stylesheets.
- **It reads local files as `xsltproc` does:** files named by `document()`, `xsl:include` and `xsl:import`, external DTDs, and external entities in the input. So a stylesheet or document can read any file the user can read. Don't run untrusted stylesheets on machines with confidential files, and don't hand their output to people you don't trust.
- **Resource limits:** libxml2's parser limits (nesting depth, text and name sizes), protection against entity-expansion attacks, libxslt's recursion limits, a guard that stops a transform before it overflows its stack, and resistance to hash flooding. The library also offers a work budget and a timeout (`xslt::exec::Limits`); the command line doesn't set them, since `xsltproc` has none.
- The XML and HTML parsers, XPath and whole transforms are fuzzed with cargo-fuzz.

Please report vulnerabilities privately, as [SECURITY.md](SECURITY.md) describes.

## Testing

```sh
cargo test
```

The main test method is comparison with `xsltproc`: the same stylesheet and input go through both, and the output bytes and exit code must match. Without fetched data, `cargo test` runs the unit tests and our own comparison cases in `tests/cases/`, and the larger suites skip themselves with a note. The larger suites are fetched, and `xsltproc`'s output recorded, by scripts that need Docker:

```sh
scripts/fetch-conformance.sh         # xalan-test conformance suite (Apache-2.0)
scripts/fetch-real-stylesheets.sh    # real build stylesheets (GPL/LGPL; never committed)
scripts/fetch-docbook.sh             # DocBook documents
scripts/fetch-docbook-xsl.sh         # DocBook XSL stylesheets, DTDs and catalogs
scripts/record-docbook-html.sh       # xsltproc's DocBook HTML output
scripts/html-diff.sh fuzz 5000 1     # --html trees against xsltproc's, on generated broken HTML (also: corpus)
scripts/dropin.sh                    # build Debian packages with oxslt beside xsltproc
```

`xsltproc` runs only inside an offline, read-only Debian container (`tests/reference/Dockerfile`), because some of its extensions write files and fetch from the network.

Fuzzing needs a nightly toolchain:

```sh
cargo +nightly fuzz run transform -- -max_total_time=300   # also: parse, xpath, html
```

## Licence

oxslt is licensed under either of

- the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), or
- the MIT licence ([LICENSE-MIT](LICENSE-MIT)),

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in oxslt by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without any additional terms or conditions. [CONTRIBUTING.md](CONTRIBUTING.md) says how to work on it.

Much of oxslt follows the code of libxml2, libxslt and libexslt (MIT licences); their notices are in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
