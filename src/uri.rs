//! URI references as libxml2 2.9 resolves them (`xmlBuildURI`), for
//! `document()` and base URIs. libxslt identifies a loaded document by its
//! URL string, so the strings must come out the same as libxml2's, relative
//! paths included: `in.xml` against the base `style.xsl` stays `in.xml`.
//!
//! Not ported: `xmlSaveUri`'s escaping of characters outside the URI
//! syntax, and query strings, which local files don't have.

/// A URI split as RFC 3986 (and libxml2's parser) does.
#[derive(Debug, Default, Clone)]
struct Parts<'a> {
    scheme: Option<&'a str>,
    authority: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

fn split(uri: &str) -> Parts<'_> {
    let mut rest = uri;
    let mut p = Parts::default();
    if let Some(i) = rest.find('#') {
        p.fragment = Some(&rest[i + 1..]);
        rest = &rest[..i];
    }
    if let Some(i) = rest.find('?') {
        p.query = Some(&rest[i + 1..]);
        rest = &rest[..i];
    }
    // A scheme: a letter, then letters, digits, '+', '-' or '.', then ':'.
    if let Some(i) = rest.find(':') {
        let s = &rest[..i];
        let mut chars = s.chars();
        if chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            p.scheme = Some(s);
            rest = &rest[i + 1..];
        }
    }
    if let Some(r) = rest.strip_prefix("//") {
        let end = r.find('/').unwrap_or(r.len());
        p.authority = Some(&r[..end]);
        rest = &r[end..];
    }
    p.path = rest;
    p
}

/// Whether `uri` has a scheme, so it isn't resolved against a base.
pub fn has_scheme(uri: &str) -> bool {
    split(uri).scheme.is_some()
}

/// xmlBuildURI: `uri` resolved against `base`.
pub fn build(uri: &str, base: Option<&str>) -> String {
    let Some(base) = base else {
        return uri.to_string();
    };
    if uri.is_empty() {
        // The base, without its fragment.
        return base.split('#').next().unwrap_or("").to_string();
    }
    let r = split(uri);
    if r.scheme.is_some() {
        return uri.to_string();
    }
    let b = split(base);
    let mut out = String::new();
    if let Some(s) = b.scheme {
        out.push_str(s);
        out.push(':');
    }
    let tail = |out: &mut String| {
        if let Some(q) = r.query {
            out.push('?');
            out.push_str(q);
        }
        if let Some(f) = r.fragment {
            out.push('#');
            out.push_str(f);
        }
    };
    if r.authority.is_some() {
        out.push_str(uri);
        return out;
    }
    if let Some(a) = b.authority {
        out.push_str("//");
        out.push_str(a);
    }
    if r.path.is_empty() {
        // Only a query or fragment: the base's path.
        out.push_str(b.path);
        if r.query.is_none()
            && let Some(q) = b.query
        {
            out.push('?');
            out.push_str(q);
        }
        tail(&mut out);
        return out;
    }
    if r.path.starts_with('/') {
        out.push_str(r.path);
        tail(&mut out);
        return out;
    }
    // The base's path up to its last '/', then the reference's path.
    let mut path = match b.path.rfind('/') {
        Some(i) => b.path[..=i].to_string(),
        None => String::new(),
    };
    if path.is_empty() && b.authority.is_some() {
        path.push('/');
    }
    path.push_str(r.path);
    out.push_str(&normalize_path(&path));
    tail(&mut out);
    out
}

/// xmlNormalizeURIPath: drops "." segments and "segment/.." pairs, and any
/// ".." segments left at the start of an absolute path.
pub fn normalize_path(path: &str) -> String {
    let lead = path.len() - path.trim_start_matches('/').len();
    let body = &path[lead..];
    if body.is_empty() {
        return path.to_string();
    }
    let trailing_slash = body.ends_with('/');
    // (c) and (d): "." segments, and empty segments from "//".
    let mut segs: Vec<&str> = Vec::new();
    let parts: Vec<&str> = body.split('/').collect();
    let n = parts.len();
    for (i, s) in parts.iter().enumerate() {
        let last = i + 1 == n;
        if *s == "." || (s.is_empty() && !last) {
            continue;
        }
        segs.push(s);
    }
    let ends_dir = trailing_slash || body.ends_with("/.") || body == ".";
    if segs.last() == Some(&"") {
        segs.pop();
    }
    // (e) and (f): "segment/.." pairs, leftmost first, backing up after each.
    let mut i = 0;
    let mut dir = ends_dir;
    while i + 1 < segs.len() {
        if segs[i] != ".." && segs[i + 1] == ".." {
            let at_end = i + 2 == segs.len();
            segs.drain(i..i + 2);
            if at_end {
                dir = true;
            }
            i = i.saturating_sub(1);
        } else {
            i += 1;
        }
    }
    let mut out = "/".repeat(lead);
    out.push_str(&segs.join("/"));
    if dir && !segs.is_empty() {
        out.push('/');
    }
    // (g): ".." segments left at the start of an absolute path go.
    if lead > 0 {
        while let Some(rest) = out.strip_prefix("/..") {
            if rest.is_empty() || rest.starts_with('/') {
                out = if rest.is_empty() {
                    "/".to_string()
                } else {
                    rest.to_string()
                };
            } else {
                break;
            }
        }
    }
    out
}

/// The local file a URI names, if it names one: a `file:` URI's path, or a
/// URI without a scheme.
pub fn to_path(uri: &str) -> Option<String> {
    let uri = uri.split('#').next().unwrap_or("");
    if let Some(rest) = uri.strip_prefix("file://") {
        // file:///path or file://localhost/path
        let path = rest.strip_prefix("localhost").unwrap_or(rest);
        return Some(path.to_string());
    }
    if let Some(rest) = uri.strip_prefix("file:") {
        return Some(rest.to_string());
    }
    (!has_scheme(uri)).then(|| uri.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_like_libxml2() {
        assert_eq!(build("in.xml", Some("style.xsl")), "in.xml");
        assert_eq!(build("in.xml", Some("dir/style.xsl")), "dir/in.xml");
        assert_eq!(build("../a.xml", Some("dir/sub/style.xsl")), "dir/a.xml");
        assert_eq!(build("../a.xml", Some("style.xsl")), "../a.xml");
        assert_eq!(build("./a.xml", Some("/x/y.xsl")), "/x/a.xml");
        assert_eq!(build("/abs.xml", Some("/x/y.xsl")), "/abs.xml");
        assert_eq!(build("", Some("/x/y.xsl#f")), "/x/y.xsl");
        assert_eq!(build("#f", Some("/x/y.xsl")), "/x/y.xsl#f");
        assert_eq!(build("b.xml", Some("http://h/p/a.xml")), "http://h/p/b.xml");
        assert_eq!(build("http://h/x", Some("/a")), "http://h/x");
        assert_eq!(build("a.xml", None), "a.xml");
        assert_eq!(build("l1/l2/../../a.xml", Some("s.xsl")), "a.xml");
        assert_eq!(build("../../a.xml", Some("/x/s.xsl")), "/a.xml");
    }

    #[test]
    fn paths() {
        assert_eq!(to_path("file:///tmp/a.xml").as_deref(), Some("/tmp/a.xml"));
        assert_eq!(to_path("a.xml#x").as_deref(), Some("a.xml"));
        assert_eq!(to_path("http://h/a.xml"), None);
    }
}

/// xmlURIEscapeStr: percent-escapes every byte that isn't unreserved
/// (letters, digits, `-_.!~*'()`), `@` or in `keep`.
pub fn escape_str(s: &str, keep: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let unreserved = b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b);
        if b == b'@' || unreserved || keep.as_bytes().contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A path as libxml2's URI parser stores it: %XX escapes decoded.
fn unescape(path: &str) -> String {
    let b = path.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (
                b.get(i + 1).and_then(|&c| hex(c)),
                b.get(i + 2).and_then(|&c| hex(c)),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The host of an authority (`user@host:port`), as libxml2's `server`.
fn server(authority: Option<&str>) -> Option<&str> {
    authority.map(|a| {
        let a = a.rsplit_once('@').map_or(a, |(_, h)| h);
        a.split_once(':').map_or(a, |(h, _)| h)
    })
}

/// xmlBuildRelativeURI: `uri` expressed relative to `base`. None where
/// libxml2 returns NULL (an empty `uri`).
pub fn build_relative(uri: &str, base: Option<&str>) -> Option<String> {
    if uri.is_empty() {
        return None;
    }
    // A reference already in "relative" form ('.') is taken as a path.
    let (r_scheme, r_server, r_path) = if uri.starts_with('.') {
        (None, None, Some(uri.to_string()))
    } else {
        let p = split(uri);
        let path = (!p.path.is_empty() || p.authority.is_none()).then(|| unescape(p.path));
        (
            p.scheme,
            server(p.authority),
            path.filter(|s| !s.is_empty()),
        )
    };
    let Some(base) = base.filter(|b| !b.is_empty()) else {
        return Some(uri.to_string());
    };
    let (b_scheme, b_server, b_path) = if base.starts_with('.') {
        (None, None, Some(base.to_string()))
    } else {
        let p = split(base);
        (
            p.scheme,
            server(p.authority),
            Some(unescape(p.path)).filter(|s| !s.is_empty()),
        )
    };
    // A different scheme or server: the URI as it is.
    if r_scheme.is_some() && (b_scheme.is_none() || b_scheme != r_scheme || b_server != r_server) {
        return Some(uri.to_string());
    }
    if b_path == r_path {
        return Some(String::new());
    }
    let Some(b_path) = b_path else {
        return Some(r_path.unwrap_or_default());
    };
    let r_path = r_path.unwrap_or_else(|| "/".to_string());

    let mut bptr = b_path.as_bytes();
    let mut rptr = r_path.as_bytes();
    if rptr.starts_with(b"./") {
        rptr = &rptr[2..];
    }
    if bptr.starts_with(b"./") {
        bptr = &bptr[2..];
    } else if bptr.first() == Some(&b'/') && rptr.first() != Some(&b'/') {
        bptr = &bptr[1..];
    }
    let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);
    let mut pos = 0;
    while at(bptr, pos) == at(rptr, pos) && at(bptr, pos) != 0 {
        pos += 1;
    }
    if at(bptr, pos) == at(rptr, pos) {
        return Some(String::new());
    }
    // Back up to the last '/' in the URI: the start of its unique suffix.
    let mut ix = pos;
    while ix > 0 && rptr[ix - 1] != b'/' {
        ix -= 1;
    }
    let uptr = &rptr[ix..];
    // Count the base's '/' from the differing point.
    let nbslash = bptr
        .get(ix..)
        .unwrap_or(&[])
        .iter()
        .filter(|&&c| c == b'/')
        .count();
    if nbslash == 0 && uptr.is_empty() {
        return Some("./".to_string());
    }
    let uptr = String::from_utf8_lossy(uptr);
    if nbslash == 0 {
        return Some(escape_str(&uptr, "/;&=+$,"));
    }
    let mut val = "../".repeat(nbslash);
    match uptr.strip_prefix('/') {
        Some(rest) => val.push_str(rest),
        None => val.push_str(&uptr),
    }
    Some(escape_str(&val, "/;&=+$,"))
}

#[cfg(test)]
mod relative_tests {
    use super::build_relative;

    /// libxml2's own examples (uri.c), and the cases XInclude meets.
    #[test]
    fn relative_uris() {
        let b = Some("http://site1.com/docs/book1.html");
        assert_eq!(
            build_relative("http://site1.com/docs/pic1.gif", b).unwrap(),
            "pic1.gif"
        );
        assert_eq!(
            build_relative("http://site2.com/docs/pic1.gif", b).unwrap(),
            "http://site2.com/docs/pic1.gif"
        );
        let b = Some("docs/book1.html");
        assert_eq!(build_relative("docs/pic1.gif", b).unwrap(), "pic1.gif");
        assert_eq!(
            build_relative("docs/img/pic1.gif", b).unwrap(),
            "img/pic1.gif"
        );
        assert_eq!(
            build_relative("img/pic1.gif", b).unwrap(),
            "../img/pic1.gif"
        );
        assert_eq!(
            build_relative("/a/b/c.xml", Some("/a/main.xml")).unwrap(),
            "b/c.xml"
        );
        assert_eq!(
            build_relative("/a/c.xml", Some("/a/main.xml")).unwrap(),
            "c.xml"
        );
        assert_eq!(
            build_relative("/a/c.xml", Some("/a/main.xml")).unwrap(),
            "c.xml"
        );
        assert_eq!(build_relative("x.xml", None).unwrap(), "x.xml");
        assert_eq!(build_relative("", Some("a")), None);
    }
}
