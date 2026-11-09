# Benchmarks: oxslt vs xsltproc

Measured 2026-11-09 with `scripts/bench.sh`, after roadmap steps 2 and 3, the hardening work, LTO, the allocation cuts and the hot-path work below. The run also writes `target/bench-results/summary.md`; rerun the script to refresh these numbers.

## Setup

- **Both tools run as native arm64 code** in the same Debian bookworm container (`tests/bench/Dockerfile`), on Apple Silicon in Colima's VM (6 CPUs, 8 GB). Neither runs under emulation, and there's no network.
- **xsltproc:** Debian's, libxml2 2.9.14 and libxslt 1.1.35, the same versions as the reference for every expected result.
- **oxslt:** a release build with fat LTO and one codegen unit (`[profile.release]` in `Cargo.toml`), no profile-guided optimisation, Rust 1.99.1.
- **Timing:** `hyperfine`, wall time, 2 warm-up runs, then 20 runs (5 for the big workloads). The ± column is the standard deviation.
- **Memory:** peak resident set size, from GNU `time`.
- **Correctness:** every workload's output is compared first, and it's **byte-identical for all 14**.

## Results

"Speed" is xsltproc's time divided by oxslt's: above 1 means oxslt is faster.

| Workload | Identical | xsltproc ms | oxslt ms | Speed (xsltproc time / oxslt time) | xsltproc MB | oxslt MB |
|---|---|---:|---:|---:|---:|---:|
| gnucash-iso | yes | 3.3 ± 0.2 | 1.9 ± 0.2 | 1.69x | 6.2 | 3.1 |
| gimp-authors | yes | 2.5 ± 0.1 | 1.6 ± 0.1 | 1.59x | 5.8 | 2.9 |
| darktable-prefs | yes | 10.1 ± 0.3 | 7.8 ± 0.2 | 1.30x | 7.9 | 4.6 |
| darktable-conf | yes | 7.2 ± 0.2 | 5.5 ± 0.1 | 1.32x | 7.5 | 4.4 |
| fluidsynth-html | yes | 3.9 ± 0.2 | 3.3 ± 0.1 | 1.18x | 6.8 | 3.9 |
| wireshark-values | yes | 14.5 ± 0.2 | 5.7 ± 0.1 | 2.55x | 10.5 | 5.0 |
| wireshark-hfdecl | yes | 9.8 ± 0.2 | 4.9 ± 0.2 | 2.00x | 10.8 | 5.0 |
| wireshark-hflist | yes | 6.2 ± 0.1 | 3.9 ± 0.1 | 1.60x | 10.6 | 4.9 |
| wireshark-values-FIX50SP2 | yes | 31.8 ± 0.4 | 9.2 ± 0.2 | 3.46x | 13.3 | 6.5 |
| identity-27MB | yes | 1003.4 ± 6.5 | 693.2 ± 3.5 | 1.45x | 659.0 | 356.1 |
| html-report-27MB | yes | 2459.7 ± 14.7 | 1095.3 ± 3.2 | 2.25x | 717.7 | 390.0 |
| sort-27MB | yes | 968.0 ± 1.2 | 544.4 ± 2.8 | 1.78x | 377.9 | 224.6 |
| recursion-27MB | yes | 2748.2 ± 31.0 | 1787.6 ± 6.6 | 1.54x | 336.5 | 198.1 |
| grouping-5k | yes | 554.9 ± 2.1 | 714.3 ± 0.4 | 0.78x | 14.1 | 7.5 |

**oxslt is faster on 13 of the 14 workloads and slower on one, grouping. It uses about half the memory on every one of them.**

**DocBook** (not yet a scripted workload), timed the same way with `html/docbook.xsl` and the DTD loaded through the catalog: `vipw.8` 84.2 ms for xsltproc, 65.0 ms for oxslt (1.30x); `usermod.8` 99.6 ms and 78.2 ms (1.27x); compiling alone (an empty `<article/>`) 61.7 ms and 46.5 ms (1.33x). Output identical. Compiling the stylesheet is still most of oxslt's time there.

## Workloads

**Real stylesheets, real inputs.** These are the 8 fetched by `scripts/fetch-real-stylesheets.sh`. They take milliseconds, so process start-up and parsing weigh heavily.

**Wireshark `Values.xsl` on `FIX50SP2.xml`.** The same real stylesheet on a larger real input (523 KB), from the same pinned commit.

**Synthetic workloads.** The stylesheets are in `tests/bench/workloads/`; the data comes from `scripts/bench-data.py` (deterministic, seed 42):

| Workload | What it exercises |
|---|---|
| `identity` | Parse, copy every node, serialize |
| `html-report` | Templates, attribute value templates, `xsl:choose`, `format-number`, `translate`, HTML output |
| `sort` | Two-key `xsl:sort`, numeric and text |
| `recursion` | Recursive named templates splitting strings word by word |
| `grouping` | Grouping without keys, `r[not(@cat = preceding-sibling::r/@cat)]`. Quadratic by design. |

The four large ones use a 27 MB file of 200,000 records; `grouping` uses 5,000.

## Fixed while benchmarking

The first run showed two outliers. Both were fixed without changing any output:

| Workload | First run | Now | Cause and fix |
|---|---:|---:|---|
| darktable-prefs | 128.6 ms (0.08×) | 8.5 ms (1.19×) | **Pattern matching was quadratic.** To match `dtconfig[type='string']`, oxslt evaluated the predicate on all 609 sibling `dtconfig` elements to rank the node among them. A predicate that can't depend on position is now checked on the node alone (`xpath::is_position_independent`), as libxslt does. |
| grouping-5k | 1,724 ms (0.32×) | 771 ms (0.73×) | **Allocation in the inner loop.** Node-set comparison copied every string value and compared pairwise. Now it borrows values (`Env::string_value_cow`) and scans directly when one side is small, hashing otherwise. Steps reuse one buffer instead of allocating per context node, and skip sorting when the order is already right. |

## Open

- **`grouping` is still 1.4× slower than libxml2.** The remaining cost is materialising `preceding-sibling::r` and its attributes, about 12.5 million nodes, for every record. Comparing while walking the axis, without building the node-set, would remove most of it.
- **No PGO yet.**
- **Variables are copied on every reference**, but that costs less than it looks: about 8% of recursion (`Vars::get`) and under 1% of a DocBook page. Making `Value`'s strings and node-sets `Rc` was tried (2026-11-09) and made things slower: recursion +7%, darktable +15%, because every computed string and node-set then needs an extra allocation, and values are created far more often than variables are read. Borrowing variables where they're function arguments or comparison operands would avoid both costs, for at most about 8% on recursion.
- **DocBook isn't a scripted workload yet.** Its time is mostly compiling the stylesheet, and what's left there is spread thin: quick-xml tokenising and validating UTF-8, opening 55 files, allocating the IR. Each page uses only 5–7% of DocBook's 2,059 templates; compiling bodies on first use, with an up-front syntax check for xsltproc's errors, is the next step if needed.

## LTO

Measured head to head, the same code built both ways in the same container, median of 8–40 runs. Output is identical. Fat LTO with one codegen unit is faster on every workload and makes the binary 17% smaller (1.62 MB vs 1.97 MB). An incremental release build takes 17 s instead of 7 s; debug builds and `cargo test` are unaffected.

| Workload | No LTO ms | LTO ms | Speed-up |
|---|---:|---:|---:|
| gnucash-iso | 2.1 | 2.1 | 0.99x |
| darktable-prefs | 9.9 | 9.6 | 1.04x |
| wireshark-values | 7.6 | 7.4 | 1.02x |
| wireshark-values-FIX50SP2 | 12.4 | 12.0 | 1.04x |
| identity-27MB | 1,023 | 1,011 | 1.01x |
| html-report-27MB | 1,493 | 1,463 | 1.02x |
| sort-27MB | 709 | 687 | 1.03x |
| recursion-27MB | 2,561 | 2,359 | 1.09x |
| grouping-5k | 863 | 795 | 1.09x |
- **Bump arenas, CLAUDE.md open questions 1–2.** Not needed for these numbers: oxslt already uses about half of xsltproc's memory with plain heap values.

## Fewer allocations

Counted with a counting global allocator for oxslt and an `LD_PRELOAD` malloc counter for xsltproc (same container). "Before" is just after LTO; output is identical.

| Workload | xsltproc | oxslt before | oxslt now |
|---|---:|---:|---:|
| gnucash-iso | 15.8k | 10.9k | 3.1k |
| gimp-authors | 10.7k | 12.0k | 4.5k |
| darktable-prefs | 127k | 114k | 57k |
| wireshark-values | 106k | 105k | 34k |
| parse only, 27 MB | 3.2M | 2.2M | 157 |
| identity-27MB | 12.8M | 9.0M | 2.2M |
| html-report-27MB | 20.9M | 15.9M | 8.3M |
| sort-27MB | 11.0M | 8.8M | 4.0M |
| recursion-27MB | 55.7M | 55.3M | 39.2M |
| grouping-5k | 4.9M | 2.6M | 1.5M |
| DocBook `vipw.8` | 416k | 553k | 254k |
| DocBook `usermod.8` | 648k | 696k | 367k |

What changed:
- **Parser:** element names are borrowed, not copied; attribute names and decoded values go into one reused buffer; duplicate attributes are checked by oxslt instead of quick-xml, whose check allocated per element. Parsing now allocates only when the tree's arrays grow.
- **Serializer:** names are written straight to the output, not formatted into a String first; HTML element lookup no longer lowercases into a new String.
- **`copy-of`:** text, comments and PIs are copied straight from the source document to the output.
- **XPath:** a path starting at the context node or the root doesn't allocate for its start, and a step from one context node hands over its scratch buffer as the result; string arguments to core functions are borrowed when they already are strings.
- **Stylesheet compiler:** in-scope namespace lists, their prefixes for XPath, element names and file names are shared between instructions instead of rebuilt for each. That halves DocBook's allocations; compiling `html/docbook.xsl` dominates a DocBook run.

## Hot paths

Profiled with macOS `sample`, oxslt run in-process in a loop. Before these changes, template selection tried every rule of the mode in order, about 1,600 for DocBook: 23% of a DocBook run and 18% of identity.

- **Template rules are indexed** (`exec::Transform::find_template`). For each mode and node kind and name (`NodeKey`), the candidates are the rules whose last step could match it (`could_match`), kept in priority order, built on first use. During development a check compared every choice with the full scan across all tests, the DocBook gate and 512k fuzzed transforms; none differed. Template selection is now 2% of a DocBook run.
- **The output builder uses fixed symbols** for `""`, `xml`, `xmlns` and the XML namespace (`NamePool::EMPTY` and friends) instead of interning them for every element.
- **The work budget costs one test per tick** when no limit is set, as on the command line.
- **Maps keyed by symbols or node IDs use `tree::SymHasher`**, an Fx-style hash. Those keys are numbered as they're created, so a document can't make them collide. Maps keyed by strings keep SipHash.

| Workload | Before (ms) | After (ms) |
|---|---:|---:|
| DocBook `usermod.8`, in-process loop | 76 | 59 |
| identity-27MB | 910 | 703 |
| html-report-27MB | 1,331 | 1,112 |
| recursion-27MB | 1,937 | 1,801 |
| gnucash-iso | 1.8 | 1.7 |

## Compiling stylesheets

DocBook's `html/docbook.xsl` is 55 modules, 1.4 MB, 2,059 templates and about 3,000 XPath expressions, compiled on every run; a page uses 104–142 of the templates. Measured head to head (the previous commit built in a worktree, same machine, alternating runs), compiling went from 29.9 to 25.4 ms (−15%); the binary also skips freeing the compiled stylesheet at exit. In the container, compile-only runs went from 57.5 to 46.5 ms.

- **XPath tokens borrow from the expression** instead of copying each name into a String, and the parser no longer clones tokens to look ahead.
- **Binary operators use precedence climbing**, one loop instead of six nested levels a simple name went through. The old and new parsers give identical trees and errors for all 11,095 distinct expressions in DocBook, the conformance suite and the tests.
- **Namespace lists and `xml:space` are cached per element** in each module (`Doc::scope_memo`, `space_memo`): an element that declares nothing shares its parent's.
- **Constant names have fixed symbols** (`NamePool::SPACE`, `BASE`, `LANG`), and the four functions whose QName arguments the compiler resolves are interned once.
- **`NamePool::intern` has a front cache** of 512 slots, picked by a fast hash and confirmed by comparing the string. Collisions only cause misses, which fall back to the SipHash table, so it can't be made slower than before.
- **Text events check for `]` before searching for `]]>`.**
- **The binary doesn't free the compiled stylesheet and names** (`oxslt::run_before_exit`); the library's `run` still does.

## Compiling template bodies on first use: measured, not done

Decided 2026-11-09 to stop performance work here. Skipping template bodies altogether halves DocBook's compile time (25.4 to 13.5 ms in-process), about 28% of a page, which is the ceiling. But xsltproc reports compile errors in every template, used or not, before any output, so bodies still need checking up front. The three ways considered:

- **A separate quick checker** (about −15% per page): a second implementation of every compile check, which can drift from the compiler.
- **A full compile on a second thread just for errors** (about −20–25% wall time): exact by construction, but about 35% more CPU per page, which makes parallel builds (`make -j`) slower.
- **Compiling only XPath on first use**, syntax-checked by the same parser (about −5%): safe but small.

None was worth its cost while oxslt is already about 1.3x xsltproc's speed on DocBook.
