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

const USAGE: &str = "Usage: oxslt [options] stylesheet file [file ...]
   Options:
      -o, --output file     save to a given file (default: standard output)
      --param name value    pass a (parameter, XPath expression) pair
      --stringparam name value  pass a (parameter, string) pair
      --nonet               refuse to fetch DTDs or entities over the network
      --novalid             skip the DTD loading phase
      --path paths          directories to search for DTDs and entities
      --xinclude            do XInclude processing on document input
      --nowrite             refuse to write to any file or resource
      --nomkdir             refuse to create directories
      --writesubtree path   allow file write only with the path subtree
      --version             print the version
";

#[derive(Default)]
struct Options {
    output: Option<PathBuf>,
    /// What exsl:document may write: xsltproc's default, or as
    /// `--nowrite`, `--nomkdir` and `--writesubtree` restrict it. Later
    /// options override earlier ones, as they replace libxslt's checks.
    writes: crate::xslt::exec::Writes,
    /// `--xinclude`: process XIncludes in the inputs.
    xinclude: bool,
    /// `--novalid`: no external DTD and no default attributes.
    novalid: bool,
    /// `--html`: the inputs are HTML, parsed as libxml2's HTML parser does.
    html: bool,
    /// `--path`: directories to search when a DTD or entity fails to load.
    paths: Vec<PathBuf>,
    /// `--catalogs`: the files in $SGML_CATALOG_FILES, used instead of
    /// XML_CATALOG_FILES (libxml2 then never reads that).
    catalogs: Option<Vec<String>>,
    params: Vec<(String, ParamArg)>,
    files: Vec<PathBuf>,
}

enum ParamArg {
    String(String),
    Expr(String),
}

fn parse_args(args: &[OsString], stderr: &mut dyn Write) -> Result<Options, i32> {
    let mut o = Options {
        writes: crate::xslt::exec::Writes::allowed(),
        ..Options::default()
    };
    let mut it = args.iter().map(|a| a.to_string_lossy().into_owned());
    let need = |it: &mut dyn Iterator<Item = String>, opt: &str, stderr: &mut dyn Write| {
        it.next().ok_or_else(|| {
            let _ = writeln!(stderr, "{opt} needs an argument\n{USAGE}");
            exit::NO_ARGUMENT
        })
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" | "-output" => {
                o.output = Some(PathBuf::from(need(&mut it, &a, stderr)?))
            }
            "--param" | "-param" | "--stringparam" | "-stringparam" => {
                let name = need(&mut it, &a, stderr)?;
                let value = need(&mut it, &a, stderr)?;
                if a.ends_with("stringparam") {
                    if value.contains('"') && value.contains('\'') {
                        let _ = writeln!(
                            stderr,
                            "stringparam contains both quote and double-quotes !"
                        );
                        return Err(exit::STRING_PARAM_HAS_BOTH_QUOTES);
                    }
                    o.params.push((name, ParamArg::String(value)));
                } else {
                    o.params.push((name, ParamArg::Expr(value)));
                }
            }
            "--path" | "-path" => {
                // Directories separated by spaces or colons, as xsltproc's parsePath splits them.
                let list = need(&mut it, &a, stderr)?;
                o.paths.extend(
                    list.split([' ', ':'])
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from),
                );
            }
            "--maxdepth" | "-maxdepth" => {
                need(&mut it, &a, stderr)?;
            }
            // libxml2's xmlLoadCatalogs on $SGML_CATALOG_FILES. Called before
            // any catalog is used, the first file it loads becomes the default
            // catalog and marks catalogs initialised, so XML_CATALOG_FILES and
            // /etc/xml/catalog are never read; the other files are added after
            // it. Files are separated by colons or blanks.
            "--catalogs" | "-catalogs" => match std::env::var("SGML_CATALOG_FILES") {
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
            "--novalid" | "-novalid" => o.novalid = true,
            "--xinclude" | "-xinclude" => o.xinclude = true,
            "--html" | "-html" => o.html = true,
            "--nowrite" | "-nowrite" => {
                o.writes.files = WriteCheck::Refuse;
                o.writes.mkdir = false;
            }
            "--nomkdir" | "-nomkdir" => o.writes.mkdir = false,
            "--writesubtree" | "-writesubtree" => {
                o.writes.files = WriteCheck::Subtree(need(&mut it, &a, stderr)?);
            }
            // oxslt never uses the network, which is what --nonet asks for.
            "--nonet" | "-nonet" => {}
            "--version" | "-version" | "-V" => {
                let _ = writeln!(stderr, "oxslt {}", env!("CARGO_PKG_VERSION"));
            }
            _ if a.starts_with('-') && a.len() > 1 => {
                let _ = writeln!(stderr, "Unknown option {a}\n{USAGE}");
                return Err(exit::UNKNOWN_OPTION);
            }
            _ => o.files.push(PathBuf::from(a)),
        }
    }
    // A stylesheet alone is allowed: xsltproc compiles it and transforms
    // nothing. Builds use that to check that a stylesheet is installed.
    if o.files.is_empty() {
        let _ = write!(stderr, "{USAGE}");
        return Err(exit::NO_ARGUMENT);
    }
    Ok(o)
}

fn display(p: &Path) -> String {
    p.display().to_string()
}

/// `exiting`: the process exits right after, so don't free what the
/// transform built (see `crate::run_before_exit`).
pub fn run(
    args: &[OsString],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    exiting: bool,
) -> i32 {
    let options = match parse_args(args, stderr) {
        Ok(o) => o,
        Err(code) => return code,
    };
    // Output and messages are collected on the transform thread and written here.
    let worker = std::thread::Builder::new()
        .name("transform".into())
        .stack_size(TRANSFORM_STACK)
        .spawn(move || {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let code = transform_all(&options, &mut out, &mut err, exiting);
            (code, out, err, options.output)
        });
    let (code, out, err, output) = match worker.map(|h| h.join()) {
        Ok(Ok(r)) => r,
        _ => {
            let _ = writeln!(stderr, "internal error: the transform thread failed");
            return exit::INTERNAL_ERROR;
        }
    };
    let _ = stderr.write_all(&err);
    // An empty or failed result writes nothing, not even an empty -o file:
    // xsltSaveResultToFilename returns before opening the file.
    if !out.is_empty() {
        let written = match &output {
            Some(path) => std::fs::write(path, &out),
            None => stdout.write_all(&out).and_then(|_| stdout.flush()),
        };
        if let Err(e) = written {
            let _ = writeln!(stderr, "failed to write the result: {e}");
            return exit::OUTPUT_WRITE_FAILED;
        }
    }
    code
}

fn transform_all(o: &Options, out: &mut Vec<u8>, err: &mut Vec<u8>, exiting: bool) -> i32 {
    let (style_path, inputs) = o.files.split_first().expect("checked in parse_args");
    let mut names = NamePool::new();
    // xsltproc's parser options: entities substituted, the DTD loaded and its
    // defaults applied, unless --novalid. Catalogs come from the environment,
    // or from $SGML_CATALOG_FILES with --catalogs.
    let options = ParseOptions {
        load_dtd: !o.novalid,
        dtd_attributes: !o.novalid,
        xinclude: o.xinclude,
        ..ParseOptions::default()
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
            let p = crate::html::parse(&bytes, &mut names, &display(&input_loc));
            Ok(crate::parse::Parsed {
                document: p.document,
                warnings: p.warnings,
            })
        } else {
            parse_with(
                &bytes,
                &mut names,
                &options,
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
        let params = match global_params(o, &mut names, err) {
            Ok(p) => p,
            Err(c) => return c,
        };
        let mut source = source;
        source.url = Some(input_loc.to_string_lossy().into_owned());
        // xsltproc processes XIncludes before transforming; any XInclude
        // error skips the input with exit 6, and no message of its own.
        if options.xinclude {
            let processed = crate::xinclude::process(source, &mut names, &loader, &options);
            for m in processed.warnings.iter().chain(&processed.errors) {
                let _ = writeln!(err, "{m}");
            }
            if !processed.errors.is_empty() {
                code = exit::DOCUMENT_ERROR;
                continue;
            }
            source = processed.document;
        }
        match Transform::run(
            &stylesheet,
            &mut names,
            &loader,
            &options,
            source,
            params,
            crate::xslt::exec::Limits {
                writes: crate::xslt::exec::Writes {
                    output_file: o.output.as_ref().map(|p| p.to_string_lossy().into_owned()),
                    ..o.writes.clone()
                },
                ..Default::default()
            },
            err,
        ) {
            Ok(result) => out.extend(crate::serialize::serialize(
                result,
                &mut names,
                &stylesheet.output,
            )),
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
                code = c;
            }
        }
    }
    if exiting {
        // Not unsafe: leaking is safe. The process is about to exit.
        std::mem::forget(stylesheet);
        std::mem::forget(names);
    }
    code
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
) -> Result<Vec<(Name, Param)>, i32> {
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
                        "XPath error : {x}\nEvaluating user parameter {name} failed"
                    );
                    return Err(exit::INTERNAL_ERROR);
                }
            },
        };
        out.push((n, p));
    }
    Ok(out)
}
