# oxslt

oxslt (pronounced "ox-ess-ell-tee") is a memory-safe Rust replacement for the `xsltproc` command-line tool, targeting XSLT 1.0. It must work as a drop-in binary: build systems and distributions should be able to swap it in for `xsltproc` without changing anything else.

## Status

Proof of concept, steps 5.1–5.3 done (2026-11-08): parser, XPath, and the XSLT engine with a first xml/text serializer and the CLI. **Both PoC criteria are met:**
- darktable's `generate_prefs.xsl` is byte-identical to xsltproc.
- 1,392 of 1,394 PoC conformance tests pass (99.9%; the target is 1,116). The two left need EBCDIC and libxml2's XHTML writer.

Step 5.5, benchmarks, is done (2026-11-09). On native arm64 against Debian's xsltproc, oxslt is faster on 13 of 14 workloads and slower on one (grouping), and uses about half the memory on all; output is byte-identical on all 14. Release builds use fat LTO (1–9% faster). Allocations were cut on 2026-11-09: parsing allocates almost nothing, and DocBook runs make about half as many as xsltproc's. Template rules are indexed by node kind and name, and compiling is about 15% faster than at first; DocBook HTML runs about 1.3x faster than xsltproc. **Performance work stopped there (2026-11-09)**; the options measured and not taken (lazy template bodies, shared XPath values) are in `tests/bench/RESULTS.md`. Details are in `tests/bench/RESULTS.md`; `scripts/bench.sh` reruns them.

Roadmap step 2, catalogs, DTD handling and `--nonet`, is done (2026-11-09). Its gate, "offline DocBook-style inputs parse correctly", is met: the 234 well-formed DocBook 4.5 man pages from shadow and Linux-PAM give trees identical to libxml2's, with the DTD loaded through the catalog and its defaults applied. IDs (`id()`) and `unparsed-entity-uri()` work too; 38 of the 298 "later" conformance tests now pass.

Roadmap step 3, full XSLT 1.0 and EXSLT, is done (2026-11-09). Imports, keys, `xsl:number`, `document()` and EXSLT's common, sets, strings and math functions work. All 298 "later" conformance tests pass. **The gate is met:** DocBook XSL's `html/docbook.xsl` gives output byte-identical to xsltproc's for all 237 DocBook documents from shadow and Linux-PAM, DocBook 5 ones included (`tests/docbook_html.rs`).

**Decided 2026-11-10: `exsl:document` works as in xsltproc** (reversing the 2026-11-09 decision to leave it out). Drop-in testing showed that most xsltproc calls in Debian builds are DocBook man pages, which write through it. The command line writes what xsltproc writes, relative to the `-o` file or the current directory, creating missing directories, restricted only by xsltproc's `--nowrite`, `--nomkdir` and `--writesubtree`. The library refuses writes unless the caller allows them (`exec::Limits::writes`), so tests and fuzzing never write files by default.

Drop-in testing on Debian builds started 2026-11-09 (`tests/dropin`, `scripts/dropin.sh`): 17 bookworm packages are built offline with a wrapper as `/usr/bin/xsltproc` that runs oxslt and xsltproc on every call and records differences. It found and fixed: catalog resolution of command-line stylesheets, stylesheet-only runs, `document()` of a result tree fragment, EXSLT dates, `dyn:evaluate`, `--xinclude`, variables in `xsl:number` patterns, undeclared command-line parameters, leaked `exclude-result-prefixes` from included modules, `--` in comments, and libxml2's XHTML serializer. Since `exsl:document` (2026-11-10), all 715 calls match, including every file `exsl:document` writes (DocBook man pages and chunked HTML). The wrapper compares written files by before/after snapshots, so builds run serially.

The second batch (2026-11-10, `tests/dropin/packages-2.txt`, 83 packages, run with `--list packages-2.txt` in groups) found and fixed: `--catalogs`, `-` as standard input, libxml2's extra slashes in XPath paths, undeclared entities in attribute values, ties between globals (first declaration wins), compile errors without inputs (exit 5), and the URL of a module found through `--path`. After the fixes all 1,778 calls match, lirc's 61 `--html` calls included since `--html` was added (2026-11-11). Two builds fail for reasons unrelated to xsltproc: devscripts (its Python install step) and grml2usb (amd64 only).

Still to do in PoC scope: EBCDIC (conformance `output22`, the only failing conformance test).

On 2026-11-08 we decided to build oxslt rather than contribute to libxml-rs. libxml-rs 0.1.0-alpha.60 passed 5 of 13 differential test cases and couldn't compile DocBook XSL. The evaluation and its test cases are kept outside this repository.

## PoC success criterion

Agreed 2026-11-08. Both parts are judged against `xsltproc`'s output and exit code.

1. **Byte-identical output for darktable's `tools/generate_prefs.xsl`**, run as darktable's build runs it: `xsltproc --nonet --stringparam HAVE_OPENCL 1 generate_prefs.xsl darktableconfig.xml`. Getting there requires:
   - tolerating the input's `<!DOCTYPE … SYSTEM "darktableconfig.dtd">` without processing the DTD
   - `generate-id()` values in libxslt's `id1`, `id2`, … format, which appear in the output
2. **At least 80% of the conformance tests in PoC scope: 1,116 of 1,394.**
   - The official OASIS suite is no longer downloadable; its committee is deactivated. We use Apache's xalan-test `tests/conf` instead (Apache-2.0, pinned at `5120be9`): 1,692 tests, the Lotus/Xalan tests contributed to the OASIS suite.
   - `scripts/classify-conformance.py` marks a test "later" if it uses anything on the plan's later list (`xsl:key`, `xsl:number`, import precedence, `document()`, extensions, locale-aware sorting) or needs the DTD. It also recognises a prefix other than `xsl:` bound to the XSLT namespace. 298 tests are later; the rest are in PoC scope.
   - `OXSLT_CONFORMANCE_VERBOSE=1 cargo test --test conformance -- --nocapture` lists each failing PoC test; `=later` lists the failing later tests too, with their reasons.

The first milestone is GnuCash's `iso-currencies-to-c.xsl` (58 lines). The real stylesheets, with pinned commits, are listed in `scripts/fetch-real-stylesheets.sh`. These files are GPL-licensed, so tests fetch them by script and never commit them.

Never run third-party stylesheets with `xsltproc` directly on the Mac. Run them only in the offline, read-only container (`docker run --network none --read-only …`), because xsltproc has extensions that write files (`exsl:document`) and can fetch from the network. Running them through oxslt in `cargo test` is fine **only with `--nowrite`**: oxslt has no network features, but its command line writes `exsl:document` files as xsltproc does. The conformance, DocBook and `tests/real` harnesses pass `--nowrite`; our own `exsl:document` cases (`tests/cases/35`–`38`, with a `writes` file) run in a temporary copy. Keep it that way, or move those tests into the container too.

## Publishing

- Licensed MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`), decided 2026-11-09. `README.md` is the public description; keep its claims in step with the status here.
- CI (`.github/workflows/ci.yml`) runs `cargo fmt --check`, clippy and `cargo test` on Linux and macOS, without fetched data. The xsltproc comparisons on fetched suites run locally.
- Keep private links and local paths out of the repository; it's meant to be public.

## Commands

```sh
cargo build
cargo test
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo run -- [xsltproc-style args] stylesheet.xsl input.xml

scripts/fetch-real-stylesheets.sh                    # fetch tests/real/ and record xsltproc output (needs docker + colima)
scripts/fetch-conformance.sh                         # fetch the Xalan conformance suite, classify it, record xsltproc output (~2 min)
OXSLT_UPDATE_CONFORMANCE=1 cargo test --test conformance       # rewrite tests/conformance/passing.txt
scripts/record-tree-dumps.sh                         # record libxml2's tree for every corpus file (~5 min)
scripts/fetch-docbook.sh                             # fetch DocBook man pages into tests/docbook/ and record libxml2's trees
scripts/fetch-docbook-xsl.sh                         # copy the container's DocBook XSL, DTDs and catalogs into tests/docbook-xsl/
scripts/record-docbook-html.sh                       # record xsltproc's html/docbook.xsl output for tests/docbook/ (~1 min)
cargo test --release --test docbook_html             # the step 3 gate: DocBook HTML identical to xsltproc
cargo +nightly fuzz run parse -- -max_total_time=300 # fuzz the parser for 5 minutes (also: xpath, transform, html)
scripts/record-xpath-oracle.sh                       # re-record tests/xpath/expected.txt after editing expressions.txt
scripts/bench.sh                                     # benchmark against xsltproc, natively in a container (~2.5 min)
scripts/record-expected.sh                           # re-record expected output for tests/cases/ (our own cases only)
scripts/html-diff.sh fuzz 5000 1                     # compare --html trees with xsltproc's on generated HTML (also: corpus)
scripts/gen-html-tables.py HTMLparser.c > src/html/tables.rs  # regenerate the HTML parser's tables
OXSLT_UPDATE_KNOWN_FAILURES=1 cargo test --test differential   # rewrite tests/known-failures.txt
```

Before saying a change is done, run `cargo fmt`, `cargo clippy` and `cargo test`.

## Layout

- `src/main.rs`: thin CLI, calling `oxslt::run_before_exit`, which skips freeing the compiled stylesheet since the process exits next. Library callers use `oxslt::run`.
- `src/lib.rs`: the library, with exit codes matching xsltproc's man page.
- `src/tree/`: the arena.
  - All of a document's nodes are in one `Vec`, in document order: element, then its attributes, then its children. So comparing `NodeId`s compares document order.
  - Text, attribute values and comments are ranges in one string buffer.
  - Names are interned in a `NamePool` shared by a whole transform. Common constants have fixed symbols (`NamePool::EMPTY`, `XML`, `XML_NS`, `XMLNS`, `SPACE`, `BASE`, `LANG`). `intern` checks a small front cache before its SipHash table; a cache collision only costs a miss.
- `src/parse/`: quick-xml events into the tree.
  - Encoding comes from the BOM or XML declaration and is transcoded with `encoding_rs`.
  - Line endings are normalised before tokenising.
  - Adjacent text and CDATA merge into one text node.
  - Undefined entities, unbound prefixes and content after the root are errors.
  - **Namespaces are resolved by the parser itself, not quick-xml**, because an entity's replacement text is parsed in the context of its reference.
  - Attribute values are decoded by `dtd::decode_attribute_value`: references, whitespace to spaces, and values of declared non-CDATA types collapsed, as libxml2 does.
- `src/dtd/`: DTD processing, without validation. It covers internal and external subsets, parameter entities, conditional sections, `ENTITY` (internal, external, unparsed) and `ATTLIST` (defaults, types).
  - The first declaration of a name wins.
  - Limits: 40 levels of entity nesting; 10 million characters of expansion in total, against "billion laughs".
  - Entity replacement text containing markup is parsed in place as a balanced fragment.
  - An undeclared entity is an error, or a warning that drops the reference when there's an external subset or a parameter-entity reference (XML 1.0's "Entity Declared" rules). libxml2 leaves an entity-reference node there, so the text on either side stays two text nodes. In an attribute value the reference is dropped the same way (`tests/cases/45-undeclared-entity-attribute`).
- `src/catalog/`: loading as xsltproc's entity loader does.
  - OASIS XML catalogs from `XML_CATALOG_FILES` (default `/etc/xml/catalog`), with `public`, `system`, the rewrite, suffix and delegate entries, `nextCatalog`, `group` and `xml:base`.
  - A system ID is made absolute against the referring file, mapped by the catalogs, then tried as a URI if it's still not local.
  - `--path` directories are tried with the URL's last segment. A stylesheet module found that way is named by the path found, as libxml2 names it, so `document()` calls in it resolve from that directory (`tests/cases/46-path-module-base`).
  - **There's no network access at all**, which is what `--nonet` asks for.
- **Parser options match xsltproc's.** By default the DTD is loaded and its defaults applied (`ParseOptions::load_dtd`, `dtd_attributes`). `--novalid` turns off both, including defaults from the internal subset. Stylesheets, `xsl:include`d files and inputs all go through the same loader.
  - Each element records its line, for error messages.
- `src/xpath/`: XPath 1.0.
  - The modules: `lexer` (with section 3.7's disambiguation), `parser` (recursive descent to `ast::Expr`), `eval`, and `number` (libxml2's number/string conversion).
  - Prefixes are resolved and names interned at compile time.
  - Tokens borrow from the expression text; binary operators are parsed by precedence climbing (`parser::binary_from`).
  - `NodeRef` is (document, node, namespace-node index). Its ordering is document order; namespace nodes sort after their element, before its attributes.
  - `Variables` and `Functions` are the hooks XSLT plugs into.
  - **Where libxml2 2.9 departs from the spec, oxslt follows libxml2**, with a comment at each place. Examples: number formatting (`1e+21`, `-2.147483648e+09`), namespace-axis order, the `following` axis of an attribute, `round()` and `substring()`. libxml2's path grammar is lax too: a relative location path may start with an extra `/` or `//`, so `///x` is `//x`, and a lone `/` not followed by a step is skipped (`parser::relative_path`, `tests/cases/40-xpath-extra-slashes`). libxslt's pattern compiler isn't lax, so a pattern like `///x` is a compile error (`compile::pattern_xpath`, `tests/cases/44`).
  - **Hostile input can't overflow the stack.** Nesting past `MAX_NESTING` (100) is rejected on the way in. Trees taller than `MAX_HEIGHT` (250) are rejected while being built, sized so evaluation fits on a 2 MB debug-build thread.
- `src/xslt/`: the engine.
  - `compile` turns the stylesheet tree into the IR in `ir`. That includes includes, literal-result-element stylesheets, namespace exclusion and aliasing, AVTs, and globals ordered by their dependencies.
  - `pattern` holds match patterns and default priorities.
  - `exec` runs the IR, covering template selection (highest priority, then the later rule), built-in templates, parameters, sorting, copying, and the XSLT functions.
  - **Template rules are indexed** by mode and node kind and name (`NodeKey`, `could_match`): only rules whose last step could match are tried, in the same order. `could_match` may say yes too often, never too rarely; the full match decides.
  - `output` builds result trees, keeping each element pending until its first child and fixing up namespace declarations.
  - Each result tree fragment is a `Document` of its own.
  - **A fragment is its own XPath type** (`xpath::Value::Tree`, libxml2's XPATH_XSLT_TREE), decided 2026-11-11. It works as a string, number or boolean, and libxml2 also lets `count()`, `sum()`, `name()`/`local-name()`/`namespace-uri()`, `id()`, comparisons, `key()`'s second argument, `copy-of`, libexslt's set/math/string functions and `exsl:node-set()` take it as the node-set of its root (`Value::nodes_or_tree`). A path step, predicate or union on it is a type error (the transform stops, exit 10), as are `generate-id()` and `saxon:line-number()` of it and `document()`'s second argument; `xsl:apply-templates` stops, `xsl:for-each` reports (exit 9); `document()` takes it as a string. `exsl:object-type` says "RTF" for it and "node-set" after `exsl:node-set()`. `tests/cases/59`–`61`.
  - **Equality compares libxml2's value hash too** (`eval::eq_key`, xmlXPathNodeValHash): `=` and `!=` between a node and a string or another node match only when the first two bytes of the value match as libxml2 hashes them. That only differs from the string value for a document node, which libxml2 hashes by its first element child (or first child), and an HTML document node, which hashes as 0: so the fragment `<a>1</a><b>2</b>` isn't equal to "12". `tests/cases/62`, `63`.
  - **Where libxslt is stricter than the spec, oxslt is too:** a variable redefined in the same template, an attribute after children, `--` in comments and `?>` in PIs are runtime errors (exit 9). An invalid literal name in `xsl:element` or `xsl:attribute`, or `xsl:attribute name="xmlns"`, is a compile error (exit 5).
  - **Runtime errors follow libxslt's two states** (decided 2026-11-11 from libxslt's source and probes compared with xsltproc; `tests/cases/50`–`58`). An error libxslt reports with xsltTransformError puts it in its *error state*: the transform **goes on** (later `xsl:message`s print, later `exsl:document`s run) and ends with exit 9 and no output. oxslt does that through `Transform::report` for: an attribute after children, a redefined variable (the first stays), an undefined called template, `apply-imports` without a current rule, `xsl:for-each` over a non-node-set, invalid computed names and unbound prefixes in `xsl:element`/`xsl:attribute`, `xmlns` as an attribute name or namespace, `--` in comments, `?>` in PIs, `exsl:document` errors. What sets libxslt's *stopped state* ends the transform at once with exit 10 (`ExecError::Terminated`), and wins over the error state: **any failed XPath evaluation in an instruction** (undefined variables or functions, type errors) in `value-of`, `if`, `choose`, `for-each`, `apply-templates`, `copy-of`, sort keys, variables, parameters, AVTs and globals; `apply-templates` over a non-node-set; circular globals; `xsl:message terminate="yes"`; the recursion limit. Exceptions: `xsl:number`'s value is evaluated on its own, so a failure writes nothing (an undefined variable still reports, exit 9), and sort keys aren't evaluated for fewer than two nodes. Other errors still stop oxslt with exit 9 (`ExecError::Runtime`), where libxslt may go on; only side effects after them can differ. With `-o` any error is exit 11. Across several inputs the last error's code stays.
  - `xsl:number` inserts its text even when empty, leaving an empty text node (`<a></a>`, not `<a/>`), as xsltCopyTextString does.
  - `generate-id()` gives `id1`, `id2`, … in order of first request, as libxslt does.
  - **Namespaces follow libxslt, not the spec.** `tests/cases/16-namespaces` pins these rules down:
    - A literal result element copies the declarations written on it, then its name's namespace, then the template's inherited namespaces. The inherited ones are copied only when the element sits directly in a template body; inside `xsl:if`, `xsl:for-each` or a variable they're lost.
    - `xsl:exclude-result-prefixes` on a literal element doesn't affect its own declarations.
    - `xsl:namespace-alias` changes the URI and keeps the stylesheet's prefix.
    - `xsl:attribute` chooses its prefix as `xsltGetSpecialNamespace` does (`output::ResultTree::attribute_prefix`). It asks for `ns_1` when unprefixed, and declares that even when another prefix for the URI is in scope.
    - `copy-of` copies an element's in-scope namespaces in `xmlGetNsList` order.
  - **Attribute sets follow libxslt too** (`compile::resolve_attribute_sets`): a set's own attributes come first, then the used sets' attributes whose names aren't already there. The caller's local variables are visible inside them.
  - **Import precedence follows libxslt** (`compile::assign_precedence`, `tests/cases/17`–`19`):
    - Modules are numbered by a post-order walk of the import tree. An included file's imports become the including module's, after its own.
    - Template rules compare precedence, then priority, then position.
    - `xsl:apply-imports` considers every rule of lower precedence than the current template rule's, not only the current module's imports (libxslt's `xsltNextImport`). Inside `xsl:for-each` there's no current rule: a runtime error.
    - Named templates and globals keep their highest-precedence definition. On a tie (a stylesheet and its includes), a named template's later definition wins but a global's **first** one, in document order with includes expanded in place, as libxslt's list of globals has it; xmlto's included encoding module relies on that. Two global `xsl:variable`s of one name in a stylesheet and its includes are a compile error; params, or a param and a variable, aren't (`tests/cases/41-global-ties`, `42-global-redefinition`). Attribute sets merge, the highest-precedence attributes first.
    - `cdata-section-elements` comes only from the highest-precedence module that has any. An imported unnamed `xsl:decimal-format` is ignored; a named one's first declaration wins.
    - An unknown top-level `xsl:` element is a compile error even in forwards-compatible mode, and so is a top-level element with no namespace.
    - **A module's `exclude-result-prefixes` is never popped** within an import unit: libxslt pushes it when it starts the module, so an included module's exclusions also apply to the including module's later templates. Each `xsl:import` starts afresh. `tests/cases/31-include-exclusions`.
  - `element-available()` and `function-available()` answer as libxslt does: `xsl:param`, `xsl:sort`, `xsl:when` and the like count as available elements, `xsl:key` and other declarations don't.
  - **Keys** (`xsl:key`, `key()`): each (document, key name) is indexed on first use, every node of the document tested against the key's patterns. Several definitions with one name add up; an undeclared key gives an empty node-set. A literal key name's prefix is resolved at compile time, as for `format-number`. `tests/cases/20-key-details` pins it down.
  - **libxslt can't match some `key()` and `id()` patterns** (`pattern::never_matches_in_libxslt`). When a step has two predicates, or a predicate on a step other than an element name or `*`, libxslt matches by evaluating `"//" + pattern` as XPath. With a `key(` or `id(` start that's invalid, so the pattern never matches.
  - `strip-space` and `preserve-space`: modules are asked in precedence order. Within one, for the same name, or the same `ns:*`, the **first** declaration wins (libxslt's hash table); for `*` the latest wins.
- **`document()`** (`exec::XsltFunctions::document`) follows libxslt's xsltDocumentFunction. `tests/cases/22-document` pins it down.
  - **Documents are identified by URL string**, as libxslt does (`doc->URL`). `src/uri.rs` ports libxml2's `xmlBuildURI`, so relative URLs stay relative: `document('in.xml')` from `style.xsl` is the input itself when both were given relatively.
  - The base URI is the instruction's (`Location::base`), a node's (its document's URL and `xml:base`, `Document::base_uri`), or the second argument's first node's. `document('')` parses the stylesheet file again, as a new document.
  - **Loading happens between evaluations.** XPath sees the documents immutably, so `document()` records what it needs and fails; `exec::env_eval` loads those and evaluates again. That's safe because nothing an evaluation does is visible until it succeeds.
  - Loaded documents go through the same parser options, catalogs and `xsl:strip-space` as the input. A file that can't be loaded gives an empty node-set and a warning. A fragment selects the element with that ID; other XPointer isn't supported. There's no network access: an `http:` URL fails to load.
- **Global variables are computed when first referenced**, as libxslt does, then any left over in declaration order. A reference to one not computed yet makes the evaluation fail and run again after computing it (`exec::compute_global`), with the root as context and no locals. A global that needs itself stops the transform (exit 10) when computed; it isn't a compile error.
- **Keys are indexed all at once per document**, as libxslt's `xsltInitAllDocKeys` does: the first `key()` on a document indexes every key there, highest precedence first, then in declaration order (`ss.key_order`). That order is visible. DocBook's `gid` key uses `generate-id()`, which numbers the nodes of every document a `key()` touches.
- **EXSLT** (`src/xslt/exslt.rs`), ported from libexslt 0.8.20: `exsl:node-set` and `exsl:object-type`, sets, strings and math, plus `saxon:systemId`, `saxon:line-number` and the `node-set` aliases libxslt registers. `tests/cases/23-exslt` pins them down.
  - Dates and times (`src/xslt/dates.rs`): `date:date-time` and the functions taking a date apart, ported with libexslt's parser quirks; the current time honours `SOURCE_DATE_EPOCH` (UTC). Without it libexslt uses the local zone, oxslt UTC. Durations too (2026-11-13): `date:add`, `add-duration`, `difference`, `duration`, `seconds` and `sum`, with libexslt's duration parser and formatter, month and 400-year carries, and type truncation in differences. `tests/cases/25-exslt-dates`, `64-exslt-date-durations` (2,457 results identical to xsltproc's).
  - `dyn:evaluate` compiles its string between evaluations (compiling may intern names) and evaluates it in the current context; errors give an empty node-set, but requests to load or compute something first still fail so the evaluation runs again. `tests/cases/26-dyn-evaluate`.
  - `function-available()` answers exactly as xsltproc does, from `exslt::AVAILABLE`. That includes functions oxslt doesn't implement yet (`dyn:map`, `saxon:expression`, `saxon:eval`, `saxon:evaluate` and the crypto functions); calling one is a runtime error naming it.
  - Functions that make nodes (`exsl:node-set` of a string, `str:tokenize`, `str:split`, `str:replace`, which returns a text node) build a small document and fail; `env_eval` adds it and runs the evaluation again, and the call finds it by its sequence number in the evaluation.
  - QNames in strings resolve with the instruction's in-scope namespaces (`Location::namespaces`). The compiler resolves literal ones, for `key()`, `format-number()` and both `*-available()` functions.
- `src/xslt/number.rs`: `xsl:number`'s formatting, a port of libxslt's `numbers.c`, floating-point steps included. The levels and the default count test are in `exec::count_numbers`. As in libxslt, only `format` is an attribute value template; `level`, `grouping-separator` and `grouping-size` are read as written. An unknown `level` writes nothing. `tests/cases/21-number-details` pins down the edge cases.
- `src/xslt/number_format.rs`: `format-number()`, a byte-level port of libxslt's `xsltFormatNumberConversion`. That includes its Java-style rules and the fallback to `0.0###` on an invalid pattern. `xsl:decimal-format` without a name changes the default format; `tests/cases/14-format-number` pins down 56 cases.
- `src/encoding.rs`: encoding names as libxml2 reads them. **ISO-8859-1 and US-ASCII are literal**, not windows-1252 as `encoding_rs` and the WHATWG labels have it. Everything else goes to `encoding_rs`. Unrepresentable output characters become `&#N;`.
- **The Mac's `xsltproc` isn't the reference.** Apple's libxslt 1.1.35 build rounds `format-number(9.95, '0.0')` to `9.9`, where Debian gives `10.0`. `scripts/record-expected.sh` records in the container.
- `src/serialize/`: output, following libxslt's `xsltSaveResultTo` and libxml2's writers.
  - `mod.rs` chooses the method as libxslt does. It's html when declared, or when the first element is `html` (any case, no namespace) with no non-blank text before it.
  - `mod.rs` also writes xml and text output and transcodes to the output encoding.
  - `html.rs` copies libxml2 2.9.14's HTML writer. Its element table (void, save-end-tag and inline flags for 92 elements) is extracted from `HTMLparser.c`. It also covers boolean attributes, URL escaping of `href`, `src` and `action`, and the content-type `<meta>` insertion.
  - **libxslt's `indent` has three states: unset, `no` and `yes`.** Unset counts as on for the trailing newline, for HTML formatting, and for newlines after a DOCTYPE or top-level comment.
  - An empty result writes nothing.
  - **Text nodes have kinds** (`tree::TextKind`): normal, unescaped (`disable-output-escaping`) and CDATA (`cdata-section-elements`). Each merges only with its own kind, and the kind survives copying from a fragment. XPath sees all three as ordinary text. CDATA is made only while the output method is xml or not yet known, and libxml2's HTML writer drops it.
  - An unprefixed `cdata-section-elements` name takes the default namespace, unlike other QNames.
  - `indent="yes"` for xml copies `xmlNodeDumpOutput`: two spaces per level up to 60, and any text child turns formatting off for that subtree.
  - Non-ASCII in xml attribute values becomes `&#xHH;` when `xsl:output` gives no encoding; text stays raw.
  - An empty comment writes nothing but its indentation.
  - UTF-16 output is little-endian with a BOM, written by `src/encoding.rs` because `encoding_rs` can't encode it.
  - `tests/cases/15-output-details` pins these down.
  - **The XHTML serializer** (xmlsave.c's xhtmlNodeDumpOutput) is used for xml output whose DOCTYPE has an XHTML 1.0 public or system ID: `<br />` for empty XHTML elements and `<p></p>` for others, a Content-Type `meta` added to a `head` under the root `html`, boolean attributes given their name as value, `id` from `name` and `lang`/`xml:lang` added in pairs, and `xmlns` on a bare `html`. `tests/cases/32`–`34` pin it down.
  - Not done: EBCDIC.
- **`exsl:document`** (`exec::Transform::document_element`, libxslt's xsltDocumentElem): the `href` is resolved against the file being written (the `-o` file, or an enclosing `exsl:document`'s), checked as xsltCheckWrite checks it (making missing directories), and the body written with the element's own output attributes only; the main `xsl:output`'s `cdata-section-elements` still applies. An extension element is recognised by its prefix, as libxslt's xsltCheckExtPrefix does, and inherited namespaces with an extension prefix are skipped. Errors reported this way (a refused write, a missing `href`) let the transform finish but fail it: exit 9, or 11 with `-o` (any failure with `-o` is 11, as in xsltproc). `tests/cases/35`–`38`.
- `src/xinclude.rs`: `--xinclude`, as libxml2 2.9.14 processes it, for inputs and `document()` loads: the document is rebuilt with each include replaced (text kept apart at include boundaries, as libxml2's marker nodes keep it), fallbacks, text includes, the shorthand, `element()`, `xpointer()` and `xmlns()` XPointer forms, and the `xml:base` fixup. An XInclude error makes xsltproc skip the input (exit 6). `tests/cases/27`, `28`.
- `src/html/`: `--html`, a function-by-function port of libxml2 2.9.14's HTML parser (HTMLparser.c's `htmlReadFile` path) and the tree its SAX2 handler builds. `tables.rs` (element table, auto-close pairs, entities) is generated by `scripts/gen-html-tables.py`.
  - Implied `html`, `head`, `body` and `p`, auto-closing (htmlStartClose, end-tag priorities), misplaced `html`/`head`/`body` dropped, script and style as CDATA up to `</` and a letter, `areBlanks`, names lowercased and cut at 100 characters.
  - No namespaces: an element name loses its prefix (`xmlSplitQName`), `xmlns` is an ordinary attribute. Boolean attributes without a value get their name; others stay **valueless** (`Document::is_valueless`), which copies keep and the HTML serializer writes as a bare name. `id`, and `name` on `a`, are IDs.
  - **Encoding as libxml2 finds it**: the first 4 bytes, then at the first non-ASCII byte a look-ahead for a `http-equiv` charset in what's been read (else ISO-8859-1), and `<meta>` charsets. The input is read in 4000-byte chunks at libxml2's `GROW` points and converted chunk by chunk, so a later charset only affects input not yet read, as in libxml2. Encoding names go through libxml2's own list, then glibc iconv's rules (`find_handler`).
  - After invalid UTF-8 (`xmlNextChar`'s error, recovery off) libxml2 sets `disableSAX`: text, comments and PIs are dropped from then on, elements still built. Ported.
  - Approximations: a name over 100 characters restarts at a 4000-byte read, where libxml2 restarts only if `realloc` moved its buffer; Unicode name classes stand in for libxml2's `IS_LETTER`; UCS-4 and EBCDIC input aren't detected.
  - `scripts/html-diff.sh` compares trees with xsltproc's on real HTML files and on generated broken HTML (`tests/html-diff/`); 2,752 real files and over 40,000 generated ones matched, apart from the restart approximation.
- `src/cli.rs`: xsltproc's options and exit codes.
  - Stylesheets and inputs named on the command line open through the catalogs, as libxml2's entity loader opens them; the resolved path is the document's location.
  - A stylesheet with no input is compiled and nothing transformed (exit 0), as xsltproc does. Compile errors are exit 5 with or without inputs.
  - `-` as the stylesheet or an input is standard input, as libxml2 reads it.
  - `--catalogs` loads the catalogs in `$SGML_CATALOG_FILES` (colons or blanks between them) **instead of** `XML_CATALOG_FILES` and `/etc/xml/catalog`: in libxml2 the first one loaded becomes the default catalog and stops the usual initialisation. Unset, it only warns. SGML-format catalogs aren't read. `tests/cases/43-catalogs`.
  - Command-line parameters are globals even when the stylesheet doesn't declare them, and are ignored when a module declares an `xsl:variable` of that name.
  - The transform runs on a thread with a 512 MiB stack.
  - Template recursion deeper than 3,000 stops with libxslt's message and exit 10.
- `tests/differential.rs`: runs every case in `tests/cases/` (ours, committed) and `tests/real/` (third-party, git-ignored), comparing stdout bytes and exit code with `expected/`.
  - A case's optional `requires` file lists paths it needs, such as fetched DocBook stylesheets; without them it's skipped with a note, so `cargo test` passes on a fresh clone.
  - Cases in `tests/known-failures.txt` are expected to fail.
  - The test fails on any new failure, and on a listed case that now passes.
  - When a change makes a case pass, remove it from the list.
- `tests/conformance.rs`: runs the conformance suite and prints the PoC pass rate against the 80% target.
  - `tests/conformance/passing.txt` (committed) lists every passing test.
  - The test fails if a listed test stops passing.
  - Without the fetched suite, it prints a note and passes.
- `tests/parse_corpus.rs`: parses every `.xml` and `.xsl` file in the fetched corpus.
  - It covers `tests/docbook/` too, parsed with the catalog in `XML_CATALOG_FILES`, else Homebrew's or `/etc/xml/catalog`.
  - Each file must be accepted or rejected as libxml2 does. The 2 rejections outside `docbook/` are listed with reasons. In `docbook/`, a file without a recorded tree is one libxml2 rejected, such as an entity fragment with several top-level elements.
  - Where libxml2's tree was recorded (via `tests/tree-dump.xsl`), ours must match it entry for entry. That's 3,815 files, all identical.
- `tests/docbook_html.rs`: roadmap step 3's gate. DocBook XSL's `html/docbook.xsl` over every document in `tests/docbook/`, with the reference container's own stylesheets and catalogs (`tests/docbook-xsl/`), compared byte for byte with xsltproc's output recorded in `tests/docbook-html/`. It passes with a note when those aren't fetched.
- `tests/xpath_oracle.rs`: the expressions in `tests/xpath/expressions.txt`, evaluated on `tests/xpath/doc.xml`, must match `xsltproc`'s results in `tests/xpath/expected.txt`, recorded by `scripts/record-xpath-oracle.sh`. 289 of 289 match. Add an expression there whenever XPath behaviour is in doubt.
- `tests/xpath_corpus.rs`: every `select`, `test`, `match`, `count`, `from`, `use` and `value` expression in the corpus stylesheets must compile. That's 7,221 expressions.
- `fuzz/`: cargo-fuzz targets, run with the nightly toolchain.
  - `parse`: any input parses or errors, never panics, and the tree is consistent.
  - `xpath`: any string compiles or errors, and evaluation never panics or overflows. Node-sets stay sorted.
  - `html`: any input parses as HTML without panicking, and the tree is consistent. Seed it with `tests/html-diff/gen.py`.
  - `transform`: a stylesheet, a NUL byte and an input; compiling, transforming and serialising never panic. It runs with a work budget and `ResourceLoader::none()`, which reads no files. Seed it with the conformance tests and `tests/cases` (stylesheet, NUL, input).
- `tests/dropin/`, `scripts/dropin.sh`: drop-in testing on Debian package builds (offline container, comparing wrapper as `/usr/bin/xsltproc`). Results in `target/dropin/out`.
- `tests/cli.rs`: command-line behaviour the differential cases can't express (no input file, missing stylesheet, standard input, `--catalogs` unset).
- `tests/limits.rs`: runaway transforms (exponential recursion, string doubling, nested XPath, a timeout) stop with the limit's error; ordinary ones don't notice the limits.
- `tests/common/mod.rs`: runs oxslt on a case and compares its result with the recorded output.
- `tests/reference/Dockerfile`: the `oxslt-reference` image, with Debian `xsltproc` and DocBook, used to record expected output.

## Known differences from xsltproc

- **Extension elements:** `exsl:document`, `xt:document` and `saxon:output` (with a static `file`) work; `redirect:write` (libxslt's Xalan namespace), `libxslt:debug` and `func:result` don't, though `element-available()` reports them available as xsltproc does. DocBook's chunker asks for `saxon:output` first and names chunks by the answer. `func:function` is a top-level element, so it isn't available, and isn't supported.
- **math:sin and the other transcendental functions** use the platform's libm, which can differ from glibc's in the last bit.
- **`xsl:sort` ignores `lang`.** The reference xsltproc has no locales installed, so it sorts the same way. On a system with the locale installed, xsltproc sorts by it (`strxfrm`).
- **Document order of mixed node-sets.** On a document xsltproc has indexed (`xmlXPathOrderDocElems`), libxml2's `xmlXPathCmpNodesExt` orders a text, comment or PI node by its nearest preceding element sibling. That sorts it before the descendants of that element. The comparison isn't transitive, so the result also depends on libxml2's timsort. oxslt sorts in true document order. This shows only in a node-set mixing such nodes with descendants of their preceding sibling, like `//p | //comment()`.

## Scope

- **The contract is compatibility with the `xsltproc` command line.** Match its output bytes, flag behaviour and exit codes. Build scripts depend on all three.
- **No C API for now.** Don't expose libxml2-style node structures. If a C API is added later, it goes in a separate crate and must not shape the core design.
- **Flags to support first:** `-o`/`--output`, `--param`, `--stringparam`, `--nonet`, `--novalid`, `--xinclude`, `--path`, `--html`, `--catalogs`. Profiling and timing flags can wait.
- **Exit codes** must match xsltproc's documented codes.
- **Later work:**
  - the EXSLT functions listed but not implemented (`dyn:map`, saxon's expression functions, crypto)
  - HTML output edge cases, the XHTML writer and EBCDIC

## Architecture rules

These are hard to change later. Don't go against them without asking.

1. **The source tree is read-only during a transform.** Borrow the source document immutably for the whole run. Build the result tree separately, by appending only. In libxslt this is only a convention, and breaking it caused CVE-2021-30560. Here the compiler must enforce it.
2. **Use arena trees with index IDs.** Keep each document's nodes in a `Vec`, with parent and sibling links stored as `NodeId` integers, the way roxmltree does. Don't use `Rc<RefCell<…>>` for the core tree. Node identity must stay stable, because `generate-id()` and document order depend on it.
3. **Node identity spans several documents.** `document()` loads documents at runtime, so a node reference must say which document's arena it belongs to.
4. **Intern names.** Store element and attribute names as small integer symbols, so template matching compares integers. Don't add a shared, reference-counted dictionary like libxslt's.
5. **Compile, then run.** Compile the stylesheet into an intermediate representation first, with XPath compiled to an AST or IR. Index templates by element name. Don't interpret the stylesheet's XML tree directly.
6. **Give result tree fragments and variables deliberate lifetimes.** Values that outlive a template call need an explicit design, not workarounds.
7. **Keep the core in pure Rust, with the CLI as a thin layer.** Transform logic goes in the library, not in `main.rs`.

## Security

Treat every stylesheet and input document as untrusted code. XSLT is Turing-complete.

- **No `unsafe` in the core.** If an exception is truly needed, isolate it, document why, and ask first.
- **Resource limits:**
  - Too-deep recursion must return a controlled error, never overflow the stack.
  - **Parsing has libxml2's limits** (without `XML_PARSE_HUGE`, as xsltproc runs it), so oxslt rejects what xsltproc rejects: 257 nested elements at most, 10,000,000 bytes in a text node, comment, PI or CDATA section, 50,000 in a name. An attribute value of 9,999,993 bytes fails ("Huge input lookup"); libxml2's real rule depends on its buffer position, so this one is approximate.
  - **Transforms take `exec::Limits`:** a budget of work units and a wall-clock timeout, checked in instructions and XPath evaluation alike (`Functions::tick`). Exceeding one stops the transform as the recursion limit does (exit 10). The CLI sets none, as xsltproc has none; the fuzz target and `tests/limits.rs` do.
  - **Files that aren't regular files** (pipes, `/dev/stdin`, devices) are read in chunks, at most 1 GiB, stopping at a NUL byte (`catalog::read_file`). `document('/dev/zero')` fails at once, as in xsltproc, instead of filling memory.
- **Entity expansion:** protect DTD handling against "billion laughs" style attacks.
- **No network access at all**, with or without `--nonet`. `document()` and external DTDs are classic attack paths.
- **What a stylesheet or document can read** (reviewed 2026-11-09). Like xsltproc, oxslt reads any local file named by `document()`, `xsl:include`, `xsl:import`, an external DTD or an external entity in the input (`<!ENTITY x SYSTEM "/etc/passwd">` puts the file's text in the tree), resolved through the catalogs. This is xsltproc's behaviour and DocBook builds depend on it. So: don't run untrusted stylesheets on confidential machines, and don't send their output to untrusted parties. The library can refuse every read (`ResourceLoader::none()`).
- **What oxslt writes:** the `-o` output file, and on the command line whatever `exsl:document` names, as xsltproc does: any file the user may write, creating directories, unless `--nowrite`, `--nomkdir` or `--writesubtree` restricts it. The library writes nothing unless the caller sets `Limits::writes`. An empty or failed result writes no file, not even an empty `-o` file.
- **Errors, not panics.** Malformed input must never cause a panic. Return errors instead.
- **Fuzz from the start** with cargo-fuzz: XML input, XPath parsing and whole transforms.

## Performance

- **Allocate little, by design:**
  - one arena per document
  - bump allocation (e.g. bumpalo) for temporary data within a transform, freed all at once
  - node-sets as iterators or `SmallVec`, so whole sets aren't built when they aren't needed
- **Stream output:** buffered serialisation written straight to the output.
- **Keep start-up fast.** That's what xsltproc users rely on most.
- **No micro-optimisation in the PoC.** Benchmark against xsltproc first (`scripts/bench.sh`, `tests/bench/RESULTS.md`).
- **Hashing:** maps keyed by symbols or node IDs use `tree::SymMap` (fast, and safe because those are numbered as created). Maps keyed by strings from documents or stylesheets keep std's keyed SipHash, against hash flooding.
- **Don't allocate per node.** Borrow from the source document or the input instead of copying into a String, reuse scratch buffers (the parser's attribute buffer, XPath's step buffer), and share per-element compile data (`Compiler::shared_scope`). Count allocations when touching a hot path; `tests/bench/RESULTS.md` has the counts.
- **Watch for hidden quadratic costs.** Both slow cases the benchmarks found were O(n²) work done once per node: ranking siblings to check a pattern predicate, and allocating per context node in a step. Check for this first when something is slow.

## Testing

The main testing method is **comparison with xsltproc**: run the same stylesheet and input through both, then compare the output bytes and the exit code. A test passes when they match.

- **Test cases:** the W3C/OASIS XSLT 1.0 conformance tests, plus real stylesheets from Debian packages.
- **Unit tests** for XPath, especially type conversion, document order and node-set comparison, where most bugs will be.
- Expect about as much test code as implementation code.
- Set up the comparison harness before building the engine.

## Dependencies

- **XML parser: quick-xml, used only as a tokenizer**, through its plain `Reader`. Decided 2026-11-08 after testing five parsers against libxml2 on conformance, DocBook and malformed inputs. We build our own arena tree from its events, and don't adopt any parser's tree.
- **Our own DTD layer** sits between quick-xml and the tree, because no Rust parser does what DocBook 4.x needs. It must handle:
  - the internal and external DTD subsets, the external one resolved offline through XML catalogs (`XML_CATALOG_FILES`, `--nonet`)
  - parameter entities and conditional sections (`INCLUDE`/`IGNORE`)
  - general entities, including ones whose text contains markup, with limits on expansion depth and total size
  - `ATTLIST` default values, and ID-typed attributes recorded for XPath `id()`
  - skipping DTD loading under `--novalid`, as xsltproc does
- **Gaps in quick-xml that we fill:**
  - normalise `\r\n` and `\r` to `\n` in text
  - reject references to undefined entities
  - detect the encoding from the BOM or XML declaration and transcode with `encoding_rs`
  - turn byte offsets into line and column numbers for error messages
- **XPath crate:** using an existing one could save about 2,000 lines. Check how mature it is before adopting it, and keep dependencies few.

## Open design questions

These are undecided unless marked decided. Before writing code that depends on an undecided one, raise it with the user instead of quietly picking an answer.

**Decided 2026-11-08 for questions 3 and 5:**
- Each result tree fragment is a small `Document` of its own, added to the transform's documents with its own `DocId`. `copy-of` and string conversion then work on it like any document; as an XPath value it's `Value::Tree`, which paths reject as libxml2 does (2026-11-11).
- The transform runs on a dedicated thread with a large stack. Recursion deeper than the limit stops with a controlled error, matching libxslt's `--maxdepth` default of 3,000 nested template calls.

**Decided 2026-11-08 for questions 1 and 2:** the PoC uses ordinary heap values (`Vec`, `String`) for XPath, behind the `xpath::Value` type. Bump arenas wait until benchmarks against xsltproc show they're needed. Questions 1 and 2 apply again only if arenas are adopted. **Benchmarks (2026-11-09) show no need yet:** oxslt uses about half of xsltproc's memory with plain heap values.

1. **How long does a bump arena live?** A bump arena only frees memory when it's reset. One arena for a whole long-running transform keeps growing, for example with recursive templates over a large document. Arenas probably need a smaller scope so memory is reclaimed during the run, such as one per template call or per top-level `apply-templates`. The scope still has to be chosen.
2. **Destructors in the bump arena.** `bumpalo` doesn't run `Drop`, so a `String` or `Vec` stored in it leaks its own heap memory. Use bumpalo's collection types (`bumpalo::collections`) or keep only plain data in the arena. Decide which, and apply it consistently.
3. **Where result tree fragments live.** A variable built from content, like `<xsl:variable name="x"><foo/></xsl:variable>`, outlives the expression that created it. With `exsl:node-set` it can also be queried like a document. So it can't live in a short-lived bump arena. The leading option is to treat each fragment as a small `Document` with its own arena and a `DocId`, so it fits the identity model across documents.
4. **When streaming output isn't possible.** Streaming straight to the output is the goal. Some cases need buffering first. A start tag must stay open until its first child, because `xsl:attribute` can still add attributes to it. (Adding an attribute after child content is an error in XSLT 1.0; match how xsltproc handles that.) An error partway through a transform also needs care when writing to `-o`: match whether xsltproc leaves a partial file. Decide where buffering is needed and how much.
5. **The transform needs a large stack.** Template recursion goes deeper than any XPath expression; libxslt allows 3,000 nested template calls. Run the transform on a dedicated thread with a large stack, and limit recursion depth with a controlled error, as the security rules require. Decide the stack size and depth limit when the XSLT engine starts.
