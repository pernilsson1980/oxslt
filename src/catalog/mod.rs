//! Loading external resources as libxml2 does for xsltproc: OASIS XML
//! catalogs (`XML_CATALOG_FILES`, default `/etc/xml/catalog`), local files,
//! and xsltproc's `--path` fallback. There's no network access at all, which
//! is what `--nonet` asks for: a URL that no catalog maps to a local file
//! fails with libxml2's "failed to load external entity" warning.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::dtd::{Base, Loader};
use crate::parse::{ParseOptions, parse};
use crate::tree::{Document, NamePool, NodeId, NodeKind};

const CATALOG_NS: &str = "urn:oasis:names:tc:entity:xmlns:xml:catalog";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
/// Catalogs that refer to each other more deeply than this are cut off.
const MAX_CATALOG_DEPTH: usize = 20;

#[derive(Debug, Clone)]
enum Entry {
    Public {
        id: String,
        uri: String,
        prefer_public: bool,
    },
    System {
        id: String,
        uri: String,
    },
    RewriteSystem {
        start: String,
        prefix: String,
    },
    SystemSuffix {
        suffix: String,
        uri: String,
    },
    Uri {
        name: String,
        uri: String,
    },
    RewriteUri {
        start: String,
        prefix: String,
    },
    UriSuffix {
        suffix: String,
        uri: String,
    },
    DelegatePublic {
        start: String,
        catalog: String,
    },
    DelegateSystem {
        start: String,
        catalog: String,
    },
    DelegateUri {
        start: String,
        catalog: String,
    },
    Next {
        catalog: String,
    },
}

/// A set of catalog files, read when first needed.
#[derive(Debug, Default)]
pub struct Catalog {
    roots: Vec<String>,
    loaded: RefCell<HashMap<String, Vec<Entry>>>,
}

/// The most read from a file that isn't a regular file (a pipe, a device).
const MAX_STREAM: u64 = 1 << 30;

/// Reads a file to parse. A regular file is read whole. Anything else (a
/// pipe, `/dev/stdin`, a device) is read in chunks, at most 1 GiB, and
/// reading stops at a NUL byte unless the data starts like UTF-16 or UCS-4:
/// XML can't contain one, so the parse fails there anyway. libxml2 reads in
/// chunks too, so `document('/dev/zero')` fails at once instead of filling
/// memory.
pub fn read_file(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    if file.metadata()?.is_file() {
        let mut out = Vec::new();
        (&file).read_to_end(&mut out)?;
        return Ok(out);
    }
    read_stream(file)
}

/// Reads a stream (a pipe, standard input, a device) as `read_file` reads a
/// file that isn't a regular file: in chunks, at most 1 GiB, stopping at a
/// NUL byte unless the data starts like UTF-16 or UCS-4.
pub fn read_stream(stream: impl std::io::Read) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    let mut reader = stream.take(MAX_STREAM);
    let mut chunk = [0u8; 4096];
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&chunk[..n]);
        // A byte-order mark, UTF-16 (one zero byte in the first two) or
        // UCS-4 ('<' padded to four bytes) may hold zero bytes.
        let wide = match out.get(..4) {
            Some([0xFE, 0xFF, ..] | [0xFF, 0xFE, ..]) => true,
            Some([0, 0, 0, 0x3C] | [0x3C, 0, 0, 0]) => true,
            Some([a, b, ..]) => (*a == 0) != (*b == 0),
            _ => false,
        };
        if !wide && chunk[..n].contains(&0) {
            return Ok(out);
        }
    }
}

/// A local file path for a URI or path, if it's local.
pub fn local_path(uri: &str) -> Option<PathBuf> {
    if let Some(rest) = uri.strip_prefix("file://") {
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        return Some(PathBuf::from(percent_decode(rest)));
    }
    if uri.contains("://") {
        return None;
    }
    Some(PathBuf::from(uri))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolves `reference` against `base` (a directory or a file's URI or path).
fn resolve_against(reference: &str, base: &str) -> String {
    if reference.contains("://") || reference.starts_with('/') {
        return reference.to_string();
    }
    let dir = match base.rfind('/') {
        Some(i) => &base[..=i],
        None => "",
    };
    format!("{dir}{reference}")
}

/// XML Catalogs 1.1 section 6.2: public identifiers are compared with
/// whitespace normalised.
fn normalize_public(id: &str) -> String {
    id.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

impl Catalog {
    /// The catalogs xsltproc would use: `XML_CATALOG_FILES` (paths or URIs
    /// separated by spaces), or `/etc/xml/catalog`.
    pub fn from_env() -> Catalog {
        let roots = match std::env::var("XML_CATALOG_FILES") {
            Ok(v) => v.split_ascii_whitespace().map(str::to_string).collect(),
            Err(_) => vec!["file:///etc/xml/catalog".to_string()],
        };
        Catalog {
            roots,
            loaded: RefCell::default(),
        }
    }

    pub fn empty() -> Catalog {
        Catalog::default()
    }

    /// Specific catalog files (paths or URIs).
    pub fn new(roots: Vec<String>) -> Catalog {
        Catalog {
            roots,
            loaded: RefCell::default(),
        }
    }

    /// The entries of one catalog file; a file that can't be read or parsed
    /// has none, as in libxml2.
    fn entries(&self, uri: &str) -> Vec<Entry> {
        if let Some(e) = self.loaded.borrow().get(uri) {
            return e.clone();
        }
        let entries = local_path(uri)
            .and_then(|p| read_file(&p).ok())
            .and_then(|bytes| {
                let mut names = NamePool::new();
                let opts = ParseOptions {
                    load_dtd: false,
                    dtd_attributes: false,
                    ..ParseOptions::default()
                };
                parse(&bytes, &mut names, &opts)
                    .ok()
                    .map(|doc| read_entries(&doc, &names, uri))
            })
            .unwrap_or_default();
        self.loaded
            .borrow_mut()
            .insert(uri.to_string(), entries.clone());
        entries
    }

    /// Resolves an external identifier (catalog spec section 7.1.2).
    pub fn resolve_external(&self, public: Option<&str>, system: Option<&str>) -> Option<String> {
        let public = public.map(normalize_public);
        self.roots
            .iter()
            .find_map(|r| self.external_in(r, public.as_deref(), system, 0))
    }

    fn external_in(
        &self,
        catalog: &str,
        public: Option<&str>,
        system: Option<&str>,
        depth: usize,
    ) -> Option<String> {
        if depth > MAX_CATALOG_DEPTH {
            return None;
        }
        let entries = self.entries(catalog);
        if let Some(sys) = system {
            if let Some(u) = entries.iter().find_map(|e| match e {
                Entry::System { id, uri } if id == sys => Some(uri.clone()),
                _ => None,
            }) {
                return Some(u);
            }
            if let Some(u) = longest(&entries, sys, |e| match e {
                Entry::RewriteSystem { start, prefix } => Some((start.as_str(), prefix.as_str())),
                _ => None,
            }) {
                return Some(format!("{}{}", u.1, &sys[u.0.len()..]));
            }
            if let Some(u) = longest_suffix(&entries, sys, |e| match e {
                Entry::SystemSuffix { suffix, uri } => Some((suffix.as_str(), uri.as_str())),
                _ => None,
            }) {
                return Some(u);
            }
            let delegates = delegates(&entries, sys, |e| match e {
                Entry::DelegateSystem { start, catalog } => {
                    Some((start.as_str(), catalog.as_str()))
                }
                _ => None,
            });
            if !delegates.is_empty() {
                return delegates
                    .iter()
                    .find_map(|c| self.external_in(c, None, system, depth + 1));
            }
        }
        if let Some(pubid) = public {
            if let Some(u) = entries.iter().find_map(|e| match e {
                Entry::Public {
                    id,
                    uri,
                    prefer_public,
                } if id == pubid && (*prefer_public || system.is_none()) => Some(uri.clone()),
                _ => None,
            }) {
                return Some(u);
            }
            let delegates = delegates(&entries, pubid, |e| match e {
                Entry::DelegatePublic { start, catalog } => {
                    Some((start.as_str(), catalog.as_str()))
                }
                _ => None,
            });
            if !delegates.is_empty() {
                return delegates
                    .iter()
                    .find_map(|c| self.external_in(c, public, None, depth + 1));
            }
        }
        entries.iter().find_map(|e| match e {
            Entry::Next { catalog } => self.external_in(catalog, public, system, depth + 1),
            _ => None,
        })
    }

    /// Resolves a URI reference (catalog spec section 7.2.2).
    pub fn resolve_uri(&self, uri: &str) -> Option<String> {
        self.roots.iter().find_map(|r| self.uri_in(r, uri, 0))
    }

    fn uri_in(&self, catalog: &str, target: &str, depth: usize) -> Option<String> {
        if depth > MAX_CATALOG_DEPTH {
            return None;
        }
        let entries = self.entries(catalog);
        if let Some(u) = entries.iter().find_map(|e| match e {
            Entry::Uri { name, uri } if name == target => Some(uri.clone()),
            _ => None,
        }) {
            return Some(u);
        }
        if let Some(u) = longest(&entries, target, |e| match e {
            Entry::RewriteUri { start, prefix } => Some((start.as_str(), prefix.as_str())),
            _ => None,
        }) {
            return Some(format!("{}{}", u.1, &target[u.0.len()..]));
        }
        if let Some(u) = longest_suffix(&entries, target, |e| match e {
            Entry::UriSuffix { suffix, uri } => Some((suffix.as_str(), uri.as_str())),
            _ => None,
        }) {
            return Some(u);
        }
        let delegates = delegates(&entries, target, |e| match e {
            Entry::DelegateUri { start, catalog } => Some((start.as_str(), catalog.as_str())),
            _ => None,
        });
        if !delegates.is_empty() {
            return delegates
                .iter()
                .find_map(|c| self.uri_in(c, target, depth + 1));
        }
        entries.iter().find_map(|e| match e {
            Entry::Next { catalog } => self.uri_in(catalog, target, depth + 1),
            _ => None,
        })
    }
}

/// The matching (start, replacement) entry with the longest start.
fn longest<'e>(
    entries: &'e [Entry],
    s: &str,
    pick: impl Fn(&'e Entry) -> Option<(&'e str, &'e str)>,
) -> Option<(&'e str, &'e str)> {
    entries
        .iter()
        .filter_map(pick)
        .filter(|(start, _)| s.starts_with(start))
        .max_by_key(|(start, _)| start.len())
}

fn longest_suffix<'e>(
    entries: &'e [Entry],
    s: &str,
    pick: impl Fn(&'e Entry) -> Option<(&'e str, &'e str)>,
) -> Option<String> {
    entries
        .iter()
        .filter_map(pick)
        .filter(|(suffix, _)| s.ends_with(suffix))
        .max_by_key(|(suffix, _)| suffix.len())
        .map(|(_, uri)| uri.to_string())
}

/// The catalogs of matching delegate entries, longest start first.
fn delegates<'e>(
    entries: &'e [Entry],
    s: &str,
    pick: impl Fn(&'e Entry) -> Option<(&'e str, &'e str)>,
) -> Vec<String> {
    let mut m: Vec<(&str, &str)> = entries
        .iter()
        .filter_map(pick)
        .filter(|(start, _)| s.starts_with(start))
        .collect();
    m.sort_by_key(|(start, _)| std::cmp::Reverse(start.len()));
    m.into_iter().map(|(_, c)| c.to_string()).collect()
}

/// Reads a catalog document's entries, resolving relative URIs against the
/// catalog's location and any `xml:base`.
fn read_entries(doc: &Document, names: &NamePool, catalog_uri: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let Some(root) = doc
        .children(doc.root())
        .find(|&c| matches!(doc.kind(c), NodeKind::Element { .. }))
    else {
        return out;
    };
    walk(doc, names, root, catalog_uri, true, &mut out);
    out
}

fn attr<'d>(
    doc: &'d Document,
    names: &NamePool,
    el: NodeId,
    local: &str,
    ns: Option<&str>,
) -> Option<&'d str> {
    doc.attributes(el).find_map(|a| {
        let q = doc.name(a)?;
        let q_ns = q.ns.map(|n| names.str(n));
        (names.str(q.local) == local && q_ns == ns).then(|| doc.value(a).unwrap_or(""))
    })
}

fn walk(
    doc: &Document,
    names: &NamePool,
    el: NodeId,
    base: &str,
    prefer_public: bool,
    out: &mut Vec<Entry>,
) {
    let base = match attr(doc, names, el, "base", Some(XML_NS)) {
        Some(b) => resolve_against(b, base),
        None => base.to_string(),
    };
    let prefer_public = match attr(doc, names, el, "prefer", None) {
        Some("system") => false,
        Some("public") => true,
        _ => prefer_public,
    };
    for c in doc.children(el) {
        let Some(q) = doc.name(c) else { continue };
        if q.ns.map(|n| names.str(n)) != Some(CATALOG_NS) {
            continue;
        }
        let a = |n: &str| attr(doc, names, c, n, None).map(str::to_string);
        let abs = |n: &str| a(n).map(|v| resolve_against(&v, &base));
        let entry = match names.str(q.local) {
            "public" => a("publicId")
                .zip(abs("uri"))
                .map(|(id, uri)| Entry::Public {
                    id: normalize_public(&id),
                    uri,
                    prefer_public,
                }),
            "system" => a("systemId")
                .zip(abs("uri"))
                .map(|(id, uri)| Entry::System { id, uri }),
            "rewriteSystem" => a("systemIdStartString")
                .zip(abs("rewritePrefix"))
                .map(|(start, prefix)| Entry::RewriteSystem { start, prefix }),
            "systemSuffix" => a("systemIdSuffix")
                .zip(abs("uri"))
                .map(|(suffix, uri)| Entry::SystemSuffix { suffix, uri }),
            "uri" => a("name")
                .zip(abs("uri"))
                .map(|(name, uri)| Entry::Uri { name, uri }),
            "rewriteURI" => a("uriStartString")
                .zip(abs("rewritePrefix"))
                .map(|(start, prefix)| Entry::RewriteUri { start, prefix }),
            "uriSuffix" => a("uriSuffix")
                .zip(abs("uri"))
                .map(|(suffix, uri)| Entry::UriSuffix { suffix, uri }),
            "delegatePublic" => {
                a("publicIdStartString")
                    .zip(abs("catalog"))
                    .map(|(start, catalog)| Entry::DelegatePublic {
                        start: normalize_public(&start),
                        catalog,
                    })
            }
            "delegateSystem" => a("systemIdStartString")
                .zip(abs("catalog"))
                .map(|(start, catalog)| Entry::DelegateSystem { start, catalog }),
            "delegateURI" => a("uriStartString")
                .zip(abs("catalog"))
                .map(|(start, catalog)| Entry::DelegateUri { start, catalog }),
            "nextCatalog" => abs("catalog").map(|catalog| Entry::Next { catalog }),
            "group" => {
                walk(doc, names, c, &base, prefer_public, out);
                None
            }
            _ => None,
        };
        out.extend(entry);
    }
}

/// Loads external resources as xsltproc's entity loader does.
#[derive(Debug)]
pub struct ResourceLoader {
    pub catalog: Catalog,
    /// `--path`: directories to try with a failed URL's last segment.
    pub paths: Vec<PathBuf>,
    /// Refuse every resource: for fuzzing, where a stylesheet mustn't
    /// read files.
    pub refuse_all: bool,
}

impl ResourceLoader {
    /// Catalogs from the environment, as xsltproc uses them.
    pub fn from_env(paths: Vec<PathBuf>) -> Self {
        ResourceLoader {
            catalog: Catalog::from_env(),
            paths,
            refuse_all: false,
        }
    }

    /// Loads nothing: every external entity, DTD, include and document()
    /// fails to load.
    pub fn none() -> Self {
        ResourceLoader {
            refuse_all: true,
            ..ResourceLoader::local()
        }
    }

    /// No catalogs: only local files, relative to the referring file.
    pub fn local() -> Self {
        ResourceLoader {
            catalog: Catalog::empty(),
            paths: Vec::new(),
            refuse_all: false,
        }
    }

    /// Where `system` (with optional `public`) refers to, as a local path:
    /// made absolute against `base`, mapped by the catalogs, and tried as a
    /// URI if the result isn't local (libxml2's xmlResolveResourceFromCatalog).
    pub fn locate(&self, public: Option<&str>, system: &str, base: &Base) -> Option<PathBuf> {
        self.locate_how(public, system, base).map(|(p, _)| p)
    }

    /// [`locate`](Self::locate), also saying whether a `--path` directory
    /// found it. libxml2 then names the document by the path found
    /// (xsltproc's loader returns an input for `dir/segment`), so its own
    /// relative references resolve against that directory.
    pub fn locate_how(
        &self,
        public: Option<&str>,
        system: &str,
        base: &Base,
    ) -> Option<(PathBuf, bool)> {
        if self.refuse_all {
            return None;
        }
        let absolute = match base {
            Some(b) if !system.contains("://") && !system.starts_with('/') => b
                .parent()
                .unwrap_or(Path::new("."))
                .join(system)
                .to_string_lossy()
                .into_owned(),
            _ => system.to_string(),
        };
        let mut resource = self
            .catalog
            .resolve_external(public, Some(&absolute))
            .unwrap_or_else(|| absolute.clone());
        if !local_path(&resource).is_some_and(|p| p.exists())
            && let Some(mapped) = self.catalog.resolve_uri(&resource)
        {
            resource = mapped;
        }
        if let Some(p) = local_path(&resource).filter(|p| p.exists()) {
            return Some((p, false));
        }
        // xsltproc's --path: each directory with the URL's last segment.
        let last = absolute.rsplit('/').next().unwrap_or(&absolute);
        self.paths
            .iter()
            .map(|d| d.join(last))
            .find(|p| p.exists())
            .map(|p| (p, true))
    }

    /// Reads a document (a stylesheet include or a `document()` target).
    pub fn read_document(&self, href: &str, base: &Base) -> Result<(Vec<u8>, PathBuf), String> {
        self.read_document_how(href, base)
            .map(|(bytes, path, _)| (bytes, path))
    }

    /// [`read_document`](Self::read_document), also saying whether a
    /// `--path` directory found it ([`locate_how`](Self::locate_how)).
    pub fn read_document_how(
        &self,
        href: &str,
        base: &Base,
    ) -> Result<(Vec<u8>, PathBuf, bool), String> {
        let (path, by_path) = self
            .locate_how(None, href, base)
            .ok_or_else(|| format!("failed to load external entity \"{href}\""))?;
        let bytes = read_file(&path)
            .map_err(|e| format!("failed to load external entity \"{href}\": {e}"))?;
        Ok((bytes, path, by_path))
    }
}

impl Loader for ResourceLoader {
    fn load(
        &self,
        public: Option<&str>,
        system: &str,
        base: &Base,
    ) -> Result<(String, Base), String> {
        let (bytes, path) = match self.locate(public, system, base) {
            Some(p) => (
                read_file(&p)
                    .map_err(|_| format!("failed to load external entity \"{system}\""))?,
                p,
            ),
            None => return Err(format!("failed to load external entity \"{system}\"")),
        };
        let text = crate::parse::decode_entity(&bytes)
            .map_err(|e| format!("{}: {}", path.display(), e.message))?;
        Ok((text, Some(path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> String {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        format!("file://{}", p.display())
    }

    #[test]
    fn catalog_entries_resolve() {
        let dir = std::env::temp_dir().join(format!("oxslt-catalog-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let next = write(
            &dir,
            "next.xml",
            &format!(
                r#"<catalog xmlns="{CATALOG_NS}"><system systemId="urn:next" uri="next.dtd"/></catalog>"#
            ),
        );
        let main = write(
            &dir,
            "catalog.xml",
            &format!(
                r#"<catalog xmlns="{CATALOG_NS}" prefer="public">
                 <public publicId="-//X//DTD  Thing//EN" uri="thing.dtd"/>
                 <rewriteSystem systemIdStartString="http://example.com/dtd/" rewritePrefix="local/"/>
                 <rewriteSystem systemIdStartString="http://example.com/dtd/deep/" rewritePrefix="deeper/"/>
                 <group xml:base="sub/"><uri name="http://example.com/style.xsl" uri="style.xsl"/></group>
                 <rewriteURI uriStartString="http://cdn/" rewritePrefix="file:///opt/cdn/"/>
                 <nextCatalog catalog="next.xml"/>
               </catalog>"#
            ),
        );
        let cat = Catalog {
            roots: vec![main],
            loaded: RefCell::default(),
        };
        let d = format!("file://{}/", dir.display());
        assert_eq!(
            cat.resolve_external(Some("-//X//DTD Thing//EN"), None),
            Some(format!("{d}thing.dtd"))
        );
        assert_eq!(
            cat.resolve_external(None, Some("http://example.com/dtd/a.dtd")),
            Some(format!("{d}local/a.dtd"))
        );
        assert_eq!(
            cat.resolve_external(None, Some("http://example.com/dtd/deep/b.dtd")),
            Some(format!("{d}deeper/b.dtd"))
        );
        assert_eq!(
            cat.resolve_uri("http://example.com/style.xsl"),
            Some(format!("{d}sub/style.xsl"))
        );
        assert_eq!(
            cat.resolve_uri("http://cdn/x/y.xsl"),
            Some("file:///opt/cdn/x/y.xsl".to_string())
        );
        assert_eq!(
            cat.resolve_external(None, Some("urn:next")),
            Some(format!("{d}next.dtd"))
        );
        assert_eq!(cat.resolve_external(None, Some("http://unknown/")), None);
        let _ = next;
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
