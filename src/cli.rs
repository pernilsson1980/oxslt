//! The xsltproc-compatible command line.
//!
//! The transform runs on a dedicated thread with a large stack, because
//! template recursion can go deep; recursion past the limit stops with a
//! controlled error instead of overflowing.

use crate::xslt::exec::WriteCheck;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::catalog::ResourceLoader;
use crate::exit;
use crate::parse::{ParseOptions, parse_with};
use crate::tree::NamePool;
use crate::xpath::ast::Name;
use crate::xslt::compile::{StylesheetError, compile_with};
use crate::xslt::exec::{ExecError, Param, Transform};

/// Stack for the transform thread. On 64-bit systems this is reserved
/// address space; memory is only used as the stack actually grows.
const TRANSFORM_STACK: usize = 512 << 20;

/// xsltproc's usage text (`usage()` in xsltproc.c), printed to standard
/// output after an unknown option or with no arguments, word for word:
/// build tools capture it.
fn usage(program: &str) -> String {
    format!(
        "Usage: {program} [options] stylesheet file [file ...]
   Options:
\t--version or -V: show the version of libxml and libxslt used
\t--verbose or -v: show logs of what's happening
\t--output file or -o file: save to a given file
\t--timing: display the time used
\t--repeat: run the transformation 20 times
\t--debug: dump the tree of the result instead
\t--dumpextensions: dump the registered extension elements and functions to stdout
\t--novalid skip the DTD loading phase
\t--nodtdattr do not default attributes from the DTD
\t--noout: do not dump the result
\t--maxdepth val : increase the maximum depth (default {})
\t--maxvars val : increase the maximum variables (default {})
\t--maxparserdepth val : increase the maximum parser depth
\t--huge: relax any hardcoded limit from the parser
\t             fixes \"parser error : internal error: Huge input lookup\"
\t--seed-rand val : initialize pseudo random number generator with specific seed
\t--html: the input document is(are) an HTML file(s)
\t--encoding: the input document character encoding
\t--param name value : pass a (parameter,value) pair
\t       name is a QName or a string of the form {{URI}}NCName.
\t       value is an UTF8 XPath expression.
\t       string values must be quoted like \"'string'\"
 or\t       use stringparam to avoid it
\t--stringparam name value : pass a (parameter, UTF8 string value) pair
\t--path 'paths': provide a set of paths for resources
\t--nonet : refuse to fetch DTDs or entities over network
\t--nowrite : refuse to write to any file or resource
\t--nomkdir : refuse to create directories
\t--writesubtree path : allow file write only with the path subtree
\t--catalogs : use SGML catalogs from $SGML_CATALOG_FILES
\t             otherwise XML Catalogs starting from 
\t         file:///etc/xml/catalog are activated by default
\t--xinclude : do XInclude processing on document input
\t--xincludestyle : do XInclude processing on stylesheets
\t--load-trace : print trace of all external entites loaded
\t--profile or --norman : dump profiling information 

Project libxslt home page: https://gitlab.gnome.org/GNOME/libxslt
",
        crate::xslt::exec::MAX_DEPTH,
        crate::xslt::exec::MAX_VARS
    )
}

/// What `--version` prints: the versions of the libraries xsltproc was
/// built with, as Debian bookworm's xsltproc prints them. oxslt answers as
/// the xsltproc it replaces, since build scripts check these numbers.
const VERSION: &str = "Using libxml 20914, libxslt 10135 and libexslt 820
xsltproc was compiled against libxml 20914, libxslt 10135 and libexslt 820
libxslt 10135 was compiled against libxml 20914
libexslt 820 was compiled against libxml 20914
";

/// What `--dumpextensions` prints: libxslt's registered extensions, in the
/// order of its hash tables, as xsltproc prints them. It lists everything
/// `function-available()` and `element-available()` report.
const EXTENSIONS: &str = include_str!("cli/extensions.txt");

/// xsltproc's MAX_PARAMETERS: names and values together.
const MAX_PARAMETERS: usize = 64;
/// xsltproc's MAX_PATHS.
const MAX_PATHS: usize = 64;

#[derive(Default)]
struct Options {
    output: Option<PathBuf>,
    /// What exsl:document may write: xsltproc's default, or as
    /// `--nowrite`, `--nomkdir` and `--writesubtree` restrict it. Later
    /// options override earlier ones, as they replace libxslt's checks.
    writes: crate::xslt::exec::Writes,
    /// `--xinclude`: process XIncludes in the inputs and in documents
    /// `document()` loads.
    xinclude: bool,
    /// `--xincludestyle`: process XIncludes in the stylesheet document, not
    /// in its imports and includes.
    xinclude_style: bool,
    /// `--novalid`: no external DTD and no default attributes.
    novalid: bool,
    /// `--nodtdattr`: the external DTD, but no default attributes.
    nodtdattr: bool,
    /// `--html`: the inputs are HTML, parsed as libxml2's HTML parser does.
    html: bool,
    /// `--noout`: no result on standard output (with `-o` the file is
    /// still written, as xsltproc ignores `--noout` there).
    noout: bool,
    /// `--encoding`: the inputs' encoding (not the stylesheet's).
    encoding: Option<String>,
    /// `--path`: directories to search when a DTD or entity fails to load.
    paths: Vec<PathBuf>,
    /// `--catalogs`: the files in $SGML_CATALOG_FILES, used instead of
    /// XML_CATALOG_FILES (libxml2 then never reads that).
    catalogs: Option<Vec<String>>,
    params: Vec<(String, ParamArg)>,
    /// `--maxdepth`, `--maxvars`: libxslt's recursion limits.
    max_depth: Option<usize>,
    max_vars: Option<usize>,
    /// `--maxparserdepth`: libxml2's xmlParserMaxDepth.
    max_parser_depth: Option<usize>,
    /// `--huge`: XML_PARSE_HUGE.
    huge: bool,
    /// The stylesheet, and every argument after it: the inputs.
    stylesheet: Option<PathBuf>,
    inputs: Vec<PathBuf>,
}

enum ParamArg {
    String(String),
    Expr(String),
}

/// `sscanf(s, "%d", &v)`: blanks, a sign and digits; None without digits.
fn c_int(s: &str) -> Option<i64> {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let v = t[..digits].parse::<i64>().unwrap_or(i64::MAX);
    Some(if neg { -v } else { v })
}

/// A positive `%d` value, as xsltproc accepts for its limits.
fn positive(s: Option<&String>) -> Option<usize> {
    c_int(s?)
        .filter(|&v| v > 0 && v <= i64::from(i32::MAX))
        .and_then(|v| usize::try_from(v).ok())
}

/// xsltproc's `main`: a first pass over the arguments reads the options
/// wherever they are, stopping at `-`; a second finds the stylesheet, the
/// first argument that isn't an option (or is `-`), skipping option values,
/// and takes every argument after it as an input, options included, as
/// xsltproc does. A missing option value is mostly ignored, as xsltproc
/// reads a NULL there.
fn parse_args(
    args: &[String],
    program: &str,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<Options, i32> {
    if args.is_empty() {
        let _ = write!(stdout, "{}", usage(program));
        return Err(exit::NO_ARGUMENT);
    }
    let mut o = Options {
        writes: crate::xslt::exec::Writes::allowed(),
        ..Options::default()
    };
    // A parameter with no name ends libxslt's list there.
    let mut params_ended = false;
    let mut nb_params = 0;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-" {
            break;
        }
        if !a.starts_with('-') {
            i += 1;
            continue;
        }
        let opt = a.strip_prefix("--").unwrap_or(&a[1..]);
        match opt {
            "debug" => {
                let _ = writeln!(stderr, "{a} isn't supported by oxslt");
                return Err(exit::UNKNOWN_OPTION);
            }
            "v" | "verbose" if a != "--v" => {}
            "o" | "output" if a != "--o" => {
                i += 1;
                o.output = args.get(i).map(PathBuf::from);
            }
            "V" | "version" if a != "--V" => {
                let _ = write!(stdout, "{VERSION}");
            }
            "repeat" | "timing" | "profile" | "norman" | "nodict" | "nonet" | "load-trace" => {}
            "novalid" => o.novalid = true,
            "nodtdattr" => o.nodtdattr = true,
            "noout" => o.noout = true,
            "html" => o.html = true,
            "encoding" => {
                i += 1;
                o.encoding = args.get(i).cloned();
            }
            "nowrite" => {
                o.writes.files = WriteCheck::Refuse;
                o.writes.mkdir = false;
            }
            "nomkdir" => o.writes.mkdir = false,
            "writesubtree" => {
                i += 1;
                o.writes.files = match args.get(i) {
                    Some(p) => WriteCheck::Subtree(p.clone()),
                    None => WriteCheck::Refuse,
                };
            }
            "path" => {
                // Directories separated by spaces or colons, as parsePath splits them.
                i += 1;
                for p in args.get(i).iter().flat_map(|l| l.split([' ', ':'])) {
                    if p.is_empty() {
                        continue;
                    }
                    if o.paths.len() >= MAX_PATHS {
                        let _ = writeln!(stderr, "MAX_PATHS reached: too many paths");
                        break;
                    }
                    o.paths.push(PathBuf::from(p));
                }
            }
            // libxml2's xmlLoadCatalogs on $SGML_CATALOG_FILES. Called before
            // any catalog is used, the first file it loads becomes the default
            // catalog and marks catalogs initialised, so XML_CATALOG_FILES and
            // /etc/xml/catalog are never read; the other files are added after
            // it. Files are separated by colons or blanks.
            "catalogs" => match std::env::var("SGML_CATALOG_FILES") {
                Ok(v) => {
                    let files = v
                        .split([':', ' ', '\t', '\n', '\r'])
                        .filter(|f| !f.is_empty())
                        .map(str::to_string);
                    o.catalogs.get_or_insert_with(Vec::new).extend(files);
                }
                Err(_) => {
                    let _ = writeln!(stderr, "Variable $SGML_CATALOG_FILES not set");
                }
            },
            "xinclude" => o.xinclude = true,
            "xincludestyle" => o.xinclude_style = true,
            "param" | "stringparam" => {
                let name = args.get(i + 1);
                let value = args.get(i + 2);
                i += 2;
                if opt == "stringparam" {
                    let v = value.map_or("", String::as_str);
                    if v.contains('"') && v.contains('\'') {
                        let _ = writeln!(
                            stderr,
                            "stringparam contains both quote and double-quotes !"
                        );
                        return Err(exit::STRING_PARAM_HAS_BOTH_QUOTES);
                    }
                    if let Some(name) = name.filter(|_| !params_ended) {
                        o.params
                            .push((name.clone(), ParamArg::String(v.to_string())));
                    } else {
                        params_ended = true;
                    }
                } else {
                    match (name, value) {
                        (Some(n), Some(v)) if !params_ended => {
                            o.params.push((n.clone(), ParamArg::Expr(v.clone())))
                        }
                        _ => params_ended = true,
                    }
                }
                nb_params += 2;
                if nb_params >= MAX_PARAMETERS {
                    let _ = writeln!(stderr, "too many params increase MAX_PARAMETERS ");
                    return Err(exit::TOO_MANY_PARAMETERS);
                }
            }
            "maxdepth" | "maxparserdepth" => {
                i += 1;
                if i == args.len() {
                    let what = if opt == "maxdepth" {
                        "XSLT maxdepth"
                    } else {
                        "XML maxparserdepth"
                    };
                    let _ = writeln!(stderr, "{what} value not specified!");
                    return Err(exit::TOO_MANY_PARAMETERS);
                }
                if let Some(v) = positive(args.get(i)) {
                    if opt == "maxdepth" {
                        o.max_depth = Some(v);
                    } else {
                        o.max_parser_depth = Some(v);
                    }
                }
            }
            "maxvars" => {
                i += 1;
                if let Some(v) = positive(args.get(i)) {
                    o.max_vars = Some(v);
                }
            }
            // math:random isn't seeded by it in oxslt.
            "seed-rand" => i += 1,
            "huge" => o.huge = true,
            "dumpextensions" => {
                let _ = write!(stdout, "{EXTENSIONS}");
            }
            _ => {
                let _ = writeln!(stderr, "Unknown option {a}");
                let _ = write!(stdout, "{}", usage(program));
                return Err(exit::UNKNOWN_OPTION);
            }
        }
        i += 1;
    }
    // The second pass.
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let opt = a.strip_prefix("--").unwrap_or(a.get(1..).unwrap_or(""));
        if a.starts_with('-') && a != "-" {
            i += match opt {
                "maxdepth" | "maxvars" | "maxparserdepth" | "seed-rand" | "o" | "output"
                | "encoding" | "writesubtree" | "path"
                    if a != "--o" =>
                {
                    2
                }
                "param" | "stringparam" => 3,
                _ => 1,
            };
            continue;
        }
        o.stylesheet = Some(PathBuf::from(a));
        o.inputs = args[i + 1..].iter().map(PathBuf::from).collect();
        break;
    }
    Ok(o)
}

fn display(p: &Path) -> String {
    p.display().to_string()
}

/// `exiting`: the process exits right after, so don't free what the
/// transform built (see `crate::run_before_exit`). `program` is the name
/// the usage text shows.
pub fn run(
    args: &[OsString],
    program: &str,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    exiting: bool,
) -> i32 {
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let options = match parse_args(&args, program, stdout, stderr) {
        Ok(o) => o,
        Err(code) => return code,
    };
    // No stylesheet (only options): nothing to do, as in xsltproc.
    if options.stylesheet.is_none() {
        let _ = stdout.flush();
        return exit::SUCCESS;
    }
    // Output and messages are collected on the transform thread and written here.
    let worker = std::thread::Builder::new()
        .name("transform".into())
        .stack_size(TRANSFORM_STACK)
        .spawn(move || {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let code = transform_all(&options, &mut out, &mut err, exiting);
            (code, out, err)
        });
    let (code, out, err) = match worker.map(|h| h.join()) {
        Ok(Ok(r)) => r,
        _ => {
            let _ = writeln!(stderr, "internal error: the transform thread failed");
            return exit::INTERNAL_ERROR;
        }
    };
    let _ = stderr.write_all(&err);
    if let Err(e) = stdout.write_all(&out).and_then(|_| stdout.flush()) {
        let _ = writeln!(stderr, "failed to write the result: {e}");
        return exit::OUTPUT_WRITE_FAILED;
    }
    code
}

fn transform_all(o: &Options, out: &mut Vec<u8>, err: &mut Vec<u8>, exiting: bool) -> i32 {
    let style_path = o.stylesheet.as_ref().expect("checked in run");
    let inputs = &o.inputs;
    let mut names = NamePool::new();
    // xsltproc's parser options: entities substituted, the DTD loaded and its
    // defaults applied, unless --novalid (neither) or --nodtdattr (no
    // defaults). Catalogs come from the environment, or from
    // $SGML_CATALOG_FILES with --catalogs. `document()` processes XIncludes
    // with --xinclude only: xsltSetCtxtParseOptions resets the default
    // --xincludestyle sets.
    let mut options = ParseOptions {
        load_dtd: !o.novalid,
        dtd_attributes: !o.novalid && !o.nodtdattr,
        xinclude: o.xinclude,
        ..ParseOptions::default()
    };
    // libxml2's xmlParserMaxDepth allows that many open elements plus one;
    // oxslt's trees go no deeper than the default.
    if let Some(d) = o.max_parser_depth {
        options.max_depth = d.saturating_add(1).min(options.max_depth);
    }
    // XML_PARSE_HUGE: libxml2's XML_MAX_HUGE_LENGTH for text and the
    // entity check, and names up to XML_MAX_TEXT_LENGTH.
    if o.huge {
        options.max_text = 1_000_000_000;
        options.max_name = 10_000_000;
        options.max_entity_expansion = 1_000_000_000;
    }
    // --encoding applies to the inputs only, not to what they load.
    let input_options = ParseOptions {
        encoding: o.encoding.clone(),
        ..options.clone()
    };
    let mut loader = ResourceLoader::from_env(o.paths.clone());
    if let Some(files) = &o.catalogs {
        loader.catalog = crate::catalog::Catalog::new(files.clone());
    }
    let (style_bytes, style_loc) = match open_argument(&loader, style_path) {
        Ok(b) => b,
        Err(e) => {
            let _ = writeln!(err, "cannot parse {}: {e}", display(style_path));
            return exit::STYLESHEET_PARSE_FAILED;
        }
    };
    let style_doc = match parse_with(
        &style_bytes,
        &mut names,
        &options,
        &loader,
        &Some(style_loc.clone()),
    ) {
        Ok(p) => {
            for w in &p.warnings {
                let _ = writeln!(err, "warning: {w}");
            }
            p.document
        }
        Err(e) => {
            let _ = writeln!(
                err,
                "{}:{}: parser error : {}\ncannot parse {}",
                display(&style_loc),
                e.line,
                e.message,
                display(style_path)
            );
            return exit::STYLESHEET_PARSE_FAILED;
        }
    };
    // --xincludestyle: XIncludes in the stylesheet document (not its
    // modules), processed as XSLT_PARSE_OPTIONS asks; xsltproc ignores
    // the result, so errors only print.
    let style_doc = if o.xinclude_style {
        let mut style_doc = style_doc;
        if style_doc.url.is_none() {
            style_doc.url = Some(style_loc.to_string_lossy().into_owned());
        }
        let processed =
            crate::xinclude::process(style_doc, &mut names, &loader, &xinclude_options(&options));
        for m in processed.warnings.iter().chain(&processed.errors) {
            let _ = writeln!(err, "{m}");
        }
        processed.document
    } else {
        style_doc
    };
    // An <?xml-stylesheet?> before the root names the stylesheet that
    // transforms this document (xsltLoadStylesheetPI); xsltproc then
    // ignores any other arguments. If it can't be loaded, the document is
    // compiled as the stylesheet after all.
    if let Some(href) = stylesheet_pi(&style_doc, &names)
        && let Some((ss_doc, ss_loc)) = load_pi_stylesheet(
            &href, &style_doc, &style_loc, &mut names, &loader, &options, err,
        )
    {
        let mut warnings = Vec::new();
        let compiled = compile_with(
            &ss_doc,
            &ss_loc,
            &mut names,
            &loader,
            &options,
            &mut warnings,
        );
        for w in &warnings {
            let _ = writeln!(err, "warning: {w}");
        }
        let stylesheet = match compiled {
            Ok(s) => s,
            Err(StylesheetError::Parse(m)) => {
                let _ = writeln!(err, "cannot parse {}: {m}", display(&ss_loc));
                return exit::STYLESHEET_PARSE_FAILED;
            }
            Err(StylesheetError::Compile(errors)) => {
                for e in errors {
                    let _ = writeln!(err, "{e}");
                }
                return exit::STYLESHEET_ERROR;
            }
        };
        let code = transform_one(
            o,
            &stylesheet,
            &mut names,
            &loader,
            &options,
            style_doc,
            &style_loc,
            style_path,
            out,
            err,
        )
        .unwrap_or(exit::SUCCESS);
        if exiting {
            std::mem::forget(stylesheet);
            std::mem::forget(names);
        }
        return code;
    }
    let mut include_warnings = Vec::new();
    let compiled = compile_with(
        &style_doc,
        &style_loc,
        &mut names,
        &loader,
        &options,
        &mut include_warnings,
    );
    for w in &include_warnings {
        let _ = writeln!(err, "warning: {w}");
    }
    let stylesheet = match compiled {
        Ok(s) => s,
        Err(StylesheetError::Parse(m)) => {
            let _ = writeln!(err, "cannot parse {}: {m}", display(style_path));
            return exit::STYLESHEET_PARSE_FAILED;
        }
        Err(StylesheetError::Compile(errors)) => {
            for e in errors {
                let _ = writeln!(err, "{e}");
            }
            // xsltproc exits 5 on a stylesheet with errors (cur->errors),
            // with or without inputs.
            return exit::STYLESHEET_ERROR;
        }
    };

    let mut code = exit::SUCCESS;
    for input in inputs {
        let (bytes, input_loc) = match open_argument(&loader, input) {
            Ok(b) => b,
            Err(e) => {
                let _ = writeln!(err, "unable to parse {}: {e}", display(input));
                code = exit::DOCUMENT_ERROR;
                continue;
            }
        };
        // htmlReadFile never fails: errors are reported and the tree is
        // whatever the parser built.
        let parsed = if o.html {
            let p = crate::html::parse_encoded(
                &bytes,
                &mut names,
                &display(&input_loc),
                o.encoding.as_deref(),
            );
            Ok(crate::parse::Parsed {
                document: p.document,
                warnings: p.warnings,
            })
        } else {
            parse_with(
                &bytes,
                &mut names,
                &input_options,
                &loader,
                &Some(input_loc.clone()),
            )
        };
        let source = match parsed {
            Ok(p) => {
                for w in &p.warnings {
                    let _ = writeln!(err, "warning: {w}");
                }
                p.document
            }
            Err(e) => {
                let _ = writeln!(
                    err,
                    "{}:{}: parser error : {}\nunable to parse {}",
                    display(&input_loc),
                    e.line,
                    e.message,
                    display(input)
                );
                code = exit::DOCUMENT_ERROR;
                continue;
            }
        };
        if let Some(c) = transform_one(
            o,
            &stylesheet,
            &mut names,
            &loader,
            &options,
            source,
            &input_loc,
            input,
            out,
            err,
        ) {
            code = c;
        }
    }
    if exiting {
        // Not unsafe: leaking is safe. The process is about to exit.
        std::mem::forget(stylesheet);
        std::mem::forget(names);
    }
    code
}

/// Transforms one parsed input and writes or collects its result, as
/// xsltproc's xsltProcess does. Returns the exit code to keep, if any.
#[allow(clippy::too_many_arguments)]
fn transform_one(
    o: &Options,
    stylesheet: &crate::xslt::ir::Stylesheet,
    names: &mut NamePool,
    loader: &ResourceLoader,
    options: &ParseOptions,
    source: crate::tree::Document,
    input_loc: &Path,
    input: &Path,
    out: &mut Vec<u8>,
    err: &mut Vec<u8>,
) -> Option<i32> {
    let mut code = None;
    // An invalid --param expression fails this input's transform as a
    // failed evaluation does: the transform stops (exit 10).
    let params = match global_params(o, names, err) {
        Ok(p) => p,
        Err(()) => {
            let _ = writeln!(err, "no result for {}", display(input));
            return Some(if o.output.is_some() {
                exit::OUTPUT_WRITE_FAILED
            } else {
                exit::TERMINATED_BY_MESSAGE
            });
        }
    };
    let mut source = source;
    source.url = Some(input_loc.to_string_lossy().into_owned());
    // xsltproc processes XIncludes before transforming; any XInclude
    // error skips the input with exit 6, and no message of its own.
    if o.xinclude {
        let processed = crate::xinclude::process(source, names, loader, &xinclude_options(options));
        for m in processed.warnings.iter().chain(&processed.errors) {
            let _ = writeln!(err, "{m}");
        }
        if !processed.errors.is_empty() {
            return Some(exit::DOCUMENT_ERROR);
        }
        source = processed.document;
    }
    match Transform::run(
        stylesheet,
        names,
        loader,
        options,
        source,
        params,
        crate::xslt::exec::Limits {
            writes: crate::xslt::exec::Writes {
                output_file: o.output.as_ref().map(|p| p.to_string_lossy().into_owned()),
                ..o.writes.clone()
            },
            max_depth: o.max_depth,
            max_vars: o.max_vars,
            stack: Some(TRANSFORM_STACK / 8 * 7),
            ..Default::default()
        },
        err,
    ) {
        Ok(result) => {
            let bytes = crate::serialize::serialize(result, names, &stylesheet.output);
            match &o.output {
                // libxslt checks the file as it checks exsl:document's
                // (xsltCheckWrite, making missing directories); a refusal
                // is an error (exit 9) but the result is saved anyway.
                // Each input's result replaces the file. An empty result
                // writes nothing, not even an empty file:
                // xsltSaveResultToFilename returns before opening it.
                Some(path) => {
                    let mut refusals = Vec::new();
                    let checked = crate::xslt::exec::check_write(
                        &o.writes,
                        &path.to_string_lossy(),
                        &mut refusals,
                    );
                    for r in &refusals {
                        let _ = writeln!(err, "runtime error\n{r}");
                    }
                    if checked == 0 {
                        let _ = writeln!(
                            err,
                            "runtime error\nxsltApplyStylesheet: forbidden to save to {}",
                            display(path)
                        );
                        code = Some(exit::INTERNAL_ERROR);
                    } else if checked < 0 {
                        let _ = writeln!(
                            err,
                            "runtime error\nxsltApplyStylesheet: saving to {} may not be possible",
                            display(path)
                        );
                        code = Some(exit::INTERNAL_ERROR);
                    }
                    // With a declared html method and a DOCTYPE, libxslt
                    // makes the result with that DOCTYPE in it
                    // (htmlNewDocNoDtD), so it's never empty: the file is
                    // made even when nothing else is written (mplayer's
                    // chunked builds leave an empty -o file).
                    let html_doctype = stylesheet.output.method == crate::xslt::ir::Method::Html
                        && (stylesheet.output.doctype_public.is_some()
                            || stylesheet.output.doctype_system.is_some());
                    if (!bytes.is_empty() || html_doctype)
                        && let Err(e) = std::fs::write(path, &bytes)
                    {
                        let _ = writeln!(err, "failed to write the result: {e}");
                        code = Some(exit::OUTPUT_WRITE_FAILED);
                    }
                }
                None if o.noout => {}
                None => out.extend(bytes),
            }
        }
        Err(e) => {
            let (msg, mut c) = match e {
                ExecError::Runtime(m) => (m, exit::INTERNAL_ERROR),
                ExecError::Terminated(m) => (m, exit::TERMINATED_BY_MESSAGE),
            };
            // With -o, xsltproc has no result to write, and saving it
            // fails first: exit 11.
            if o.output.is_some() {
                c = exit::OUTPUT_WRITE_FAILED;
            }
            if !msg.is_empty() {
                let _ = writeln!(err, "{msg}");
            }
            let _ = writeln!(err, "no result for {}", display(input));
            code = Some(c);
        }
    }
    code
}

/// The parser options xsltproc processes the inputs' and the stylesheet's
/// XIncludes with: libxslt's XSLT_PARSE_OPTIONS, so included documents get
/// their DTD and its default attributes even with `--novalid` or
/// `--nodtdattr`, and `--huge` doesn't apply; libxml2's depth limit is
/// global, so `--maxparserdepth` does. (`document()`'s XIncludes use the
/// command line's options, ctxt->parserOptions.)
fn xinclude_options(options: &ParseOptions) -> ParseOptions {
    ParseOptions {
        load_dtd: true,
        dtd_attributes: true,
        max_depth: options.max_depth,
        xinclude: options.xinclude,
        ..ParseOptions::default()
    }
}

/// The `href` of an `<?xml-stylesheet?>` before the document element whose
/// type is text/xml or text/xsl, as xsltLoadStylesheetPI finds it.
fn stylesheet_pi(doc: &crate::tree::Document, names: &NamePool) -> Option<String> {
    use crate::tree::NodeKind;
    for c in doc.children(doc.root()) {
        match doc.kind(c) {
            NodeKind::Element { .. } => return None,
            NodeKind::Pi { target, .. } if names.str(target) == "xml-stylesheet" => {
                if let Some(href) = parse_stylesheet_pi(doc.value(c).unwrap_or("")) {
                    return Some(href);
                }
            }
            _ => {}
        }
    }
    None
}

/// xsltParseStylesheetPI: the pseudo-attributes `type` and `href`, read as
/// libxslt reads them (the first href; a type other than text/xml or
/// text/xsl, compared without case, ends the search with no href).
fn parse_stylesheet_pi(value: &str) -> Option<String> {
    let b = value.as_bytes();
    let blank = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r');
    let mut i = 0;
    let mut href = None;
    let mut is_xml = false;
    // A quoted value after `name` and `=`: (value, end), or where to go on.
    let quoted = |mut i: usize| -> Result<(String, usize), usize> {
        while i < b.len() && blank(b[i]) {
            i += 1;
        }
        if b.get(i) != Some(&b'=') {
            return Err(i);
        }
        i += 1;
        let q = match b.get(i) {
            Some(&q) if q == b'"' || q == b'\'' => q,
            _ => return Err(i),
        };
        let start = i + 1;
        match b[start..].iter().position(|&c| c == q) {
            Some(n) => Ok((value[start..start + n].to_string(), start + n + 1)),
            None => Err(b.len()),
        }
    };
    while i < b.len() {
        while i < b.len() && blank(b[i]) {
            i += 1;
        }
        if b[i..].starts_with(b"type") {
            match quoted(i + 4) {
                Ok((v, end)) => {
                    i = end;
                    if !v.eq_ignore_ascii_case("text/xml") && !v.eq_ignore_ascii_case("text/xsl") {
                        break;
                    }
                    is_xml = true;
                }
                Err(at) => i = at,
            }
        } else if b[i..].starts_with(b"href") {
            match quoted(i + 4) {
                Ok((v, end)) => {
                    i = end;
                    href.get_or_insert(v);
                }
                Err(at) => i = at,
            }
        } else {
            while i < b.len() && !blank(b[i]) {
                i += 1;
            }
        }
    }
    href.filter(|_| is_xml)
}

/// The stylesheet an `<?xml-stylesheet?>` names: an element of the document
/// itself for `#id`, copied into a document of its own with its ancestors'
/// namespaces, or a file resolved against the document. None (reported)
/// when it can't be found or parsed.
fn load_pi_stylesheet(
    href: &str,
    doc: &crate::tree::Document,
    doc_loc: &Path,
    names: &mut NamePool,
    loader: &ResourceLoader,
    options: &ParseOptions,
    err: &mut Vec<u8>,
) -> Option<(crate::tree::Document, PathBuf)> {
    if let Some(fragment) = href.strip_prefix('#') {
        let id = fragment.strip_prefix('#').unwrap_or(fragment);
        let Some(el) = doc.element_by_id(id) else {
            let _ = writeln!(err, "xml-stylesheet : no ID {fragment} found");
            return None;
        };
        return Some((
            crate::xinclude::extract_element(doc, el),
            doc_loc.to_path_buf(),
        ));
    }
    let base = Some(doc_loc.to_path_buf());
    let (bytes, path) = match loader.read_document(href, &base) {
        Ok(r) => r,
        Err(_) => {
            let url = crate::uri::build(href, Some(&doc_loc.to_string_lossy()));
            let _ = writeln!(err, "xsltParseStylesheetFile : cannot parse {url}");
            return None;
        }
    };
    match parse_with(&bytes, names, options, loader, &Some(path.clone())) {
        Ok(p) => {
            for w in &p.warnings {
                let _ = writeln!(err, "warning: {w}");
            }
            Some((p.document, path))
        }
        Err(e) => {
            let _ = writeln!(
                err,
                "{}:{}: parser error : {}\nxsltParseStylesheetFile : cannot parse {}",
                display(&path),
                e.line,
                e.message,
                display(&path)
            );
            None
        }
    }
}

/// Opens a stylesheet or input named on the command line as xsltproc does,
/// through the entity loader: a name the catalogs map (such as DocBook's
/// `http://docbook.sourceforge.net/release/xsl/current/...` URLs) opens the
/// local copy, and that copy's path becomes the document's location, the
/// base for its includes. Other names are opened as given.
fn open_argument(
    loader: &ResourceLoader,
    arg: &std::path::Path,
) -> std::io::Result<(Vec<u8>, std::path::PathBuf)> {
    // "-" is standard input, as libxml2's xmlFileOpen reads it; the
    // document's URL is "-", so relative references resolve against the
    // current directory.
    if arg.as_os_str() == "-" {
        let bytes = crate::catalog::read_stream(std::io::stdin().lock())?;
        return Ok((bytes, arg.to_path_buf()));
    }
    let path = loader
        .locate(None, &arg.to_string_lossy(), &None)
        .unwrap_or_else(|| arg.to_path_buf());
    Ok((crate::catalog::read_file(&path)?, path))
}

fn global_params(
    o: &Options,
    names: &mut NamePool,
    err: &mut Vec<u8>,
) -> Result<Vec<(Name, Param)>, ()> {
    let mut out = Vec::new();
    for (name, value) in &o.params {
        let n: Name = (None, names.intern(name));
        let p = match value {
            ParamArg::String(s) => Param::String(s.clone()),
            ParamArg::Expr(e) => match crate::xpath::compile(e, names, &|_| None) {
                Ok(expr) => Param::Expr(expr),
                Err(x) => {
                    let _ = writeln!(
                        err,
                        "XPath error : {x}\nruntime error\nEvaluating user parameter {name} failed"
                    );
                    return Err(());
                }
            },
        };
        out.push((n, p));
    }
    Ok(out)
}
