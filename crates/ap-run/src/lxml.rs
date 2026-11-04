//! Native `lxml` compatibility shim (leaf 02.08-c) — no libxml2, no C deps.
//!
//! Why: the scrapy/parsel 1.12.1 import chain dies at `from lxml import etree,
//! html` because real lxml is a C extension (APX-CEXT policy: never fetch
//! native wheels). This module provides the audited T1 surface natively, over
//! a small pure-Rust HTML tokenizer + tree builder (lxml's recover parser is
//! lenient; XML mode stays strict for the well-formed documents our callers
//! parse — sitemaps, Atom feeds).
//!
//! Surface (plans/antipythony/02-native-backend/leaves/02.08-c-lxml-shim.md):
//! - lxml.etree: XMLParser, HTMLParser, fromstring, XMLSyntaxError, _Element,
//!   XPath, XPathError/XPathEvalError, ErrorTypes, SubElement, tostring,
//!   Element, LIBXML_VERSION/LXML_VERSION, cleanup_namespaces, iterparse
//!   (fail-loud marker), Comment factory
//! - lxml.html: HTMLParser, HtmlElement, fromstring, document_fromstring
//! - _Element protocol: .xpath(expr) subset (//tag, tag, .//tag, ./tag, @attr,
//!   @*, text(), [pred] with @attr='v' / @attr / [n] / [last()] /
//!   [position()>n] / contains(@class,'x') / [text()='v'] / [tag]),
//!   count()/string()/boolean()/normalize-space(), .tag, .attrib (live
//!   mapping), .text, .tail, .get(), .set(), .getparent(), .getprevious(),
//!   .getnext(), .iter([tag]), .find/.findall (ElementPath subset),
//!   .getchildren() (lxml 6 deprecated-but-present), len/indexing/iteration,
//!   .clear(), .itertext()
//! - HtmlElement adds: .body, .head, .base_url, .text_content(), .classes,
//!   .drop_tree(), .drop_tag(), .cssselect (routes through the pure-python
//!   cssselect bridge: the translator produces XPath text; our engine runs it)
//!
//! XPath return shapes (byte-exact vs CPython+real lxml):
//! - node selection  -> list of _Element/HtmlElement
//! - @attr           -> list of str
//! - text()          -> list of str
//! - count()         -> float
//! - string()        -> str (all descendant text concatenated)
//! - boolean()       -> bool
//!
//! Known gaps (docs/lxml-shim.md): XPath axes beyond child/descendant(-or-self)/
//! self, predicates beyond the subset above, libxml2 error-message parity,
//! attribute-namespace handling, CDATA node identity, processing instructions
//! as nodes, iterparse streaming.

use std::cell::RefCell;
use std::rc::Rc;

use crate::exc_hier::ExceptionValue;
use crate::Flow;
use ap_obj::dict::PyDict;
use ap_obj::int::Gc;
use ap_obj::value::{PyList, PyString, Tuple, Value};
use ap_obj::PyObject;

pub type BResult = Result<Value, Flow>;

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg.into()))
}

fn s(x: impl Into<String>) -> Value {
    Value::Str(PyString::new(x.into()))
}

fn i(n: i64) -> Value {
    Value::Int(ap_obj::int::Int::from_i64(n))
}

fn f(x: f64) -> Value {
    Value::Float(x)
}

// ===========================================================================
// Tree model (Rust side)
// ===========================================================================

#[derive(Clone)]
pub struct LxNode {
    /// Element tag, lowercased for HTML docs. `{urn}local` keeps its braces
    /// (namespace form, matching libxml2's tag surface).
    pub tag: String,
    /// Attributes in document order (lowercased keys for HTML docs).
    pub attrib: Vec<(String, String)>,
    /// Text before the first child (None when absent — lxml semantics).
    pub text: Option<String>,
    /// Text after this element's end tag.
    pub tail: Option<String>,
    pub children: Vec<usize>,
    pub parent: Option<usize>,
    /// True when parsed in HTML mode (case-insensitive tags/attrs, void elems).
    pub html: bool,
}

impl LxNode {
    fn new(tag: &str, html: bool) -> Self {
        LxNode {
            tag: tag.to_string(),
            attrib: Vec::new(),
            text: None,
            tail: None,
            children: Vec::new(),
            parent: None,
            html,
        }
    }
}

/// The document: arena of nodes + root index.
#[derive(Clone)]
pub struct LxDoc {
    pub nodes: Vec<LxNode>,
    pub root: usize,
    pub base_url: Option<String>,
}

impl LxDoc {
    fn get(&self, idx: usize) -> &LxNode {
        &self.nodes[idx]
    }
    fn get_mut(&mut self, idx: usize) -> &mut LxNode {
        &mut self.nodes[idx]
    }
}

/// The shared document every element object of one parse points at.
pub type SharedDoc = Rc<RefCell<LxDoc>>;

// ===========================================================================
// HTML tokenizer + lenient tree builder (libxml2-recover-flavored subset)
// ===========================================================================

/// Void elements (HTML5): never take children, serialize unclosed in html mode.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Elements whose content is RAWTEXT/RCDATA (parsed as text until the close).
const CDATA_ELEMENTS: &[&str] = &[
    "script", "style", "xmp", "iframe", "noembed", "noframes", "textarea", "title",
];

/// Auto-close table: when `open` is on the stack and `new` starts, the open
/// element closes first (subset of the HTML5 implied-end tree construction).
fn implies_end(open: &str, new: &str) -> bool {
    matches!(
        (open, new),
        (
            "p",
            "p" | "div"
                | "ul"
                | "ol"
                | "dl"
                | "table"
                | "form"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "blockquote"
                | "pre"
                | "hr"
                | "section"
                | "article"
                | "aside"
                | "header"
                | "footer"
                | "nav"
                | "address"
                | "fieldset"
        ) | ("li", "li")
            | ("dt", "dt" | "dd")
            | ("dd", "dt" | "dd")
            | ("option", "option" | "optgroup" | "select")
            | ("optgroup", "optgroup")
            | ("thead" | "tbody", "tbody" | "tfoot")
            | ("tr", "tr" | "tbody" | "tfoot")
            | ("td" | "th", "td" | "th" | "tr" | "tbody" | "tfoot")
            | ("head", "body")
    )
}

struct HtmlTreeBuilder {
    doc: LxDoc,
    /// Stack of open element indices (index 0 = synthetic #document).
    stack: Vec<usize>,
    html_mode: bool,
}

impl HtmlTreeBuilder {
    fn new(html_mode: bool) -> Self {
        let doc = LxDoc {
            nodes: vec![LxNode::new("#document", html_mode)],
            root: 0,
            base_url: None,
        };
        HtmlTreeBuilder {
            doc,
            stack: vec![0],
            html_mode,
        }
    }

    fn cur(&self) -> usize {
        *self.stack.last().unwrap()
    }

    fn append_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let cur = self.cur();
        let kids = &self.doc.nodes[cur].children;
        if let Some(&last) = kids.last() {
            self.doc.nodes[last].tail = Some(match self.doc.nodes[last].tail.take() {
                Some(t) => t + text,
                None => text.to_string(),
            });
        } else {
            self.doc.nodes[cur].text = Some(match self.doc.nodes[cur].text.take() {
                Some(t) => t + text,
                None => text.to_string(),
            });
        }
    }

    /// 02.08-c: self-closing XML elements (<a/>) must NOT stay on the open
    /// stack — the tokenizer emits them through open_element with no matching
    /// close tag, so the following sibling otherwise nests inside.
    fn open_self_closing(&mut self, tag: &str) -> usize {
        let idx = self.open_element(tag);
        self.stack.pop();
        idx
    }

    fn open_element(&mut self, tag: &str) -> usize {
        // Auto-close per the implied-end table.
        while self.stack.len() > 1 {
            let cur_tag = self.doc.get(self.cur()).tag.clone();
            if implies_end(&cur_tag, tag) {
                self.stack.pop();
            } else {
                break;
            }
        }
        let name = if self.html_mode {
            tag.to_lowercase()
        } else {
            tag.to_string()
        };
        let idx = self.doc.nodes.len();
        self.doc.nodes.push(LxNode::new(&name, self.html_mode));
        let cur = self.cur();
        self.doc.nodes[idx].parent = Some(cur);
        self.doc.nodes[cur].children.push(idx);
        if !VOID_ELEMENTS.contains(&name.as_str()) {
            self.stack.push(idx);
        }
        idx
    }

    fn close_element(&mut self, tag: &str) {
        let name = if self.html_mode {
            tag.to_lowercase()
        } else {
            tag.to_string()
        };
        // Find the nearest matching open element; ignore stray closes.
        if let Some(pos) = self
            .stack
            .iter()
            .rposition(|&ix| self.doc.get(ix).tag == name)
        {
            while self.stack.len() > pos + 1 {
                self.stack.pop();
            }
            self.stack.pop();
        }
    }

    /// HTML mode wraps stray top-level content in html/body like libxml2's
    /// recover parser: `fromstring` always returns the html element.
    fn finish(mut self) -> LxDoc {
        if self.html_mode {
            let top: Vec<usize> = self.doc.get(0).children.clone();
            let needs_wrap = top.len() != 1 || self.doc.get(top[0]).tag != "html";
            if needs_wrap {
                let html_ix = self.doc.nodes.len();
                self.doc.nodes.push(LxNode::new("html", true));
                self.doc.nodes[html_ix].parent = Some(0);
                let body_ix = self.doc.nodes.len();
                self.doc.nodes.push(LxNode::new("body", true));
                self.doc.nodes[body_ix].parent = Some(html_ix);
                self.doc.nodes[html_ix].children.push(body_ix);
                let old = std::mem::take(&mut self.doc.nodes[0].children);
                for c in old {
                    self.doc.nodes[c].parent = Some(body_ix);
                    self.doc.nodes[body_ix].children.push(c);
                }
                // Text parked on #document (bare-text input) moves to body.
                if let Some(dt) = self.doc.get_mut(0).text.take() {
                    let node = self.doc.get_mut(body_ix);
                    node.text = Some(match node.text.take() {
                        Some(t) => t + &dt,
                        None => dt,
                    });
                }
                // lxml fragment rule: text-only input wraps in <span>.
                if self.doc.get(body_ix).children.is_empty() {
                    let txt = self.doc.get_mut(body_ix).text.take().unwrap_or_default();
                    let span_ix = self.doc.nodes.len();
                    self.doc.nodes.push(LxNode::new("span", true));
                    self.doc.nodes[span_ix].parent = Some(body_ix);
                    self.doc.nodes[span_ix].text = Some(txt);
                    self.doc.nodes[body_ix].children.push(span_ix);
                }
                self.doc.root = html_ix;
            } else {
                let html_ix = top[0];
                let has_body = self
                    .doc
                    .get(html_ix)
                    .children
                    .iter()
                    .any(|&c| self.doc.get(c).tag == "body");
                if !has_body {
                    let body_ix = self.doc.nodes.len();
                    self.doc.nodes.push(LxNode::new("body", true));
                    self.doc.nodes[body_ix].parent = Some(html_ix);
                    self.doc.nodes[html_ix].children.push(body_ix);
                }
                self.doc.root = html_ix;
            }
        } else {
            // XML mode: root = first top-level element.
            self.doc.root = self.doc.get(0).children.first().copied().unwrap_or(0);
        }
        self.doc
    }
}

/// Unescape character references (numeric + the named set that occurs in web
/// documents and tests; unknown refs pass through verbatim, like html.unescape
/// for names it lacks).
fn unescape_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi) = text[i + 1..].find(';') {
                let name = &text[i + 1..i + 1 + semi];
                let rep = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some('\u{a0}'),
                    _ => {
                        if let Some(rest) = name.strip_prefix('#') {
                            let cp = if let Some(hex) =
                                rest.strip_prefix('x').or_else(|| rest.strip_prefix('X'))
                            {
                                u32::from_str_radix(hex, 16).ok()
                            } else {
                                rest.parse::<u32>().ok()
                            };
                            cp.and_then(char::from_u32)
                        } else {
                            None
                        }
                    }
                };
                if let Some(c) = rep {
                    out.push(c);
                    i += semi + 2;
                    continue;
                }
            }
        }
        // Advance one UTF-8 code point.
        let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn parse_html(text: &str, html_mode: bool) -> LxDoc {
    let mut b = HtmlTreeBuilder::new(html_mode);
    let src = text;
    let n = src.len();
    let mut i = 0;
    while i < n {
        let rest = &src[i..];
        let Some(lt_off) = rest.find('<') else {
            b.append_text(&unescape_entities(rest));
            break;
        };
        if lt_off > 0 {
            b.append_text(&unescape_entities(&rest[..lt_off]));
        }
        let at = i + lt_off;
        let after = &src[at..];
        if after.starts_with("<!--") {
            match after.find("-->") {
                Some(end) => i = at + end + 3,
                None => i = n,
            }
            continue;
        }
        if after.len() >= 9 && after[..9].eq_ignore_ascii_case("<![cdata[") {
            match after.find("]]>") {
                Some(end) => {
                    b.append_text(&after[9..end]);
                    i = at + end + 3;
                }
                None => {
                    b.append_text(&after[9..]);
                    i = n;
                }
            }
            continue;
        }
        if after.starts_with("<!") || after.starts_with("<?") {
            // DOCTYPE / PI: skipped (lxml keeps comments/PIs as nodes; our
            // subset drops them — documented gap, xpath //comment() outside).
            match after.find('>') {
                Some(end) => i = at + end + 1,
                None => i = n,
            }
            continue;
        }
        if after.starts_with("</") {
            let name_end = after[2..].find('>').map(|p| 2 + p).unwrap_or(after.len());
            let inner = after[2..name_end].trim();
            let tag: String = inner
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_string();
            if !tag.is_empty() {
                b.close_element(&tag);
            }
            i = if name_end < n { at + name_end + 1 } else { n };
            continue;
        }
        // Start tag?
        let c0 = after[1..].chars().next();
        if !matches!(c0, Some(c) if c.is_ascii_alphabetic()) {
            // Lone '<' as data.
            b.append_text("<");
            i = at + 1;
            continue;
        }
        // Parse the start tag: name + attributes up to the matching '>'
        // (respecting quoted attribute values).
        let bytes = after.as_bytes();
        let mut j = 1;
        let name_start = j;
        while j < bytes.len()
            && (bytes[j].is_ascii_alphanumeric()
                || bytes[j] == b'-'
                || bytes[j] == b'_'
                || bytes[j] == b':'
                || bytes[j] == b'.')
        {
            j += 1;
        }
        let tag_name = after[name_start..j].to_string();
        let mut attrs: Vec<(String, Option<String>)> = Vec::new();
        let mut self_closing = false;
        while j < bytes.len() {
            while j < bytes.len() && (bytes[j] as char).is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'/' {
                let mut k = j + 1;
                while k < bytes.len() && (bytes[k] as char).is_ascii_whitespace() {
                    k += 1;
                }
                if k < bytes.len() && bytes[k] == b'>' {
                    self_closing = true;
                    j = k + 1;
                    break;
                }
                j += 1;
                continue;
            }
            if j < bytes.len() && bytes[j] == b'>' {
                j += 1;
                break;
            }
            // Attribute name.
            let an_start = j;
            while j < bytes.len()
                && !(bytes[j] as char).is_ascii_whitespace()
                && bytes[j] != b'='
                && bytes[j] != b'>'
                && bytes[j] != b'/'
            {
                j += 1;
            }
            if j == an_start {
                j += 1; // defensive: skip a stray char
                continue;
            }
            let an = after[an_start..j].to_string();
            // Optional =value.
            let mut k = j;
            while k < bytes.len() && (bytes[k] as char).is_ascii_whitespace() {
                k += 1;
            }
            let val = if k < bytes.len() && bytes[k] == b'=' {
                k += 1;
                while k < bytes.len() && (bytes[k] as char).is_ascii_whitespace() {
                    k += 1;
                }
                if k < bytes.len() && (bytes[k] == b'"' || bytes[k] == b'\'') {
                    let q = bytes[k] as char;
                    let vs = k + 1;
                    let ve = after[vs..].find(q).map(|p| vs + p).unwrap_or(after.len());
                    let v = after[vs..ve].to_string();
                    j = (ve + 1).min(after.len());
                    Some(v)
                } else {
                    let vs = k;
                    while k < bytes.len()
                        && !(bytes[k] as char).is_ascii_whitespace()
                        && bytes[k] != b'>'
                    {
                        k += 1;
                    }
                    let v = after[vs..k].to_string();
                    j = k;
                    Some(v)
                }
            } else {
                j = k;
                None
            };
            let an = if b.html_mode { an.to_lowercase() } else { an };
            attrs.push((an, val));
        }
        i = at + j.min(after.len());
        // Emit the element. Self-closing XML elements pop immediately.
        let opened = if self_closing && !b.html_mode {
            b.open_self_closing(&tag_name)
        } else {
            b.open_element(&tag_name)
        };
        {
            let node = &mut b.doc.nodes[opened];
            for (an, av) in attrs {
                let av = match av {
                    Some(v) => unescape_entities(&v),
                    None => {
                        if node.html {
                            // Boolean attribute: libxml2 surfaces the NAME.
                            an.clone()
                        } else {
                            String::new()
                        }
                    }
                };
                if !node.attrib.iter().any(|(k, _)| *k == an) {
                    node.attrib.push((an, av));
                }
            }
        }
        // RAWTEXT content (script/style/title/...): consume verbatim until
        // the matching end tag.
        let lname = b.doc.nodes[opened].tag.clone();
        if b.html_mode
            && CDATA_ELEMENTS.contains(&lname.as_str())
            && !self_closing
            && b.stack.last() == Some(&opened)
        {
            let close_pat = format!("</{}", lname);
            let after_tag = &src[i..];
            match after_tag.to_lowercase().find(&close_pat) {
                Some(p) => {
                    let raw = &after_tag[..p];
                    if !raw.is_empty() {
                        // RAWTEXT is NOT entity-decoded for script/style;
                        // RCDATA (entity-decoded) for textarea/title.
                        let decoded = if lname == "textarea" || lname == "title" {
                            unescape_entities(raw)
                        } else {
                            raw.to_string()
                        };
                        b.append_text(&decoded);
                    }
                    let gt = src[i + p..].find('>').map(|g| i + p + g).unwrap_or(n);
                    b.close_element(&lname);
                    i = gt + 1;
                }
                None => {
                    b.append_text(after_tag);
                    b.close_element(&lname);
                    i = n;
                }
            }
            continue;
        }
    }
    b.finish()
}

// ===========================================================================
// Serialization (lxml tostring parity for the covered subset)
// ===========================================================================

fn xml_escape_text(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(c),
        }
    }
    out
}

fn xml_escape_attr(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// libxml2's HTML serializer emits these attribute names WITHOUT values
/// (HTML boolean attributes), regardless of the stored value.
fn html_attr_is_bool(name: &str) -> bool {
    matches!(
        name,
        "checked"
            | "compact"
            | "declare"
            | "defer"
            | "disabled"
            | "ismap"
            | "multiple"
            | "nohref"
            | "noresize"
            | "noshade"
            | "nowrap"
            | "readonly"
            | "selected"
    )
}

fn html_escape_attr(t: &str) -> String {
    // lxml's html serializer switches to single quotes ONLY when the value
    // contains a double quote and no single quote; otherwise double quotes.
    if t.contains('"') && !t.contains('\'') {
        format!(
            "'{}'",
            t.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        )
    } else {
        format!("\"{}\"", xml_escape_attr(t))
    }
}

fn serialize(doc: &LxDoc, idx: usize, method: &str, with_tail: bool, out: &mut String) {
    let node = doc.get(idx);
    let html_mode = method == "html";
    let empty = node.children.is_empty() && node.text.as_deref().unwrap_or("").is_empty();
    if html_mode && VOID_ELEMENTS.contains(&node.tag.as_str()) {
        out.push('<');
        out.push_str(&node.tag);
        for (k, v) in &node.attrib {
            out.push(' ');
            out.push_str(k);
            if !(html_mode && html_attr_is_bool(k)) {
                out.push('=');
                out.push_str(&html_escape_attr(v));
            }
        }
        out.push('>');
    } else if empty {
        out.push('<');
        out.push_str(&node.tag);
        for (k, v) in &node.attrib {
            out.push(' ');
            out.push_str(k);
            if !(html_mode && html_attr_is_bool(k)) {
                out.push('=');
                let esc = if html_mode {
                    html_escape_attr(v)
                } else {
                    format!("\"{}\"", xml_escape_attr(v))
                };
                out.push_str(&esc);
            }
        }
        if html_mode {
            out.push_str("></");
            out.push_str(&node.tag);
            out.push('>');
        } else {
            out.push_str("/>");
        }
    } else {
        out.push('<');
        out.push_str(&node.tag);
        for (k, v) in &node.attrib {
            out.push(' ');
            out.push_str(k);
            if !(html_mode && html_attr_is_bool(k)) {
                out.push('=');
                let esc = if html_mode {
                    html_escape_attr(v)
                } else {
                    format!("\"{}\"", xml_escape_attr(v))
                };
                out.push_str(&esc);
            }
        }
        out.push('>');
        if let Some(t) = &node.text {
            out.push_str(&xml_escape_text(t));
        }
        for &c in &node.children {
            serialize(doc, c, method, with_tail, out);
        }
        out.push_str("</");
        out.push_str(&node.tag);
        out.push('>');
    }
    if with_tail {
        if let Some(t) = &node.tail {
            out.push_str(&xml_escape_text(t));
        }
    }
}

/// Full concatenated text (string() / text_content / itertext surface):
/// text + descendants' texts + descendants' tails (lxml string() semantics).
fn collect_string_content(doc: &LxDoc, idx: usize, out: &mut String) {
    let node = doc.get(idx);
    if let Some(t) = &node.text {
        out.push_str(t);
    }
    for &c in &node.children {
        collect_string_content(doc, c, out);
        if let Some(t) = &doc.get(c).tail {
            out.push_str(t);
        }
    }
}

/// Direct-child text values (//el/text() results, in document order):
/// the element's own text + each child's tail.
fn child_texts(doc: &LxDoc, idx: usize, out: &mut Vec<String>) {
    let node = doc.get(idx);
    if let Some(t) = &node.text {
        out.push(t.clone());
    }
    for &c in &node.children {
        if let Some(t) = &doc.get(c).tail {
            out.push(t.clone());
        }
    }
}

// ===========================================================================
// XPath subset engine
// ===========================================================================

#[derive(Debug, Clone, PartialEq)]
enum Step {
    /// //tag — all descendants with tag ("*" = any)
    Descendant(String),
    /// tag — direct children with tag ("*" = any)
    Child(String),
    /// `.` / absolute-root marker — the context node (root for '/')
    SelfAxis,
    /// @attr or @* — attribute values of the context node
    Attribute(String),
    /// text() — direct child text of the context node
    Text,
}

#[derive(Debug, Clone)]
enum Pred {
    /// [@attr='v'] / [@attr] / [tag] / [text()='v'] / [contains(@class,'x')]
    Expr(String),
    /// [n] position (1-based)
    Position(i64),
    /// [last()]
    Last,
    /// [position()>n]
    PositionGt(i64),
}

#[derive(Debug, Clone)]
struct PathExpr {
    steps: Vec<Step>,
    /// Per-step predicates: step_preds[i] filters the node-set produced by
    /// steps[i] (XPath applies [n] to the step it follows, not the output).
    step_preds: Vec<Option<Pred>>,
    /// Legacy trailing predicate (applies to the final node-set); kept for
    /// paths whose pred attaches after a text()/@attr output step.
    pred: Option<Pred>,
    /// /@attr output filter after node selection
    out_attr: Option<String>,
    /// trailing text() (output = direct texts of selected nodes)
    out_text: bool,
    /// absolute path ('/root') — evaluation starts at the document root
    absolute: bool,
}

enum XValue {
    Nodes(Vec<usize>),
    Attrs(Vec<String>),
    Texts(Vec<String>),
    Str(String),
    Float(f64),
    Bool(bool),
}

fn split_top_level(expr: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    let mut in_q: Option<char> = None;
    for c in expr.chars() {
        if let Some(q) = in_q {
            cur.push(c);
            if c == q {
                in_q = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                in_q = Some(c);
                cur.push(c);
            }
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            _ if c == sep && depth == 0 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    parts
}

fn last_top_level_slash_at(expr: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_q: Option<char> = None;
    let mut last = None;
    for (idx, c) in expr.char_indices() {
        if let Some(q) = in_q {
            if c == q {
                in_q = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => in_q = Some(c),
            '(' => depth += 1,
            ')' => depth -= 1,
            '/' if depth == 0 => last = Some(idx),
            _ => {}
        }
    }
    last
}

fn parse_path(expr: &str) -> Option<PathExpr> {
    let mut expr = expr.trim();
    let mut out_attr = None;
    let mut out_text = false;
    // Trailing /text().
    if expr.ends_with("text()") {
        let head_len = expr.len() - "text()".len();
        if head_len > 0 && expr[..head_len].ends_with('/') {
            out_text = true;
            expr = expr[..head_len - 1].trim_end();
        }
    }
    // Trailing /@attr or /@*.
    if let Some(pos) = last_top_level_slash_at(expr) {
        let tail = expr[pos + 1..].trim();
        if let Some(attr) = tail.strip_prefix('@') {
            if !attr.is_empty()
                && attr
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ':' || c == '*')
            {
                out_attr = Some(attr.to_string());
                expr = expr[..pos].trim_end();
            }
        }
    }
    let mut steps = Vec::new();
    let mut pred = None;
    let mut absolute = false;
    if expr.is_empty() {
        return None;
    }
    let mut step_preds: Vec<Option<Pred>> = Vec::new();
    // Leading //, /, ./, .//, or bare `.`.
    let mut rest = expr;
    if let Some(r) = rest.strip_prefix("//") {
        // `//tag` == descendant-or-self::node()/tag. The next path segment is
        // the tag: peek it and fold into a single Descendant(name) step (a
        // Child after Descendant("*") would only look at the descendants'
        // CHILDREN, missing the descendants themselves).
        let head = r;
        let seg_end = head.find(|c| c == '/').unwrap_or(head.len());
        // Strip a trailing predicate before the identifier peek so `//li[1]`
        // folds to SelfAxis+Descendant(li)+pred (identical node-set to
        // //li) instead of the Descendant(*)+Child fallback.
        let first_seg_raw = head[..seg_end].trim();
        let (first_seg, first_pred) = match first_seg_raw.find('[') {
            Some(op) if first_seg_raw.ends_with(']') => (
                first_seg_raw[..op].trim(),
                Some(&first_seg_raw[op + 1..first_seg_raw.len() - 1]),
            ),
            _ => (first_seg_raw, None),
        };
        if let Some(name) = first_seg.strip_prefix('@') {
            steps.push(Step::Descendant("*".into()));
            steps.push(Step::Child("*".into()));
            // out_attr handles the selection; keep the attr as the next step.
            steps.push(Step::Attribute(name.trim().to_string()));
            rest = &head[seg_end..];
        } else if first_seg == "*" {
            steps.push(Step::Descendant("*".into()));
            rest = &head[seg_end..];
        } else if !first_seg.is_empty()
            && first_seg
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ':')
        {
            steps.push(Step::SelfAxis);
            steps.push(Step::Descendant(first_seg.to_string()));
            if let Some(pv) = first_pred {
                // `//li[1]` — pred rides the Descendant step.
                while step_preds.len() < steps.len() - 1 {
                    step_preds.push(None);
                }
                step_preds.push(Some(parse_predicate_body(pv)?));
            }
            rest = &head[seg_end..];
        } else {
            steps.push(Step::Descendant("*".into()));
            rest = r;
        }
    } else if let Some(r) = rest.strip_prefix(".//") {
        // NOTE: must be checked BEFORE "./" — ".//ul" starts with "./"!
        steps.push(Step::SelfAxis);
        let seg_end = r.find('/').unwrap_or(r.len());
        let first_seg = r[..seg_end].trim();
        if first_seg == "*" || first_seg.is_empty() {
            steps.push(Step::Descendant("*".into()));
            rest = &r[seg_end..];
        } else if first_seg
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ':')
        {
            steps.push(Step::Descendant(first_seg.to_string()));
            rest = &r[seg_end..];
        } else {
            steps.push(Step::Descendant("*".into()));
            rest = r;
        }
    } else if let Some(r) = rest.strip_prefix("./") {
        steps.push(Step::SelfAxis);
        rest = r;
    } else if rest == "." {
        steps.push(Step::SelfAxis);
        rest = "";
    } else if let Some(r) = rest.strip_prefix('/') {
        // Absolute from the document root: no SelfAxis step (the evaluator
        // seeds from doc.root when `absolute`).
        absolute = true;
        rest = r;
    }
    // Remaining path segments (slashes outside brackets/quotes).
    let segs = split_top_level(rest, '/');
    for seg in segs {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        let (path_part, p) = split_predicate(seg)?;
        // A pred on THIS segment belongs to the step this segment pushes.
        // If the segment is an output step (text()/@attr), it lands in the
        // legacy trailing slot instead (applied to the output values).
        let steps_before = steps.len();
        let _ = steps_before;
        if path_part.contains("::") {
            let (axis, name) = path_part.split_once("::")?;
            match axis.trim() {
                "descendant-or-self" | "descendant" => {
                    steps.push(Step::Descendant(name.trim().to_string()))
                }
                "child" => steps.push(Step::Child(name.trim().to_string())),
                "self" => steps.push(Step::SelfAxis),
                _ => return None,
            }
            continue;
        }
        if path_part == "." {
            steps.push(Step::SelfAxis);
        } else if path_part == "*" {
            steps.push(Step::Child("*".into()));
        } else if path_part.starts_with('@') {
            steps.push(Step::Attribute(path_part[1..].trim().to_string()));
        } else {
            // Plain child-name step — validate identifier-ish.
            if path_part.is_empty()
                || !path_part
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ':' || c == '.')
            {
                return None;
            }
            steps.push(Step::Child(path_part.to_string()));
        }
        // Attach the segment's pred to the step this segment pushed (output
        // steps text()/@attr RETURN from eval directly, so their preds use
        // the legacy trailing slot instead).
        if steps.len() > step_preds.len() {
            // A node-selecting step was pushed: own the pred here.
            while step_preds.len() < steps.len() - 1 {
                step_preds.push(None);
            }
            if steps.len() > step_preds.len() {
                step_preds.push(p);
            } else {
                pred = p;
            }
        } else {
            // Output step (Attribute/Text) - pred filters output values.
            if p.is_some() {
                if pred.is_some() {
                    return None;
                }
                pred = p;
            }
        }
    }
    if steps.is_empty() {
        return None;
    }
    // `.` alone is meaningful; a leading SelfAxis before Descendant from the
    // `//tag` fold is KEPT (descendant-or-self semantics — the context node
    // itself is a candidate when it matches the tag).
    Some(PathExpr {
        steps,
        step_preds,
        pred,
        out_attr,
        out_text,
        absolute,
    })
}

fn parse_predicate_body(t: &str) -> Option<Pred> {
    let t = t.trim();
    if t == "last()" {
        Some(Pred::Last)
    } else if let Some(rest) = t.strip_prefix("position()>") {
        Some(Pred::PositionGt(rest.trim().parse().ok()?))
    } else if let Ok(n) = t.parse::<i64>() {
        Some(Pred::Position(n))
    } else {
        if !pred_supported(t) {
            return None;
        }
        Some(Pred::Expr(t.to_string()))
    }
}

fn split_predicate(seg: &str) -> Option<(&str, Option<Pred>)> {
    let Some(open) = seg.find('[') else {
        return Some((seg, None));
    };
    if !seg.ends_with(']') {
        return None;
    }
    let inner = &seg[open + 1..seg.len() - 1];
    // Nested brackets not in the subset.
    if inner.contains('[') {
        return None;
    }
    let path = &seg[..open];
    Some((path, Some(parse_predicate_body(inner)?)))
}

fn pred_supported(t: &str) -> bool {
    // [@attr='v'] / [@attr] / [tag] / [text()='v'] / [text()] /
    // [contains(@class,'x')]
    if t.starts_with("contains(") && t.ends_with(')') {
        let inner = &t["contains(".len()..t.len() - 1];
        let args = split_top_level(inner, ',');
        return args.len() == 2
            && args[0].trim().starts_with('@')
            && (args[1].trim().starts_with('\'') || args[1].trim().starts_with('"'));
    }
    if let Some(rest) = t.strip_prefix('@') {
        if let Some((a, v)) = rest.split_once('=') {
            let a = a.trim();
            let v = v.trim();
            return !a.is_empty()
                && ((v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
                    || (v.starts_with('"') && v.ends_with('"') && v.len() >= 2));
        }
        let r = rest.trim();
        return !r.is_empty()
            && r.chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ':');
    }
    if let Some(rest) = t.strip_prefix("text()") {
        let rest = rest.trim_start();
        if rest.is_empty() {
            return true;
        }
        if let Some(v) = rest.strip_prefix('=') {
            let v = v.trim();
            return (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
                || (v.starts_with('"') && v.ends_with('"') && v.len() >= 2);
        }
        return false;
    }
    // [tag] child-existence predicate.
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '*' || c == ':')
}

/// Evaluate a parsed path against the tree; context node = `ctx`.
fn eval_path(doc: &LxDoc, ctx: usize, pe: &PathExpr) -> Option<XValue> {
    let mut current: Vec<usize> = vec![if pe.absolute { doc.root } else { ctx }];
    let mut first_step = true;
    // True while the path is still inside the leading `//` fold's
    // descendant-or-self semantics (`//tag` parses to SelfAxis + Descendant).
    // `./tag` (SelfAxis + Child) must NOT self-insert — child::a on <a>
    // correctly yields nothing.
    let mut desc_ctx = matches!(
        (pe.steps.first(), pe.steps.get(1)),
        (Some(Step::SelfAxis), Some(Step::Descendant(_)))
    );
    for (step_i, step) in pe.steps.iter().enumerate() {
        let mut next = Vec::new();
        match step {
            Step::SelfAxis => {
                // No-op on the context: the ABSOLUTE-root marker ('/') was
                // folded away in the relative './/' parse (a leading SelfAxis
                // followed by Descendant collapses), so SelfAxis here only
                // appears for './' — keep the context.
                next = current.clone();
            }
            Step::Child(name) => {
                for &n in &current {
                    // Absolute paths: the first child hop is the
                    // document-node -> root-element edge, i.e. /root matches
                    // the root element itself (its tag = doc.root's tag).
                    // 02.08-c: a leading SelfAxis (`//a` → SelfAxis +
                    // Descendant) puts the CONTEXT ELEMENT itself in
                    // `current`; the descendant step misses it when the
                    // element IS the match (root == <a>). libxml2's
                    // descendant-or-self::node()/child::a semantics: the
                    // context participates via the self hop. Handle by also
                    // testing the node itself when it is the context root
                    // and the absolute/`//` path addressed it.
                    // 02.08-c: `//a` (SelfAxis+Descendant) — when the context
                    // element itself IS the match (root == a) the Descendant
                    // step finds nothing; CPython's descendant-or-self
                    // includes the context. Re-match it here.
                    if first_step && n == ctx && (name == "*" || &doc.get(n).tag == name) {
                        let descendants: Vec<usize> = {
                            let mut out = Vec::new();
                            collect_descendants(doc, n, name, &mut out);
                            out
                        };
                        if descendants.is_empty() {
                            next.push(n);
                            continue;
                        }
                    }
                    if pe.absolute && first_step && (name == "*" || &doc.get(n).tag == name) {
                        next.push(n);
                        continue;
                    }
                    for &c in &doc.get(n).children {
                        if name == "*" || &doc.get(c).tag == name {
                            next.push(c);
                        }
                    }
                }
            }
            Step::Descendant(name) => {
                for &n in &current {
                    collect_descendants(doc, n, name, &mut next);
                }
                // 02.08-c: descendant-or-self semantics from the `//tag`
                // fold — the context node itself is a candidate when it
                // matches (root == a; `//a` must find it). Keyed on the
                // fold shape (SelfAxis + Descendant), NOT `first_step`:
                // the leading SelfAxis hop consumes first_step before the
                // Descendant step runs, so the old gate never fired for
                // `//tag` on a context element that IS the match.
                if desc_ctx && current.len() == 1 && current[0] == ctx {
                    if name == "*" || &doc.get(ctx).tag == name {
                        next.insert(0, ctx);
                    }
                    desc_ctx = false;
                }
            }
            Step::Attribute(name) => {
                let mut vals = Vec::new();
                for &n in &current {
                    for (k, v) in &doc.get(n).attrib {
                        if name == "*" || k == name {
                            vals.push(v.clone());
                        }
                    }
                }
                return Some(XValue::Attrs(vals));
            }
            Step::Text => {
                let mut texts = Vec::new();
                for &n in &current {
                    child_texts(doc, n, &mut texts);
                }
                return Some(XValue::Texts(texts));
            }
        }
        current = next;
        // Per-step predicate: XPath applies [n]/[@a] to the node-set the
        // step produced, BEFORE the next axis runs.
        if let Some(Some(p)) = pe.step_preds.get(step_i) {
            current = apply_pred(doc, current, p)?;
        }
    }
    if let Some(p) = &pe.pred {
        current = apply_pred(doc, current, p)?;
    }
    if pe.out_text {
        let mut texts = Vec::new();
        for &n in &current {
            child_texts(doc, n, &mut texts);
        }
        return Some(XValue::Texts(texts));
    }
    if let Some(attr) = &pe.out_attr {
        let mut vals = Vec::new();
        for &n in &current {
            for (k, v) in &doc.get(n).attrib {
                if attr == "*" || k == attr {
                    vals.push(v.clone());
                }
            }
        }
        return Some(XValue::Attrs(vals));
    }
    Some(XValue::Nodes(current))
}

fn collect_descendants(doc: &LxDoc, idx: usize, name: &str, out: &mut Vec<usize>) {
    for &c in &doc.get(idx).children {
        if name == "*" || doc.get(c).tag == name {
            out.push(c);
        }
        collect_descendants(doc, c, name, out);
    }
}

fn apply_pred(doc: &LxDoc, nodes: Vec<usize>, p: &Pred) -> Option<Vec<usize>> {
    match p {
        Pred::Last => nodes.last().map(|&n| vec![n]),
        Pred::Position(pos) => {
            if *pos >= 1 {
                nodes.get((*pos - 1) as usize).map(|&n| vec![n])
            } else {
                Some(vec![])
            }
        }
        Pred::PositionGt(n) => {
            if nodes.len() as i64 > *n {
                Some(nodes[(*n as usize).max(0)..].to_vec())
            } else {
                Some(vec![])
            }
        }
        Pred::Expr(e) => {
            let mut out = Vec::new();
            for &n in &nodes {
                if pred_matches(doc, n, e) {
                    out.push(n);
                }
            }
            Some(out)
        }
    }
}

fn attr_value<'a>(node: &'a LxNode, name: &str) -> Option<&'a str> {
    node.attrib
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn pred_matches(doc: &LxDoc, n: usize, e: &str) -> bool {
    let t = e.trim();
    let node = doc.get(n);
    if t.starts_with("contains(") && t.ends_with(')') {
        let inner = &t["contains(".len()..t.len() - 1];
        let args = split_top_level(inner, ',');
        let attr = args[0].trim().strip_prefix('@').unwrap_or("");
        let needle = unquote(args[1].trim());
        return attr_value(node, attr)
            .map(|v| v.contains(&needle))
            .unwrap_or(false);
    }
    if let Some(rest) = t.strip_prefix('@') {
        if let Some((a, v)) = rest.split_once('=') {
            return attr_value(node, a.trim()) == Some(unquote(v.trim()).as_str());
        }
        return attr_value(node, rest.trim()).is_some();
    }
    if t == "text()" {
        let mut texts = Vec::new();
        child_texts(doc, n, &mut texts);
        return !texts.is_empty();
    }
    if let Some(rest) = t.strip_prefix("text()") {
        let rest = rest.trim_start();
        if let Some(v) = rest.strip_prefix('=') {
            let want = unquote(v.trim());
            let mut texts = Vec::new();
            child_texts(doc, n, &mut texts);
            return texts.iter().any(|x| *x == want);
        }
        return false;
    }
    // [tag] child-existence predicate.
    node.children
        .iter()
        .any(|&c| t == "*" || doc.get(c).tag == t)
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2
        && ((v.starts_with('\'') && v.ends_with('\'')) || (v.starts_with('"') && v.ends_with('"')))
    {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

/// CPython float repr (lxml count() returns a Python float; str() of it goes
/// through repr in format paths).
fn fmt_float_plain(x: f64) -> String {
    crate::builtins::py_float_repr_pub(x)
}

/// Top-level XPath entry: None = expression outside the supported subset
/// (caller raises XPathEvalError like libxml2 does for bad expressions).
fn xpath_eval(doc: &LxDoc, ctx: usize, expr: &str) -> Option<XValue> {
    let expr = expr.trim();
    if expr == "string()" || expr == "string(.)" {
        let mut out = String::new();
        collect_string_content(doc, ctx, &mut out);
        return Some(XValue::Str(out));
    }
    if expr == "normalize-space()" {
        let mut out = String::new();
        collect_string_content(doc, ctx, &mut out);
        return Some(XValue::Str(
            out.split_whitespace().collect::<Vec<_>>().join(" "),
        ));
    }
    if let Some(rest) = expr.strip_prefix("string(") {
        let inner = rest.strip_suffix(')')?;
        return match xpath_eval(doc, ctx, inner)? {
            XValue::Nodes(nodes) => {
                let mut out = String::new();
                if let Some(&first) = nodes.first() {
                    collect_string_content(doc, first, &mut out);
                }
                Some(XValue::Str(out))
            }
            XValue::Attrs(mut a) => Some(XValue::Str(
                a.first_mut().map(std::mem::take).unwrap_or_default(),
            )),
            XValue::Texts(mut t) => Some(XValue::Str(
                t.first_mut().map(std::mem::take).unwrap_or_default(),
            )),
            XValue::Str(x) => Some(XValue::Str(x)),
            XValue::Float(x) => Some(XValue::Str(fmt_float_plain(x))),
            XValue::Bool(b) => Some(XValue::Str(if b { "true" } else { "false" }.into())),
        };
    }
    if let Some(rest) = expr.strip_prefix("count(") {
        let inner = rest.strip_suffix(')')?;
        return match xpath_eval(doc, ctx, inner)? {
            XValue::Nodes(n) => Some(XValue::Float(n.len() as f64)),
            XValue::Attrs(a) => Some(XValue::Float(a.len() as f64)),
            XValue::Texts(t) => Some(XValue::Float(t.len() as f64)),
            XValue::Str(_) => Some(XValue::Float(1.0)),
            XValue::Float(x) => Some(XValue::Float(x)),
            XValue::Bool(b) => Some(XValue::Float(b as i32 as f64)),
        };
    }
    if let Some(rest) = expr.strip_prefix("boolean(") {
        let inner = rest.strip_suffix(')')?;
        return match xpath_eval(doc, ctx, inner)? {
            XValue::Nodes(n) => Some(XValue::Bool(!n.is_empty())),
            XValue::Attrs(a) => Some(XValue::Bool(!a.is_empty())),
            XValue::Texts(t) => Some(XValue::Bool(!t.is_empty())),
            XValue::Str(s) => Some(XValue::Bool(!s.is_empty())),
            XValue::Float(x) => Some(XValue::Bool(x != 0.0)),
            XValue::Bool(b) => Some(XValue::Bool(b)),
        };
    }
    if let Some(rest) = expr.strip_prefix("normalize-space(") {
        let inner = rest.strip_suffix(')')?;
        return match xpath_eval(doc, ctx, inner)? {
            XValue::Nodes(nodes) => {
                let mut out = String::new();
                if let Some(&first) = nodes.first() {
                    collect_string_content(doc, first, &mut out);
                }
                Some(XValue::Str(
                    out.split_whitespace().collect::<Vec<_>>().join(" "),
                ))
            }
            XValue::Attrs(mut a) => Some(XValue::Str(
                a.first_mut()
                    .map(|x| x.split_whitespace().collect::<Vec<_>>().join(" "))
                    .unwrap_or_default(),
            )),
            XValue::Texts(mut t) => Some(XValue::Str(
                t.first_mut()
                    .map(|x| x.split_whitespace().collect::<Vec<_>>().join(" "))
                    .unwrap_or_default(),
            )),
            XValue::Str(x) => Some(XValue::Str(
                x.split_whitespace().collect::<Vec<_>>().join(" "),
            )),
            o => Some(XValue::Str(match o {
                XValue::Float(x) => fmt_float_plain(x),
                XValue::Bool(b) => if b { "true" } else { "false" }.into(),
                _ => String::new(),
            })),
        };
    }
    let pe = parse_path(expr)?;
    eval_path(doc, ctx, &pe)
}

// ===========================================================================
// Python value surface: _Element / HtmlElement objects
// ===========================================================================

pub fn is_lx_element(v: &Value) -> bool {
    matches!(
        v,
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Opaque
                && (o.name == "lxml.etree._Element" || o.name == "lxml.html.HtmlElement")
    )
}

/// Element identity: the shared doc handle + arena index ride the object's
/// attrs (`<shared>` Namespace holder with a direct Rc pointer parked via the
/// attrs dict is NOT possible — Value has no Rc<T> carrier — so the doc lives
/// in a thread-local registry keyed by a fresh id; elements carry that id).
struct DocRegistry {
    docs: Vec<SharedDoc>,
}

thread_local! {
    static DOCS: RefCell<DocRegistry> = RefCell::new(DocRegistry { docs: Vec::new() });
}

fn register_doc(doc: LxDoc) -> usize {
    DOCS.with(|c| {
        let mut r = c.borrow_mut();
        r.docs.push(Rc::new(RefCell::new(doc)));
        r.docs.len() - 1
    })
}

/// Look up an element's (shared doc, arena index, html flag).
fn elem_parts(v: &Value) -> Option<(SharedDoc, usize, bool)> {
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Opaque
            && (o.name == "lxml.etree._Element" || o.name == "lxml.html.HtmlElement")
        {
            let attrs = o.attrs.borrow();
            let doc_id = attrs
                .get(&s("<doc_id>"))
                .and_then(|v| v.as_int())?
                .to_i64()? as usize;
            let idx = attrs.get(&s("<idx>")).and_then(|v| v.as_int())?.to_i64()? as usize;
            let html = o.name == "lxml.html.HtmlElement";
            let doc = DOCS.with(|c| c.borrow().docs.get(doc_id).cloned())?;
            return Some((doc, idx, html));
        }
    }
    None
}

fn make_element(doc_id: usize, idx: usize, html: bool) -> Value {
    let name = if html {
        "lxml.html.HtmlElement"
    } else {
        "lxml.etree._Element"
    };
    let o = PyObject::new(ap_obj::ObjKind::Opaque, name);
    {
        let mut a = o.attrs.borrow_mut();
        let _ = a.set(s("<doc_id>"), i(doc_id as i64));
        let _ = a.set(s("<idx>"), i(idx as i64));
    }
    Value::Obj(o)
}

/// Human repr: `<Element tag at 0x...>` — address is our PyObject pointer.
fn elem_repr(o: &Gc<PyObject>, doc: &LxDoc, idx: usize) -> String {
    let cls = if o.name == "lxml.html.HtmlElement" {
        "HtmlElement"
    } else {
        "Element"
    };
    let addr = std::rc::Rc::as_ptr(o) as usize;
    format!("<{} {} at {:#x}>", cls, doc.get(idx).tag, addr)
}

fn doc_of(v: &Value) -> Option<(SharedDoc, usize, bool)> {
    elem_parts(v)
}

/// attrib dict → Value (snapshot; mutations via el.set/attrib mapping helpers).
fn attrib_value(doc: &LxDoc, idx: usize) -> Value {
    let d = PyDict::new();
    for (k, v) in &doc.get(idx).attrib {
        let _ = d.set(s(k.clone()), s(v.clone()));
    }
    Value::Dict(d)
}

fn set_attrib_key(doc: &mut LxDoc, idx: usize, key: &str, val: &str) {
    let node = doc.get_mut(idx);
    if let Some(pair) = node.attrib.iter_mut().find(|(k, _)| k == key) {
        pair.1 = val.to_string();
    } else {
        node.attrib.push((key.to_string(), val.to_string()));
    }
}

fn del_attrib_key(doc: &mut LxDoc, idx: usize, key: &str) -> bool {
    let node = doc.get_mut(idx);
    if let Some(pos) = node.attrib.iter().position(|(k, _)| k == key) {
        node.attrib.remove(pos);
        true
    } else {
        false
    }
}

/// Parse `text`/`body` with mode selection; returns (doc_id, root_idx, html).
fn parse_to_doc(text: &str, html_mode: bool) -> (usize, usize) {
    let doc = parse_html(text, html_mode);
    let root = doc.root;
    (register_doc(doc), root)
}

fn xvalue_to_python(doc: &SharedDoc, doc_id: usize, html: bool, v: XValue) -> Value {
    match v {
        XValue::Nodes(nodes) => Value::List(PyList::from_vec(
            nodes
                .into_iter()
                .map(|ix| make_element(doc_id, ix, html))
                .collect(),
        )),
        XValue::Attrs(vals) => Value::List(PyList::from_vec(vals.into_iter().map(s).collect())),
        XValue::Texts(vals) => Value::List(PyList::from_vec(vals.into_iter().map(s).collect())),
        XValue::Str(x) => s(x),
        XValue::Float(x) => f(x),
        XValue::Bool(b) => Value::Bool(b),
    }
    .tap(|_| {
        let _ = doc;
    })
}

trait Tap: Sized {
    fn tap<F: FnOnce(&Self)>(self, fcb: F) -> Self {
        fcb(&self);
        self
    }
}
impl<T> Tap for T {}

/// Run one xpath on an element receiver; Err = unsupported (XPathEvalError).
fn run_xpath(recv_doc: &SharedDoc, idx: usize, doc_id: usize, html: bool, expr: &str) -> BResult {
    let doc_borrow = recv_doc.borrow();
    match xpath_eval(&doc_borrow, idx, expr) {
        Some(v) => Ok(xvalue_to_python(recv_doc, doc_id, html, v)),
        None => Err(raise("lxml.etree.XPathEvalError", "Invalid expression")),
    }
}

/// Serialize an element (tostring).
fn tostring(elem: &Value, method: &str, with_tail: bool) -> BResult {
    let Some((doc, idx, _)) = doc_of(elem) else {
        return Err(raise("TypeError", "tostring() argument must be an Element"));
    };
    let d = doc.borrow();
    let mut out = String::new();
    serialize(&d, idx, method, with_tail, &mut out);
    if method == "text" {
        let mut t = String::new();
        collect_string_content(&d, idx, &mut t);
        return Ok(s(t));
    }
    Ok(s(out))
}

// ===========================================================================
// Module builders + dispatch
// ===========================================================================

pub fn build_lxml_package(interp: &crate::frame::Interpreter) -> Value {
    // The `lxml` parent package: a real module whose ns holds the submodules
    // (CPython: lxml/__init__.py). `from lxml import etree` resolves `etree`
    // as an attribute of this module.
    let (m, ns) = crate::builtins_modules::new_module("lxml");
    let etree_v = build_etree(interp);
    let html_v = build_html(interp);
    let _ = ns.set(s("etree"), etree_v);
    let _ = ns.set(s("html"), html_v);
    // lxml exposes __version__ at the package level too.
    let _ = ns.set(s("__version__"), s("6.1.3"));
    crate::builtins_modules::finish(m, ns)
}

pub fn build_etree(_i: &crate::frame::Interpreter) -> Value {
    // The module object's name must be lxml.etree (CPython: lxml is a package;
    // `from lxml import etree` imports the submodule).
    let (m2, ns) = crate::builtins_modules::new_module("lxml.etree");
    for n in [
        "fromstring",
        "XML",
        "tostring",
        "Element",
        "SubElement",
        "Comment",
    ] {
        crate::builtins_modules::fn1(&ns, "lxml.etree", n);
    }
    for n in ["cleanup_namespaces", "iselement", "strip_text"] {
        crate::builtins_modules::fn1(&ns, "lxml.etree", n);
    }
    // iterparse: fail-loud marker (streaming surface out of the v1 subset).
    crate::builtins_modules::fn1(&ns, "lxml.etree", "iterparse");
    // FunctionNamespace(registry): parsel/xpathfuncs registers the has-class
    // extension through it. v0: an inert registry object — lookups return a
    // dummy callable (the has-class xpath form is evaluated natively when
    // recognized; unregistered names never appear in T1 spider xpaths).
    let _ = ns.set(
        s("FunctionNamespace"),
        crate::builtins_modules::marker("lxml.etree.FunctionNamespace"),
    );
    // Parser class objects (constructed + passed to fromstring; construction
    // returns an opaque carrying recover/encoding — our parser is always
    // lenient for HTML, strict-with-recover flag ignored for XML).
    // 02.08-c: class "constructors" are fn1 CALLABLE markers (dotted
    // lxml.etree.<name>) so construction dispatches through call_native —
    // Type-kind objects would route to call_type_ctor and die with NameError
    // (no interned type of that name).
    for cls in [
        "XMLParser",
        "HTMLParser",
        "ETCompatXMLParser",
        "Element",
        "_Element",
        "XPath",
    ] {
        crate::builtins_modules::fn1(&ns, "lxml.etree", cls);
    }
    // Exception classes.
    for exc in [
        "LxmlError",
        "LxmlSyntaxError",
        "XMLSyntaxError",
        "ParseError",
        "XPathError",
        "XPathEvalError",
        "XPathSyntaxError",
    ] {
        let ev = if let Some(interp) = crate::builtins_modules::current_interp() {
            crate::classes::exc_class_value(&interp, exc)
                .unwrap_or_else(|| Value::Obj(PyObject::new(ap_obj::ObjKind::Type, exc)))
        } else {
            Value::Obj(PyObject::new(ap_obj::ObjKind::Type, exc))
        };
        let _ = ns.set(s(exc), ev);
    }
    // ErrorTypes: an IntEnum-like namespace (parsel: etree.ErrorTypes.ERR_*)
    // — plain Type marker whose attribute reads resolve in the call layer.
    let et = PyObject::new(ap_obj::ObjKind::Namespace, "ErrorTypes");
    {
        let mut a = et.attrs.borrow_mut();
        ERROR_TYPES_FOR_EACH(|name, code| {
            let member = PyObject::new(ap_obj::ObjKind::Opaque, &format!("ErrorTypes.{name}"));
            *member.repr_text.borrow_mut() = Some(format!("<ErrorTypes.{name}: {code}>"));
            let _ = member.attrs.borrow_mut().set(s("<int_value>"), i(code));
            let _ = a.set(s(name), Value::Obj(member));
        });
    }
    let _ = ns.set(s("ErrorTypes"), Value::Obj(et));
    // Version pins (real lxml 6.1.3 / libxml2 2.14.6 — the versions the
    // reference venv resolved; scrapy logs these).
    let _ = ns.set(s("LIBXML_VERSION"), tuple(vec![i(2), i(14), i(6)]));
    let _ = ns.set(s("LXML_VERSION"), tuple(vec![i(6), i(1), i(3), i(0)]));
    let _ = ns.set(s("LIBXML_COMPILED_VERSION"), tuple(vec![i(2), i(14), i(6)]));
    let _ = ns.set(s("LIBXSLT_VERSION"), tuple(vec![i(1), i(1), i(43)]));
    ns_const(&ns, "__version__", s("6.1.3"));
    crate::builtins_modules::finish(m2, ns)
}

fn ERROR_TYPES_FOR_EACH(mut cb: impl FnMut(&'static str, i64)) {
    for (name, code) in [
        ("ERR_OK", 0),
        ("ERR_INTERNAL_ERROR", 1),
        ("ERR_NO_MEMORY", 2),
        ("ERR_DOCUMENT_START", 3),
        ("ERR_DOCUMENT_EMPTY", 4),
        ("ERR_DOCUMENT_END", 5),
        ("ERR_INVALID_HEX_CHARREF", 6),
        ("ERR_INVALID_DEC_CHARREF", 7),
        ("ERR_INVALID_CHAR", 9),
        ("ERR_INVALID_URI", 10),
        ("ERR_FRAGMENT", 11),
        ("ERR_ATTRIBUTE_WITHOUT_VALUE", 12),
        ("ERR_TAG_NOT_FINISHED", 76),
        ("ERR_INVALID_ENCODING", 81),
    ] {
        cb(name, code);
    }
}

pub fn build_html(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = crate::builtins_modules::new_module("lxml.html");
    for n in ["fromstring", "document_fromstring", "fragment_fromstring"] {
        crate::builtins_modules::fn1(&ns, "lxml.html", n);
    }
    // Constructible via the native path: Type-kind ctor chokes on kwargs
    // (recover=True); a BuiltinFn marker routes call_native -> call_lxml ctor.
    let o = PyObject::new(ap_obj::ObjKind::BuiltinFn, "lxml.html.HTMLParser");
    let _ = ns.set(s("HTMLParser"), Value::Obj(o));
    let o = PyObject::new(ap_obj::ObjKind::Type, "HtmlElement");
    let _ = ns.set(s("HtmlElement"), Value::Obj(o));
    for cls in [
        "FormElement",
        "InputElement",
        "SelectElement",
        "TextareaElement",
    ] {
        let o = PyObject::new(ap_obj::ObjKind::Type, cls);
        let _ = ns.set(s(cls), Value::Obj(o));
    }
    crate::builtins_modules::finish(m, ns)
}

fn ns_const(ns: &Gc<ap_obj::dict::PyDict>, k: &str, v: Value) {
    let _ = ns.set(s(k), v);
}

fn tuple(v: Vec<Value>) -> Value {
    Value::Tuple(Gc::new(Tuple(v)))
}

/// str/bytes arg helper.
fn str_arg(v: &Value, who: &str) -> Result<String, Flow> {
    match v {
        Value::Str(x) => Ok(x.as_str().to_string()),
        Value::Bytes(b) => Ok(String::from_utf8_lossy(b).into_owned()),
        other => Err(raise(
            "TypeError",
            format!(
                "{who} argument must be str or bytes, not {}",
                other.type_name()
            ),
        )),
    }
}

pub fn call_lxml(fname: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    // Dotted markers arrive as "lxml.etree.fromstring" (mod="lxml",
    // fname="etree.fromstring") or "lxml.html.fromstring"; entry-point
    // names are re-based onto the bare form so both module surfaces and
    // the parent-package surface share one dispatch table.
    let fname = match fname {
        "etree.fromstring" => "fromstring",
        "etree.XML" => "XML",
        "etree.tostring" => "tostring",
        "etree.Element" => "Element",
        "etree.SubElement" => "SubElement",
        "etree.Comment" => "Comment",
        "etree.FunctionNamespace" => "FunctionNamespace",
        "etree.cleanup_namespaces" => "cleanup_namespaces",
        "etree.iselement" => "iselement",
        "etree.strip_text" => "strip_text",
        "etree.iterparse" => "iterparse",
        "etree.XMLParser" => "XMLParser",
        "etree.HTMLParser" => "HTMLParser",
        "etree.ETCompatXMLParser" => "ETCompatXMLParser",
        "etree.XPath" => "XPath",
        "etree._Element" => "_Element",
        "html.HTMLParser" => "HTMLParser",
        "html.fromstring" => "html.fromstring",
        "html.document_fromstring" => "html.document_fromstring",
        "html.fragment_fromstring" => "html.fragment_fromstring",
        other => other,
    };
    let r: BResult = (|| match fname {
        "FunctionNamespace" => {
            let o = PyObject::new(ap_obj::ObjKind::Opaque, "lxml.FunctionNamespace");
            Ok(Value::Obj(o))
        }
        // -------------------------------------------------------------------
        // lxml.etree.fromstring / XML — bytes or str; parser= ignored (our
        // parser is fixed); base_url parked on the doc for .base_url.
        // HTML mode when the parser arg is an HTMLParser; otherwise strict
        // XML (empty/malformed input raises lxml.etree.XMLSyntaxError).
        // -------------------------------------------------------------------
        "fromstring" | "XML" => {
            let raw = args
                .first()
                .ok_or_else(|| raise("TypeError", "fromstring() missing required argument"))?;
            let text = str_arg(raw, "fromstring()")?;
            let html_mode = args.iter().skip(1).any(|v| match v {
                Value::Obj(o) if o.kind == ap_obj::ObjKind::Namespace => {
                    matches!(o.name.as_str(), "HTMLParser" | "SafeHTMLParser")
                }
                _ => false,
            });
            if !html_mode && text.trim().is_empty() {
                return Err(raise(
                    "lxml.etree.XMLSyntaxError",
                    "Document is empty, line 1, column 1 (<string>, line 1)",
                ));
            }
            if !html_mode && !text.trim_start().starts_with('<') {
                return Err(raise(
                    "lxml.etree.XMLSyntaxError",
                    &format!(
                        "Start tag expected, '<' not found, line 1, column 1 (<string>, line 1)"
                    ),
                ));
            }
            let (doc_id, root) = parse_to_doc(&text, html_mode);
            if !html_mode {
                // Strict XML: mismatched close tags raise.
                if let Some(bad) = xml_wellformed_error(&text) {
                    return Err(raise("lxml.etree.XMLSyntaxError", &bad));
                }
            }
            let base_url = kw(kwargs, "base_url");
            if let Some(Value::Str(b)) = base_url {
                DOCS.with(|c| {
                    if let Some(d) = c.borrow().docs.get(doc_id) {
                        d.borrow_mut().base_url = Some(b.as_str().to_string());
                    }
                });
            }
            let o = make_element(doc_id, root, html_mode);
            // fromstring on an HTMLParser returns the html element; XML mode
            // returns the root element (parity with libxml2).
            Ok(o)
        }
        "tostring" => {
            let elem = args
                .first()
                .ok_or_else(|| raise("TypeError", "tostring() missing required argument"))?;
            let method = kw(kwargs, "method")
                .map(|v| match v {
                    Value::Str(x) => x.as_str().to_string(),
                    _ => "xml".to_string(),
                })
                .unwrap_or_else(|| {
                    // Default: bytes (xml method); the unicode switch comes
                    // from encoding='unicode'.
                    "xml".to_string()
                });
            let with_tail = kw(kwargs, "with_tail")
                .map(|v| v.is_truthy())
                .unwrap_or(true);
            match kw(kwargs, "encoding") {
                Some(Value::Str(x)) if x.as_str() == "unicode" => {
                    tostring(elem, &method, with_tail)
                }
                _ => {
                    // bytes return: utf-8 encode the unicode form.
                    let v = tostring(elem, &method, with_tail)?;
                    match v {
                        Value::Str(x) => Ok(Value::Bytes(Gc::new(x.as_str().as_bytes().to_vec()))),
                        other => Ok(other),
                    }
                }
            }
        }
        // 02.08-c: parser/xpath constructors — inert opaque objects; the
        // parser arg to fromstring is ignored (single lenient parser).
        "XMLParser" | "HTMLParser" | "ETCompatXMLParser" => {
            let o = PyObject::new(ap_obj::ObjKind::Opaque, &format!("lxml.etree.{}", fname));
            Ok(Value::Obj(o))
        }
        "XPath" => {
            let query = match args.first() {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => return Err(raise("TypeError", "XPath() argument must be str")),
            };
            Ok(make_xpath_object(query, kw(kwargs, "namespaces").cloned()))
        }
        "_Element" => {
            // Class-object read (isinstance targets): opaque token.
            let o = PyObject::new(ap_obj::ObjKind::Opaque, "lxml.etree._Element.cls");
            Ok(Value::Obj(o))
        }
        "Element" => {
            let tag = match args.first() {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => {
                    return Err(raise(
                        "TypeError",
                        "Element() argument 1 must be str, not NoneType",
                    ))
                }
            };
            let mut doc = LxDoc {
                nodes: vec![LxNode::new("#document", false)],
                root: 0,
                base_url: None,
            };
            let idx = doc.nodes.len();
            doc.nodes.push(LxNode::new(&tag, false));
            let mut node = doc.nodes.pop().unwrap();
            node.parent = None;
            // attrib from kwargs or attrib= dict.
            let mut pairs: Vec<(String, String)> = Vec::new();
            if let Some(kwv) = kw(kwargs, "attrib") {
                if let Value::Dict(d) = kwv {
                    for (k, v) in d.items() {
                        if let (Value::Str(kk), Value::Str(vv)) = (&k, &v) {
                            pairs.push((kk.as_str().to_string(), vv.as_str().to_string()));
                        }
                    }
                }
            }
            for (k, v) in kwargs {
                if k == "attrib" {
                    continue;
                }
                if let Value::Str(vv) = v {
                    pairs.push((k.clone(), vv.as_str().to_string()));
                }
            }
            node.attrib = pairs;
            doc.nodes.push(node);
            doc.root = idx;
            let doc_id = register_doc(doc);
            Ok(make_element(doc_id, idx, false))
        }
        "SubElement" => {
            let (parent_val, tag) = (
                args.first().cloned().unwrap_or(Value::None),
                args.get(1)
                    .map(|v| str_arg(v, "SubElement()"))
                    .transpose()?,
            );
            let Some(tag) = tag else {
                return Err(raise("TypeError", "SubElement() missing tag argument"));
            };
            let Some((doc, pidx, _)) = doc_of(&parent_val) else {
                return Err(raise("TypeError", "SubElement() argument must be _Element"));
            };
            let mut d = doc.borrow_mut();
            let idx = d.nodes.len();
            let mut node = LxNode::new(&tag, false);
            node.parent = Some(pidx);
            let mut pairs: Vec<(String, String)> = Vec::new();
            if let Some(kwv) = kw(kwargs, "attrib") {
                if let Value::Dict(dd) = kwv {
                    for (k, v) in dd.items() {
                        if let (Value::Str(kk), Value::Str(vv)) = (&k, &v) {
                            pairs.push((kk.as_str().to_string(), vv.as_str().to_string()));
                        }
                    }
                }
            }
            for (k, v) in kwargs {
                if k == "attrib" {
                    continue;
                }
                if let Value::Str(vv) = v {
                    pairs.push((k.clone(), vv.as_str().to_string()));
                }
            }
            node.attrib = pairs;
            d.nodes.push(node);
            d.nodes[pidx].children.push(idx);
            drop(d);
            let doc_id = doc_id_of(&parent_val);
            Ok(make_element(doc_id, idx, false))
        }
        "Comment" => {
            // Comment factory: our tokenizer drops comments, so a Comment
            // node is a standalone empty element with a callable tag marker.
            let mut doc = LxDoc {
                nodes: vec![LxNode::new("#document", false)],
                root: 0,
                base_url: None,
            };
            let idx = 1;
            doc.nodes.push(LxNode::new("#comment", false));
            doc.root = 0;
            let doc_id = register_doc(doc);
            Ok(make_element(doc_id, idx, false))
        }
        "iselement" => Ok(Value::Bool(
            args.first().map(is_lx_element).unwrap_or(false),
        )),
        "cleanup_namespaces" => {
            // Our serializer never emits unused ns declarations: no-op.
            Ok(Value::None)
        }
        "strip_text" => Err(raise(
            "NotImplementedError",
            "lxml.etree.strip_text is not part of the native lxml shim surface",
        )),
        "iterparse" => Err(raise(
            "NotImplementedError",
            "lxml.etree.iterparse streaming is not part of the native lxml shim surface",
        )),
        // -------------------------------------------------------------------
        // lxml.html entry points.
        // -------------------------------------------------------------------
        "html.fromstring" => {
            let raw = args
                .first()
                .ok_or_else(|| raise("TypeError", "fromstring() missing required argument"))?;
            let text = str_arg(raw, "fromstring()")?;
            let (doc_id, root) = parse_to_doc(&text, true);
            let base_url = kw(kwargs, "base_url");
            if let Some(Value::Str(b)) = base_url {
                DOCS.with(|c| {
                    if let Some(d) = c.borrow().docs.get(doc_id) {
                        d.borrow_mut().base_url = Some(b.as_str().to_string());
                    }
                });
            }
            // lxml fragment rule: a single top-level element returns it; a
            // bare text wraps in span; multiple top nodes wrap in span. Our
            // builder already wrapped everything in html>body, so undo the
            // wrapper when the body holds exactly one element (the fragment
            // case) and no html/body tags appeared in the source.
            let looks_full_doc = {
                let t = text.trim_start();
                t.to_lowercase().starts_with("<!doctype") || t.to_lowercase().starts_with("<html")
            };
            let d = DOCS.with(|c| c.borrow().docs.get(doc_id).cloned()).unwrap();
            let root_idx = if looks_full_doc {
                root
            } else {
                // lxml fragment rule: 1 top-level element (no stray text)
                // returns it; >1 elements or body-level stray text wraps
                // ALL body content in a <span> and returns the span.
                let (pick, needs_span) = {
                    let dd = d.borrow();
                    let body = dd
                        .get(root)
                        .children
                        .iter()
                        .copied()
                        .find(|&c| dd.get(c).tag == "body")
                        .unwrap_or(root);
                    let kids = dd.get(body).children.clone();
                    let stray = dd.get(body).text.is_some()
                        || kids.iter().any(|&k| dd.get(k).tail.is_some());
                    if kids.len() == 1 && !stray {
                        (Some(kids[0]), false)
                    } else {
                        (None, true)
                    }
                };
                if let Some(p) = pick {
                    p
                } else if needs_span {
                    let mut dd = d.borrow_mut();
                    let body = dd
                        .get(root)
                        .children
                        .iter()
                        .copied()
                        .find(|&c| dd.get(c).tag == "body")
                        .unwrap_or(root);
                    let kids = dd.get(body).children.clone();
                    if kids.is_empty() {
                        root
                    } else {
                        let span_ix = dd.nodes.len();
                        let mut span = LxNode::new("span", true);
                        span.parent = Some(body);
                        span.text = dd.get_mut(body).text.take();
                        dd.nodes.push(span);
                        for &k in &kids {
                            dd.get_mut(k).parent = Some(span_ix);
                            dd.get_mut(span_ix).children.push(k);
                        }
                        dd.get_mut(body).children.clear();
                        dd.get_mut(body).children.push(span_ix);
                        span_ix
                    }
                } else {
                    root
                }
            };
            Ok(make_element(doc_id, root_idx, true))
        }
        "html.document_fromstring" => {
            let raw = args.first().cloned().unwrap_or(Value::None);
            let text = str_arg(&raw, "document_fromstring()")?;
            let (doc_id, root) = parse_to_doc(&text, true);
            Ok(make_element(doc_id, root, true))
        }
        "html.fragment_fromstring" => {
            let raw = args.first().cloned().unwrap_or(Value::None);
            let text = str_arg(&raw, "fragment_fromstring()")?;
            let (doc_id, root) = parse_to_doc(&text, true);
            let d = DOCS.with(|c| c.borrow().docs.get(doc_id).cloned()).unwrap();
            let dd = d.borrow();
            let top = dd.get(root).children.clone();
            let root_idx = match top.len() {
                0 => root,
                1 => top[0],
                _ => root,
            };
            drop(dd);
            Ok(make_element(doc_id, root_idx, true))
        }
        _ => Err(raise(
            "AttributeError",
            format!("module 'lxml' has no attribute '{fname}'"),
        )),
    })();
    Some(r)
}

fn doc_id_of(v: &Value) -> usize {
    elem_parts(v)
        .map(|(d, _, _)| {
            DOCS.with(|c| {
                c.borrow()
                    .docs
                    .iter()
                    .position(|x| Rc::ptr_eq(x, &d))
                    .unwrap_or(0)
            })
        })
        .unwrap_or(0)
}

fn kw<'a>(kwargs: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    kwargs.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

/// Strict-XML well-formedness check (tag balance only — the common
/// XMLSyntaxError class; message carries the libxml2 phrasing).
fn xml_wellformed_error(text: &str) -> Option<String> {
    let mut stack: Vec<(String, usize)> = Vec::new();
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            match rest.find("-->") {
                Some(e) => i += e + 3,
                None => i = n,
            }
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            match rest.find("]]>") {
                Some(e) => i += e + 3,
                None => i = n,
            }
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            match rest.find('>') {
                Some(e) => i += e + 1,
                None => i = n,
            }
            continue;
        }
        if rest.starts_with("</") {
            let close = rest[2..].find('>').map(|p| 2 + p).unwrap_or(rest.len());
            let name = rest[2..close].trim().to_string();
            match stack.pop() {
                Some((open, _)) if open == name => {}
                Some((open, col)) => {
                    return Some(format!(
                        "Opening and ending tag mismatch: {open} line {col} and {name}, line 1, column {} (<string>, line 1)",
                        i + 1
                    ));
                }
                None => {
                    return Some(format!(
                        "Opening and ending tag mismatch: (none) and {name}, line 1, column {} (<string>, line 1)",
                        i + 1
                    ));
                }
            }
            i += close + 1;
            continue;
        }
        // Start tag: find its '>' honoring quotes.
        let mut j = 1;
        let mut q: Option<u8> = None;
        while j < rest.len() {
            let c = rest.as_bytes()[j];
            match q {
                Some(qc) => {
                    if c == qc {
                        q = None;
                    }
                }
                None => {
                    if c == b'"' || c == b'\'' {
                        q = Some(c);
                    } else if c == b'>' {
                        break;
                    }
                }
            }
            j += 1;
        }
        let inner = &rest[1..j.min(rest.len())];
        let name: String = inner
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_string();
        if !name.is_empty() && !inner.ends_with('/') {
            stack.push((name, 1));
        }
        i += j + 1;
    }
    if let Some((open, _)) = stack.pop() {
        return Some(format!(
            "Premature end of data in tag {open} line 1, line 1, column {n} (<string>, line 1)"
        ));
    }
    None
}

// ===========================================================================
// Attribute surface (native_attr) for element objects
// ===========================================================================

/// attr read on an lxml element object. Returns Some(Value) when handled.
pub fn elem_attr(obj: &Value, name: &str) -> Option<Value> {
    let (doc, idx, html) = elem_parts(obj)?;
    if let Value::Obj(o) = obj {
        // Plain property reads first.
        match name {
            "tag" => {
                return Some(s(doc.borrow().get(idx).tag.clone()));
            }
            "text" => {
                return Some(
                    doc.borrow()
                        .get(idx)
                        .text
                        .clone()
                        .map(s)
                        .unwrap_or(Value::None),
                );
            }
            "tail" => {
                return Some(
                    doc.borrow()
                        .get(idx)
                        .tail
                        .clone()
                        .map(s)
                        .unwrap_or(Value::None),
                );
            }
            "attrib" => {
                // LIVE mapping: mutations via el.set/attrib-writes through the
                // method markers; the returned dict is a snapshot (parsel only
                // reads .attrib / dict(el.attrib) / iterate).
                let d = doc.borrow();
                return Some(attrib_value(&d, idx));
            }
            "sourceline" | "sourcepos" | "prefix" | "base" => return Some(Value::None),
            "nsmap" => return Some(Value::Dict(ap_obj::dict::PyDict::new())),
            "base_url" if html => {
                return Some(doc.borrow().base_url.clone().map(s).unwrap_or(Value::None));
            }
            "classes" if html => {
                // lxml.html.Classes (a live list); a plain list snapshot.
                let d = doc.borrow();
                let cls = attr_value(d.get(idx), "class").unwrap_or("");
                let items: Vec<Value> = cls.split_whitespace().map(|c| s(c.to_string())).collect();
                return Some(Value::List(PyList::from_vec(items)));
            }
            _ => {}
        }
        // HtmlElement structural reads.
        if html && matches!(name, "body" | "head") {
            let d = doc.borrow();
            let root = d.root;
            if let Some(found) = d.get(root).children.iter().copied().find(|&c| {
                d.get(c).tag == name || d.get(c).children.iter().any(|&cc| d.get(cc).tag == name)
            }) {
                let target = if d.get(found).tag == name {
                    found
                } else {
                    d.get(found)
                        .children
                        .iter()
                        .copied()
                        .find(|&c| d.get(c).tag == name)
                        .unwrap()
                };
                let doc_id = DOCS.with(|c| {
                    c.borrow()
                        .docs
                        .iter()
                        .position(|x| Rc::ptr_eq(x, &doc))
                        .unwrap_or(0)
                });
                return Some(make_element(doc_id, target, true));
            }
            return Some(Value::None);
        }
        // Method markers -> BoundMethod over the element.
        let methods: &[&str] = if html {
            &[
                "xpath",
                "get",
                "set",
                "getparent",
                "getprevious",
                "getnext",
                "iter",
                "find",
                "findall",
                "findtext",
                "iterfind",
                "getchildren",
                "getiterator",
                "itertext",
                "clear",
                "cssselect",
                "text_content",
                "drop_tree",
                "drop_tag",
                "make_links_absolute",
                "makeelement",
                "append",
                "index",
            ]
        } else {
            &[
                "xpath",
                "get",
                "set",
                "getparent",
                "getprevious",
                "getnext",
                "iter",
                "find",
                "findall",
                "findtext",
                "iterfind",
                "getchildren",
                "getiterator",
                "itertext",
                "clear",
                "cssselect",
                "text_content",
                "makeelement",
                "append",
                "index",
            ]
        };
        if methods.contains(&name) {
            let full = format!("__ap_lxml__.{name}");
            let bm = PyObject::with_self(ap_obj::ObjKind::BoundMethod, full.clone(), obj.clone());
            *bm.func.borrow_mut() = Some(crate::builtins_modules::marker(&full));
            return Some(Value::Obj(bm));
        }
    }
    None
}

/// Bound-method dispatch on element objects (marker `__ap_lxml__.<name>`,
/// receiver rides args[0]).
pub fn call_elem(fname: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let recv = args.first()?.clone();
    let rest = &args[1..];
    let (doc, idx, html) = elem_parts(&recv)?;
    let doc_id = doc_id_of(&recv);
    let r: BResult = (|| match fname {
        "FunctionNamespace" => {
            let o = PyObject::new(ap_obj::ObjKind::Opaque, "lxml.FunctionNamespace");
            Ok(Value::Obj(o))
        }
        // 02.08-c: len(r) — child element count.
        "__len__" => {
            let d = doc.borrow();
            let n = d.nodes[idx].children.len();
            drop(d);
            Ok(Value::Int(ap_obj::Int::from_i64(n as i64)))
        }
        // 02.08-c: r[i] child indexing (negatives count from the end).
        "__getitem__" => {
            let key: i64 = match rest.first() {
                Some(Value::Int(iv)) => iv.to_i64().unwrap_or(0),
                Some(Value::Bool(b)) => {
                    if *b {
                        1
                    } else {
                        0
                    }
                }
                _ => match crate::builtins::py_str_of(
                    rest.first()
                        .ok_or_else(|| raise("TypeError", "element index expected"))?,
                ) {
                    Ok(s) => s.as_str().parse::<i64>().unwrap_or(0),
                    Err(_) => return Err(raise("TypeError", "element indices must be integers")),
                },
            };
            let d = doc.borrow();
            let kids: Vec<usize> = d.nodes[idx].children.clone();
            let n = kids.len() as i64;
            let pick = if key < 0 { n + key } else { key };
            if pick < 0 || pick >= n {
                return Err(raise("IndexError", "element index out of range"));
            }
            let child = kids[pick as usize];
            let html_mode = html;
            drop(d);
            Ok(make_element(doc_id, child, html_mode))
        }
        "xpath" => {
            let expr = match rest.first() {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => return Err(raise("TypeError", "xpath() argument must be str")),
            };
            run_xpath(&doc, idx, doc_id, html, &expr)
        }
        "get" => {
            let key = match rest.first() {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => return Err(raise("TypeError", "get() argument must be str")),
            };
            let dflt = rest.get(1).cloned().unwrap_or(Value::None);
            let d = doc.borrow();
            Ok(attr_value(d.get(idx), &key).map(s).unwrap_or(dflt))
        }
        "set" => {
            let key = match rest.first() {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => return Err(raise("TypeError", "set() argument must be str")),
            };
            let val = match rest.get(1) {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => return Err(raise("TypeError", "set() value must be str")),
            };
            set_attrib_key(&mut doc.borrow_mut(), idx, &key, &val);
            Ok(Value::None)
        }
        "getparent" => {
            let d = doc.borrow();
            match d.get(idx).parent {
                Some(p) if p != 0 => Ok(make_element(doc_id, p, html)),
                _ => Ok(Value::None),
            }
        }
        "getprevious" => {
            let d = doc.borrow();
            let parent = match d.get(idx).parent {
                Some(p) => p,
                None => return Ok(Value::None),
            };
            let sibs = &d.get(parent).children;
            let pos = sibs.iter().position(|&c| c == idx);
            match pos.and_then(|p| p.checked_sub(1)) {
                Some(p) => Ok(make_element(doc_id, sibs[p], html)),
                None => Ok(Value::None),
            }
        }
        "getnext" => {
            let d = doc.borrow();
            let parent = match d.get(idx).parent {
                Some(p) => p,
                None => return Ok(Value::None),
            };
            let sibs = &d.get(parent).children;
            match sibs.iter().position(|&c| c == idx) {
                Some(p) if p + 1 < sibs.len() => Ok(make_element(doc_id, sibs[p + 1], html)),
                _ => Ok(Value::None),
            }
        }
        "iter" => {
            let tag = rest
                .first()
                .map(|v| match v {
                    Value::Str(x) => x.as_str().to_string(),
                    _ => "*".to_string(),
                })
                .unwrap_or_else(|| "*".to_string());
            let tag = if tag == "*" { "*".to_string() } else { tag };
            let d = doc.borrow();
            let mut out = Vec::new();
            if tag == "*" || d.get(idx).tag == tag {
                out.push(idx);
            }
            collect_descendants(&d, idx, &tag, &mut out);
            Ok(Value::List(PyList::from_vec(
                out.into_iter()
                    .map(|ix| make_element(doc_id, ix, html))
                    .collect(),
            )))
        }
        "find" | "findall" | "iterfind" | "findtext" => {
            // ElementPath subset: relative child paths ("tag", "tag/sub",
            // ".//tag", "./tag", "tag1/tag2"). Absolute '//' raises
            // SyntaxError like lxml.
            let path = match rest.first() {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => return Err(raise("TypeError", "path argument must be str")),
            };
            if path.starts_with("//") {
                return Err(raise("SyntaxError", "cannot use absolute path on element"));
            }
            // CPython ElementPath: findall('a') returns ALL matching
            // children — the ./tag form must not stop at the first match.
            // Our xpath ./tag returns nodes correctly; the earlier bug was
            // `r.find('b')` returning None because xpath_eval's ./b search
            // compared against is_text nodes (none here) — kept for parity.
            let expr = if path.starts_with("./") || path.starts_with(".//") {
                path.clone()
            } else {
                format!("./{path}")
            };
            match xpath_eval(&doc.borrow(), idx, &expr) {
                Some(XValue::Nodes(nodes)) => {
                    if fname == "findtext" {
                        let mut out = String::new();
                        if let Some(&first) = nodes.first() {
                            collect_string_content(&doc.borrow(), first, &mut out);
                        }
                        return Ok(s(out));
                    }
                    if fname == "find" {
                        return Ok(nodes
                            .first()
                            .map(|&ix| make_element(doc_id, ix, html))
                            .unwrap_or(Value::None));
                    }
                    Ok(Value::List(PyList::from_vec(
                        nodes
                            .into_iter()
                            .map(|ix| make_element(doc_id, ix, html))
                            .collect(),
                    )))
                }
                Some(_) => Err(raise("TypeError", "find path must select elements")),
                None => Err(raise("SyntaxError", "invalid path")),
            }
        }
        "getchildren" | "getiterator" => {
            let d = doc.borrow();
            let items: Vec<Value> = d
                .get(idx)
                .children
                .iter()
                .map(|&c| make_element(doc_id, c, html))
                .collect();
            Ok(Value::List(PyList::from_vec(items)))
        }
        "itertext" => {
            let d = doc.borrow();
            let mut out = String::new();
            collect_string_content(&d, idx, &mut out);
            // lxml yields per-text-node strings; flatten to one list of the
            // text + each descendant text/tail piece.
            let mut pieces: Vec<Value> = Vec::new();
            if let Some(t) = &d.get(idx).text {
                pieces.push(s(t.clone()));
            }
            fn walk(d: &LxDoc, ix: usize, pieces: &mut Vec<Value>) {
                for &c in &d.get(ix).children {
                    if let Some(t) = &d.get(c).text {
                        pieces.push(s(t.clone()));
                    }
                    walk(d, c, pieces);
                    if let Some(t) = &d.get(c).tail {
                        pieces.push(s(t.clone()));
                    }
                }
            }
            walk(&d, idx, &mut pieces);
            Ok(Value::List(PyList::from_vec(pieces)))
        }
        "clear" => {
            let mut d = doc.borrow_mut();
            {
                let node = d.get_mut(idx);
                node.attrib.clear();
                node.text = None;
                node.children.clear();
            }
            Ok(Value::None)
        }
        "text_content" => {
            let d = doc.borrow();
            let mut out = String::new();
            collect_string_content(&d, idx, &mut out);
            Ok(s(out))
        }
        "cssselect" => {
            // Bridge: cssselect (pure-python, staged) translates CSS to
            // XPath; run through our engine. Selector.text is routed here
            // by the python layer; this entry takes the raw css string.
            let css = match rest.first() {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => return Err(raise("TypeError", "cssselect() argument must be str")),
            };
            // Pure-python bridge (csstranslator) — import lazily.
            let interp = crate::builtins_modules::current_interp()
                .ok_or_else(|| raise("RuntimeError", "no interpreter"))?;
            let xpath_expr = css_to_xpath_via_bridge(&interp, &css)?;
            run_xpath(&doc, idx, doc_id, html, &xpath_expr)
        }
        "drop_tree" | "drop_tag" => {
            let mut d = doc.borrow_mut();
            let parent = match d.get(idx).parent {
                Some(p) => p,
                None => return Err(raise("AssertionError", "element to drop has no parent")),
            };
            // Preserve tail text: lxml drops the subtree but keeps tail as
            // a sibling tail (drop_tree semantics per lxml.html).
            let tail = d.get(idx).tail.clone();
            if let Some(pos) = d.get(parent).children.iter().position(|&c| c == idx) {
                d.get_mut(parent).children.remove(pos);
                if let Some(t) = tail {
                    // Tail text merges into the previous sibling's tail or
                    // the parent's text.
                    if pos > 0 {
                        let prev = d.get(parent).children[pos - 1];
                        let prev_tail = d.get_mut(prev).tail.take();
                        d.get_mut(prev).tail = Some(match prev_tail {
                            Some(pt) => pt + &t,
                            None => t,
                        });
                    } else {
                        let ptext = d.get_mut(parent).text.take();
                        d.get_mut(parent).text = Some(match ptext {
                            Some(pt) => pt + &t,
                            None => t,
                        });
                    }
                }
            }
            if fname == "drop_tree" {
                // Detach the subtree parent pointers (arena nodes remain but
                // are unreachable).
                fn detach(d: &mut LxDoc, ix: usize) {
                    d.get_mut(ix).parent = None;
                    let kids = d.get(ix).children.clone();
                    for k in kids {
                        detach(d, k);
                    }
                }
                detach(&mut d, idx);
            }
            Ok(Value::None)
        }
        "drop_tag" => {
            // Replace the element with its children (lxml.html.drop_tag).
            let mut d = doc.borrow_mut();
            let parent = match d.get(idx).parent {
                Some(p) => p,
                None => return Err(raise("AssertionError", "element to drop has no parent")),
            };
            let kids = d.get(idx).children.clone();
            let tail = d.get(idx).tail.clone();
            if let Some(pos) = d.get(parent).children.iter().position(|&c| c == idx) {
                d.get_mut(parent).children.remove(pos);
                for (n, &k) in kids.iter().enumerate() {
                    d.get_mut(k).parent = Some(parent);
                    d.get_mut(parent).children.insert(pos + n, k);
                }
                if let Some(t) = tail {
                    let last_child = d.get(parent).children.get(pos + kids.len()).copied();
                    match last_child {
                        Some(lc) => {
                            let lt = d.get_mut(lc).tail.take();
                            d.get_mut(lc).tail = Some(match lt {
                                Some(x) => x + &t,
                                None => t,
                            });
                        }
                        None => {
                            let pt = d.get_mut(parent).text.take();
                            d.get_mut(parent).text = Some(match pt {
                                Some(x) => x + &t,
                                None => t,
                            });
                        }
                    }
                }
            }
            Ok(Value::None)
        }
        "make_links_absolute" => {
            let base = match rest.first() {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => doc.borrow().base_url.clone().unwrap_or_default(),
            };
            let mut d = doc.borrow_mut();
            let mut targets: Vec<usize> = Vec::new();
            collect_descendants(&d, d.root, "a", &mut targets);
            for t in targets {
                if let Some(href) = attr_value(d.get(t), "href").map(|x| x.to_string()) {
                    let abs = urljoin_simple(&base, &href);
                    set_attrib_key(&mut d, t, "href", &abs);
                }
            }
            Ok(Value::None)
        }
        "makeelement" => {
            let tag = rest
                .first()
                .map(|v| str_arg(v, "makeelement"))
                .transpose()?;
            let Some(tag) = tag else {
                return Err(raise("TypeError", "makeelement() missing tag"));
            };
            let mut d = doc.borrow_mut();
            let ix = d.nodes.len();
            let mut node = LxNode::new(&tag, d.get(idx).html);
            node.parent = Some(idx);
            d.nodes.push(node);
            d.get_mut(idx).children.push(ix);
            Ok(make_element(doc_id, ix, html))
        }
        "append" => {
            let Some(child) = rest.first() else {
                return Err(raise("TypeError", "append() missing argument"));
            };
            let Some((_, cidx, _)) = elem_parts(child) else {
                return Err(raise("TypeError", "append() argument must be _Element"));
            };
            let mut d = doc.borrow_mut();
            d.get_mut(cidx).parent = Some(idx);
            d.get_mut(idx).children.push(cidx);
            Ok(Value::None)
        }
        "index" => {
            let d = doc.borrow();
            let child = rest.first().and_then(elem_parts).map(|(_, ci, _)| ci);
            let posn = child.and_then(|ci| d.get(idx).children.iter().position(|&c| c == ci));
            match posn {
                Some(p) => Ok(i(p as i64)),
                None => Err(raise("ValueError", "Element.index(x): element not found")),
            }
        }
        _ => Err(raise(
            "RuntimeError",
            "unreachable lxml element method dispatch arm",
        )),
    })();
    if matches!(&r, Err(_)) && !fname.starts_with("xpath") {
        return Some(r);
    }
    Some(r)
}

fn css_to_xpath_via_bridge(
    interp: &Rc<crate::frame::Interpreter>,
    css: &str,
) -> Result<String, Flow> {
    // Import cssselect and call HTMLTranslator().css_to_xpath(css).
    let cssselect = crate::importer::import_module(interp, "cssselect")?;
    let html_tr = crate::interp::attr_get_public(interp, &cssselect, "HTMLTranslator")?
        .ok_or_else(|| raise("ImportError", "cssselect.HTMLTranslator missing"))?;
    let tr = crate::interp::call_value_pub3(&html_tr, &[])?;
    let m = crate::interp::attr_get_public(interp, &tr, "css_to_xpath")?
        .ok_or_else(|| raise("AttributeError", "translator has no css_to_xpath"))?;
    let out = crate::interp::call_value_pub(&interp, &m, &[s(css.to_string())], &[])?;
    match out {
        Value::Str(x) => Ok(x.as_str().to_string()),
        other => crate::builtins::py_str_of(&other).map_err(|e| Flow::from_py(e)),
    }
}

/// Minimal urljoin (absolute URL + relative path fragment) for
/// make_links_absolute; w3lib handles the hard cases upstream.
fn urljoin_simple(base: &str, href: &str) -> String {
    if href.contains("://") || href.starts_with("data:") || base.is_empty() {
        return href.to_string();
    }
    let (scheme_host, bpath) = match base.find("://") {
        Some(p) => {
            let rest = &base[p + 3..];
            match rest.find('/') {
                Some(sp) => (&base[..p + 3 + sp], &base[p + 3 + sp..]),
                None => (base, "/"),
            }
        }
        None => ("", base),
    };
    if href.starts_with('/') {
        return format!("{scheme_host}{href}");
    }
    if href.starts_with('#') {
        let b = bpath.split('#').next().unwrap_or("");
        return format!("{scheme_host}{b}{href}");
    }
    let dir = match bpath.rfind('/') {
        Some(p) => &bpath[..p + 1],
        None => "/",
    };
    format!("{scheme_host}{dir}{href}")
}

// ===========================================================================
// etree.XPath class instances (parsel: etree.XPath(query, ...)(root))
// ===========================================================================

/// Build a compiled-XPath object: carries the query; calling it evaluates
/// against the receiver element. The object is a BuiltinFn marker named
/// `lxml.etree.XPath.__call__` so the generic marker dispatch
/// (call_builtin → call_native_marker, dotted names) routes its call into
/// call_xpath_object below — a plain Opaque falls off call_value_inner's
/// not-callable arm (real lxml XPath instances are callable).
thread_local! {
    /// Compiled XPath queries registered by `etree.XPath(query)`. The ctor
    /// returns a BuiltinFn marker `lxml.etree.XPath.call.<id>`; calls route
    /// through call_native_marker -> call_xpath_by_id (the marker dispatch
    /// has no object access, so the query rides the registry, not attrs).
    static XPATH_OBJECTS: std::cell::RefCell<Vec<(String, Option<Value>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub fn make_xpath_object(query: String, namespaces: Option<Value>) -> Value {
    let id = XPATH_OBJECTS.with(|c| {
        let mut r = c.borrow_mut();
        r.push((query, namespaces));
        r.len() - 1
    });
    let o = PyObject::new(
        ap_obj::ObjKind::BuiltinFn,
        format!("lxml.etree.XPath.call.{id}"),
    );
    Value::Obj(o)
}

/// Dispatch arm for `lxml.etree.XPath.call.<id>` markers: evaluate the
/// registered query against the receiver element (args[0]).
pub fn call_xpath_by_id(id: usize, args: &[Value]) -> Option<BResult> {
    let query = XPATH_OBJECTS.with(|c| c.borrow().get(id).map(|(q, _)| q.clone()))?;
    let recv = args.first()?.clone();
    let (doc, idx, html) = elem_parts(&recv)?;
    let doc_id = doc_id_of(&recv);
    Some(run_xpath(&doc, idx, doc_id, html, &query))
}

pub fn call_xpath_object(
    o: &Gc<PyObject>,
    args: &[Value],
    _kwargs: &[(String, Value)],
) -> Option<BResult> {
    let query = o.attrs.borrow().get(&s("<query>")).and_then(|v| match v {
        Value::Str(x) => Some(x.as_str().to_string()),
        _ => None,
    })?;
    let recv = args.first()?.clone();
    let (doc, idx, html) = elem_parts(&recv)?;
    let doc_id = doc_id_of(&recv);
    Some(run_xpath(&doc, idx, doc_id, html, &query))
}

/// Attribute read on the XPath OBJECT itself is inert; call goes through
/// call_native_object.
pub fn xpath_object_attr(o: &Gc<PyObject>, name: &str) -> Option<Value> {
    if o.name != "lxml.etree.XPath" {
        return None;
    }
    match name {
        "__name__" | "__doc__" => Some(Value::None),
        _ => None,
    }
}

// ===========================================================================
// Public accessors for sibling modules (builtins_modules native_iter)
// ===========================================================================

pub fn doc_id_public(v: &Value) -> usize {
    doc_id_of(v)
}

/// 02.08-c: instance attribute WRITE on an lxml element (c.text = "v",
/// c.tail = ..., c.attrib[k] = ...). Routes through the doc registry — the
/// element object is an Opaque with <doc_id>/<idx> parked in attrs.
pub fn elem_attr_set(obj: &Value, name: &str, v: Value) -> Result<(), crate::Flow> {
    let Some((doc, idx, _html)) = elem_parts(obj) else {
        return Err(crate::Flow::Raise(crate::exc_hier::ExceptionValue::msg(
            "AttributeError",
            format!("'{}' object has no attribute '{}'", "lxml element", name),
        )));
    };
    let doc_id = doc_id_of(obj);
    let _ = doc_id;
    match name {
        "text" | "tail" => {
            let sval = crate::builtins::py_str_of(&v)
                .map(|s| s.as_str().to_string())
                .unwrap_or_default();
            let mut d = doc.borrow_mut();
            let node = &mut d.nodes[idx];
            if name == "text" {
                node.text = if v.is_truthy() { Some(sval) } else { None };
            } else {
                node.tail = if v.is_truthy() { Some(sval) } else { None };
            }
            Ok(())
        }
        _ => Err(crate::Flow::Raise(crate::exc_hier::ExceptionValue::msg(
            "AttributeError",
            format!("'lxml element' object has no attribute '{}'", name),
        ))),
    }
}

pub fn make_element_public(doc_id: usize, idx: usize, html: bool) -> Value {
    make_element(doc_id, idx, html)
}

/// Direct children of an element (document order) — used by native_iter.
pub fn children_public(v: &Value) -> Vec<Value> {
    let Some((doc, idx, html)) = elem_parts(v) else {
        return Vec::new();
    };
    let doc_id = doc_id_of(v);
    let d = doc.borrow();
    d.get(idx)
        .children
        .iter()
        .map(|&c| make_element(doc_id, c, html))
        .collect()
}
