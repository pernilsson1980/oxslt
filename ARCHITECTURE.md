# Architecture

oxslt reimplements Debian bookworm's `xsltproc` (libxml2 2.9.14, libxslt 1.1.35, libexslt 0.8.20) in Rust. The goal is the same output bytes and exit codes, quirks included. This document is a map of the code. The behaviour itself, decision by decision, is recorded in [CLAUDE.md](CLAUDE.md), and the README says what's supported.

## One run, end to end

```
oxslt [options] style.xsl in.xml
  │
  cli          arguments read as xsltproc's main() reads them
  │            a thread with a 512 MiB stack does the rest
  ├─ catalog   locate files: system ID, XML catalogs, --path
  ├─ parse     style.xsl → tree::Document      (quick-xml tokens, dtd, entities)
  │   └─ xinclude, if --xincludestyle; <?xml-stylesheet?> may name another stylesheet
  ├─ xslt::compile   Document → xslt::ir::Stylesheet   (imports, includes, XPath compiled)
  └─ for each input:
      ├─ parse or html     in.xml → Document
      ├─ xinclude          if --xinclude
      ├─ xslt::exec        Transform::run(stylesheet, source) → result Document
      └─ serialize         result → bytes (xml, html, xhtml, text) → stdout or -o
```

`src/main.rs` only calls `oxslt::run_before_exit`. Everything else is in the library, so tests and fuzzing drive the same code as the command line.

## Modules

| Module | What it does |
|---|---|
| `cli` | xsltproc's options, usage and version texts and exit codes, the transform thread, writing the result. |
| `tree` | The document arena (`Document`, `NodeId`), interned names (`NamePool`), building trees (`Builder`). |
| `parse` | XML parsing: quick-xml as a tokenizer only. Encodings, namespaces, entity expansion and the tree are oxslt's own. |
| `dtd` | Internal and external subsets: entities, parameter entities, conditional sections, default attributes, IDs. No validation. |
| `catalog` | Loading as xsltproc's entity loader does: XML catalogs, `--catalogs`, `--path`, reading files. No network access at all. |
| `encoding` | Encoding names as libxml2 reads them, and decoding and encoding with `encoding_rs`. |
| `html` | `--html`: a function-by-function port of libxml2's HTML parser. |
| `xinclude` | XInclude, as libxml2 processes it. |
| `uri` | libxml2's URI building (`xmlBuildURI`, `xmlBuildRelativeURI`). |
| `xpath` | XPath 1.0: `lexer`, `parser` → `ast`, `eval` (core functions in `eval/functions.rs`), `number` (libxml2's number formatting). |
| `xslt::compile` | The stylesheet to IR: modules and import precedence (`modules`, `link`), top-level declarations (`declarations`), template bodies (`body`). |
| `xslt::ir` | The compiled stylesheet: templates, rules, globals, keys, output settings, instructions (`Instr`). |
| `xslt::pattern` | Match patterns and default priorities. |
| `xslt::exec` | The transform: `Transform` (`mod.rs`), with XSLT/EXSLT functions (`functions`), rule selection (`rules`), copying (`copy`), `xsl:number` counting (`numbering`), `exsl:document` (`write`), `xsl:strip-space` (`strip`), limits (`limits`). |
| `xslt::output` | Building result trees (`ResultTree`): pending elements, namespace fixup. |
| `xslt::exslt`, `xslt::dates` | EXSLT's common, sets, strings, math, dynamic, dates and times. |
| `xslt::number`, `xslt::number_format` | `xsl:number`'s and `format-number()`'s formatting, ported from libxslt. |
| `serialize` | Output methods, following libxslt's `xsltSaveResultTo` and libxml2's xml, html and xhtml writers. |

## Core data structures

**Documents are arenas.** All of a document's nodes sit in one `Vec`, in document order: an element, then its attributes, then its children. A `NodeId` is an index into it, so comparing IDs compares document order, and a node's descendants are one range of the vector. Text and attribute values are ranges in one string buffer. `Document` is a pointer-sized handle to its `DocumentData`, so a freed document costs almost nothing while its number stays taken.

**Names are interned.** `NamePool` gives every name and namespace URI a small integer `Sym`, shared by the whole transform. Template matching and name tests compare integers.

**Nodes across documents.** An XPath `NodeRef` is (document, node, namespace-node index). The transform keeps every document it uses in one list: the source, documents `document()` loads, and result tree fragments. A `NodeRef` names its document by its index there. Ordering `NodeRef`s gives document order within a document.

**Values.** `xpath::Value` is a string, number, boolean, node-set, or `Tree`: a result tree fragment, which libxml2 treats as a type of its own (XPATH_XSLT_TREE).

## Design rules

These are hard to change later; CLAUDE.md lists them in full.

1. **The source is read-only during a transform.** Result trees are built separately, append only. This is what keeps oxslt clear of bugs like libxslt's CVE-2021-30560.
2. **Compile, then run.** Stylesheets and XPath are compiled to the IR and an AST once. Template rules are indexed by node kind and name.
3. **Arena trees with index IDs**, not reference-counted node graphs. Node identity is stable, which `generate-id()` and document order rely on.
4. **No `unsafe` code** (`#![forbid(unsafe_code)]`), and no panics on any input: errors are returned.
5. **The core is a library**; the command line is a thin layer over it.

## How the transform works

`Transform::run` computes globals lazily, applies template rules from the root, and returns the result document.

**Evaluations are immutable; work happens between them.** XPath evaluates with read-only access to the transform (`Env`). Something that needs the transform changed fails the evaluation after recording what it needs: loading a `document()`, computing a global, compiling a `dyn:evaluate` expression, adding a fragment an extension function made, or calling a `func:function`. `Transform::env_eval` does that work, then evaluates again, with earlier results cached by their order in the evaluation. That keeps evaluation pure without copying the transform's state.

**Errors follow libxslt's two states.** An error libxslt only reports (an attribute after children, a redefined variable) lets the transform go on, then fails it: exit 9 and no output. A failed XPath evaluation, `xsl:message terminate="yes"` or the recursion limit stops it at once: exit 10. Compile errors are exit 5; with `-o`, any error during the transform is exit 11.

**Limits.** Recursion is counted as libxslt counts it, per nested sequence constructor (`--maxdepth`, `--maxvars`). The stack is measured rather than estimated, so a transform nested too deeply stops before it can overflow. The parser has libxml2's limits (depth, text and name sizes, entity expansion). The library also offers a work budget and a timeout (`exec::Limits`), which the fuzz targets use.

**Fragment lifetimes.** Each result tree fragment is a document of its own. A template call frees the fragments its variables made when it returns, keeping their numbers taken, so large DocBook builds don't accumulate millions of them.

## Faithfulness to libxml2 and libxslt

Where the C libraries' behaviour matters for output, oxslt ports their code rather than following the specifications: parsing (HTML especially), number formatting, `xsl:number`, `format-number()`, EXSLT, XInclude, and the serializers. Comments name the C function each port follows. Where libxml2 or libxslt departs from a specification, oxslt follows the library, and CLAUDE.md records each such case. The few differences left are listed there too, with the reason for each.

## Testing

The main method is comparison with xsltproc: the same stylesheet and input go through both, and the output bytes and exit code must match. xsltproc only ever runs inside an offline, read-only Debian container, because some of its extensions write files and fetch from the network.

| Layer | Where |
|---|---|
| Recorded cases, ours | `tests/cases/` (stylesheet, input, xsltproc's recorded output and exit code); `scripts/record-expected.sh` records them |
| Conformance | Apache's xalan-test `conf` suite (fetched), `tests/conformance.rs` |
| Real stylesheets and DocBook | fetched, `tests/differential.rs` (`tests/real/`), `tests/docbook_html.rs` |
| Parser against libxml2 | `tests/parse_corpus.rs` (trees compared node for node), `scripts/html-diff.sh` for `--html` |
| XPath | `tests/xpath_oracle.rs`, expressions against xsltproc's recorded results |
| Command line | `tests/cli.rs` |
| Hostile input | `tests/limits.rs`, and cargo-fuzz targets for parsing, HTML, XPath and whole transforms (`fuzz/`) |
| Real builds | `scripts/dropin.sh` builds Debian packages with a wrapper as `xsltproc` that runs both tools on every call and records any difference |
| Performance | `scripts/bench.sh`, against xsltproc on 15 workloads (`tests/bench/RESULTS.md`) |

Without fetched data, `cargo test` runs the unit tests, our own cases and the command-line tests; the larger suites skip themselves with a note.
