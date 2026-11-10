# oxslt

oxslt is a memory-safe replacement for the `xsltproc` command, written in Rust, for XSLT 1.0. The goal is a drop-in binary: a build system or distribution should be able to swap it in for `xsltproc` and get the same output bytes and the same exit codes, without changing anything else.

**Status: proof of concept.** It isn't ready for production use yet. In a first round of drop-in testing, 17 Debian packages built with oxslt beside `xsltproc` gave identical output, written files included, for all 715 `xsltproc` calls (`tests/dropin`, `scripts/dropin.sh`).

## What works

Compatibility is measured against Debian bookworm's `xsltproc` (libxml2 2.9.14, libxslt 1.1.35, libexslt 0.8.20): oxslt is expected to give byte-identical output and the same exit code.

- **XSLT 1.0 and XPath 1.0**, including imports and import precedence, keys, `xsl:number`, `document()`, `format-number()` and the xml, html and text output methods.
- **EXSLT:** common (`exsl:node-set`, `exsl:object-type`, `exsl:document`), sets, strings, math, most of dates and times, and `dyn:evaluate`.
- **XInclude** (`--xinclude`) and libxml2's XHTML serializer.
- **DTDs and XML catalogs:** internal and external subsets, entities, default attributes and IDs, with catalogs from `XML_CATALOG_FILES` (or `SGML_CATALOG_FILES` with `--catalogs`).
- **Conformance:** 1,691 of the 1,692 tests in Apache's xalan-test `conf` suite (the Lotus/Xalan contribution to the OASIS XSLT suite) give output identical to `xsltproc`'s.
- **DocBook:** DocBook XSL's single-page HTML stylesheet (`html/docbook.xsl`) gives output identical to `xsltproc`'s for 237 DocBook 4.5 and 5 documents.
- **Real build stylesheets** from GnuCash, GIMP, darktable, FluidSynth and Wireshark give identical output.

### Not supported yet

- Extension elements other than `exsl:document` and `xt:document`, such as `func:function` and `saxon:output`.
- EXSLT's `date:add`, `date:difference`, `date:duration`, `date:seconds` and `date:sum`, and the crypto functions. `function-available()` reports them as available, as `xsltproc` does, but calling one is an error.
- `xsltproc` options other than those listed below.
- EBCDIC encodings.
- `xsl:sort`'s `lang` attribute is ignored.

The smaller known differences from `xsltproc` are listed in [CLAUDE.md](CLAUDE.md#known-differences-from-xsltproc).

## Usage

```sh
cargo build --release
target/release/oxslt [options] stylesheet.xsl input.xml [input.xml ...]
```

The options are `xsltproc`'s:

| Option | |
|---|---|
| `-o`, `--output file` | Write to `file` instead of standard output |
| `--param name value` | Pass a parameter as an XPath expression |
| `--stringparam name value` | Pass a parameter as a string |
| `--nonet` | Accepted; oxslt never uses the network anyway |
| `--novalid` | Don't load the external DTD or apply its default attributes |
| `--xinclude` | Process XIncludes in the input documents |
| `--nowrite`, `--nomkdir` | Refuse to write files with `exsl:document`, or to create directories |
| `--writesubtree path` | Allow `exsl:document` to write only below `path` |
| `--path paths` | Directories to search for DTDs and entities |
| `--maxdepth n` | Accepted; the template recursion limit is fixed at 3,000 |
| `--version` | Print the version |

Exit codes match those documented in the `xsltproc` man page.

## Performance

On native arm64 Linux, oxslt is faster than `xsltproc` on 13 of 14 benchmark workloads and uses about half the memory on all of them; DocBook HTML pages run about 1.3 times as fast. The one slower workload is grouping without keys (`preceding-sibling` comparisons), at about 0.8 times `xsltproc`'s speed. Details and methods are in [tests/bench/RESULTS.md](tests/bench/RESULTS.md); `scripts/bench.sh` reruns the benchmarks.

## Security

oxslt treats stylesheets and documents as untrusted input.

- There's no `unsafe` code: the crate is built with `#![forbid(unsafe_code)]`.
- **No network access at all**, with or without `--nonet`.
- **Files oxslt writes:** the one given with `-o`, and, as `xsltproc` does, whatever a stylesheet's `exsl:document` names, anywhere the user may write. `--nowrite`, `--nomkdir` and `--writesubtree` restrict that, as in `xsltproc`; use them for untrusted stylesheets.
- **It reads local files as `xsltproc` does:** files named by `document()`, `xsl:include` and `xsl:import`, external DTDs, and external entities in the input. So a stylesheet or document can read any file the user can read. Don't run untrusted stylesheets on machines with confidential files, and don't hand their output to people you don't trust.
- **Resource limits:** libxml2's parser limits (nesting depth, text and name sizes), protection against entity-expansion attacks, a template recursion limit of 3,000, and resistance to hash flooding. The library also offers a work budget and a timeout (`xslt::exec::Limits`); the command line doesn't set them, since `xsltproc` has none.
- The parser, XPath and whole transforms are fuzzed with cargo-fuzz.

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
```

`xsltproc` runs only inside an offline, read-only Debian container (`tests/reference/Dockerfile`), because some of its extensions write files and fetch from the network.

Fuzzing needs a nightly toolchain:

```sh
cargo +nightly fuzz run transform -- -max_total_time=300   # also: parse, xpath
```

## Licence

oxslt is licensed under either of

- the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), or
- the MIT licence ([LICENSE-MIT](LICENSE-MIT)),

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in oxslt by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without any additional terms or conditions.
