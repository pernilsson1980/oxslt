//! exsl:document (libxslt's xsltDocumentElem) and the checks on what it may
//! write (xsltCheckWrite): `Writes`, set by the command line's --nowrite,
//! --nomkdir and --writesubtree.

use super::*;

/// What `exsl:document` may write, as xsltproc's security preferences
/// decide (libxslt's xsltCheckWrite).
#[derive(Debug, Clone, Default)]
pub struct Writes {
    pub files: WriteCheck,
    /// Missing directories may be created.
    pub mkdir: bool,
    /// The `-o` file: an `exsl:document`'s `href` is relative to it, or to
    /// the current directory without one.
    pub output_file: Option<String>,
}

/// Which files may be written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum WriteCheck {
    /// Nothing (xsltproc's `--nowrite`); the default for library callers.
    #[default]
    Refuse,
    /// Anything the user may write: xsltproc's default.
    Allow,
    /// Paths starting with this prefix (`--writesubtree`).
    Subtree(String),
}

impl Writes {
    /// xsltproc's defaults: any file, creating directories.
    pub fn allowed() -> Writes {
        Writes {
            files: WriteCheck::Allow,
            mkdir: true,
            output_file: None,
        }
    }

    fn file_allowed(&self, path: &str) -> bool {
        match &self.files {
            WriteCheck::Refuse => false,
            WriteCheck::Allow => true,
            WriteCheck::Subtree(prefix) => path.starts_with(prefix.as_str()),
        }
    }
}

impl<'a> Transform<'a> {
    /// exsl:document, as libxslt's xsltDocumentElem: the file name is the
    /// href resolved against the file being written (or the current
    /// directory), checked against the write rules (making missing
    /// directories), and the body written there with the element's own
    /// output attributes; the main xsl:output doesn't apply. Failures are
    /// reported and the transform goes on.
    pub(super) fn document_element(
        &mut self,
        href: Option<&Avt>,
        settings: &[(&'static str, Avt)],
        body: &Body,
        focus: &Focus,
        loc: &Location,
    ) -> R<()> {
        let Some(href) = href else {
            self.report(loc, "xsltDocumentElem: href/URI-Reference not found");
            return Ok(());
        };
        let url = self.avt(href, focus, loc)?;
        let filename = crate::uri::build(&url, self.writes.output_file.as_deref());
        match self.check_write(&filename) {
            1 => {}
            0 => {
                self.report(
                    loc,
                    &format!("xsltDocumentElem: write rights for {filename} denied"),
                );
                return Ok(());
            }
            _ => return Ok(()),
        }

        let mut output = Output::default();
        let mut append = false;
        let mut unsupported = None;
        for (name, avt) in settings {
            let value = self.avt(avt, focus, loc)?;
            let yes_no = |t: &mut Self, what: &str| match value.as_str() {
                "yes" => Some(true),
                "no" => Some(false),
                other => {
                    t.report(loc, &format!("invalid value for {what}: {other}"));
                    None
                }
            };
            match *name {
                "version" => output.version = Some(value.clone()),
                "encoding" => output.encoding = Some(value.clone()),
                "method" => match value.as_str() {
                    "xml" => output.method = Method::Xml,
                    "html" => output.method = Method::Html,
                    "text" => output.method = Method::Text,
                    "xhtml" => {
                        self.report(loc, "xsltDocumentElem: unsupported method xhtml");
                        output.method = Method::Html;
                    }
                    other if other.contains(':') => unsupported = Some(other.to_string()),
                    other => self.report(loc, &format!("invalid value for method: {other}")),
                },
                "doctype-system" => output.doctype_system = Some(value.clone()),
                "doctype-public" => output.doctype_public = Some(value.clone()),
                "standalone" => {
                    if let Some(b) = yes_no(self, "standalone") {
                        output.standalone = Some(b);
                    }
                }
                "indent" => {
                    if let Some(b) = yes_no(self, "indent") {
                        output.indent = Some(b);
                    }
                }
                "omit-xml-declaration" => {
                    if let Some(b) = yes_no(self, "omit-xml-declaration") {
                        output.omit_xml_declaration = Some(b);
                    }
                }
                // Read into a stylesheet nothing consults: CDATA comes from
                // the main xsl:output, as in libxslt.
                "cdata-section-elements" => {}
                "append" => append = value == "true" || value == "yes",
                _ => {}
            }
        }
        if let Some(m) = unsupported {
            self.report(loc, &format!("xsltDocumentElem: unsupported method ({m})"));
            return Ok(());
        }
        if append {
            output.omit_xml_declaration = Some(true);
        }

        // The body, into a document of its own; nested exsl:documents are
        // relative to this file.
        let cdata = match output.method {
            Method::Unspecified | Method::Xml => self.cdata.clone(),
            Method::Html | Method::Text => Rc::new(Vec::new()),
        };
        let saved_file = self.writes.output_file.replace(filename.clone());
        self.out.push(ResultTree::new(cdata));
        let result = self.body(body, focus);
        let tree = self.out.pop().expect("exsl:document output");
        self.writes.output_file = saved_file;
        result?;
        let doc = tree
            .finish(self.names)
            .map_err(|e| ExecError::Runtime(format!("{e:?}")))?;
        // An empty result writes no file at all (xsltSaveResultToFilename).
        if doc.first_child(doc.root()).is_none() {
            return Ok(());
        }
        let bytes = crate::serialize::serialize(doc, self.names, &output);
        let path = crate::catalog::local_path(&filename);
        let written = path.ok_or(()).and_then(|p| {
            let mut options = std::fs::OpenOptions::new();
            if append {
                options.append(true).create(true);
            } else {
                options.write(true).create(true).truncate(true);
            }
            options
                .open(p)
                .and_then(|mut f| f.write_all(&bytes))
                .map_err(|_| ())
        });
        if written.is_err() {
            self.report(
                loc,
                &format!("xsltDocumentElem: unable to save to {filename}"),
            );
        }
        Ok(())
    }

    /// xsltCheckWrite: 1 if `filename` may be written, 0 if refused (with a
    /// message), negative if a directory couldn't be made. Only local files
    /// are checked; other URLs fail when written.
    fn check_write(&mut self, filename: &str) -> i32 {
        match crate::catalog::local_path(filename) {
            Some(p) => self.check_write_path(&p.to_string_lossy()),
            None if self.writes.files == WriteCheck::Refuse => {
                self.report_plain(&format!("File write for {filename} refused"));
                0
            }
            None => 1,
        }
    }

    /// xsltCheckWritePath: the file check, then the directory, made (after
    /// checking it the same way) if it's missing.
    fn check_write_path(&mut self, path: &str) -> i32 {
        if !self.writes.file_allowed(path) {
            self.report_plain(&format!("File write for {path} refused"));
            return 0;
        }
        // xmlParserGetDirectory: up to the last '/', else the current one.
        let directory = match path.rfind('/') {
            Some(0) => "/".to_string(),
            Some(i) => path[..i].to_string(),
            None => return 1,
        };
        if std::path::Path::new(&directory).exists() {
            return 1;
        }
        if !self.writes.mkdir {
            self.report_plain(&format!("Directory creation for {path} refused"));
            return 0;
        }
        let mut ret = self.check_write_path(&directory);
        if ret == 1 {
            ret = if std::fs::create_dir(&directory).is_ok() {
                0
            } else {
                -1
            };
        }
        if ret < 0 {
            return ret;
        }
        1
    }
}
