//! 02.08-c differential gates: the antipythony native lxml shim vs
//! CPython 3.14 + real lxml. Byte-exact comparison on a fixed corpus.
//!
//! Reference interpreter: CPython 3.14 with lxml installed in a uv venv at
//! /tmp/lxml-ref-venv (pure-wheel-only install). If either the reference
//! python or the venv is missing, tests SKIP (CI has them provisioned).

use std::process::Command;

fn ap_bin() -> String {
    std::env::var("CARGO_BIN_EXE_antipythony").unwrap_or_else(|_| {
        format!(
            "{}/../../target/debug/antipythony",
            env!("CARGO_MANIFEST_DIR")
        )
    })
}
const REF_VENV_PY: &str = "/tmp/lxml-ref-venv/bin/python";

fn ref_available() -> bool {
    std::path::Path::new(REF_VENV_PY).exists()
}

/// Run `body` (module-level statements) on both runtimes via temp files and
/// assert byte-identical stdout.
fn diff_check_main(body: &str) {
    if !ref_available() {
        eprintln!("SKIP: reference venv missing");
        return;
    }
    let tmp = std::env::temp_dir();
    // Unique per call: nanostamp alone can collide across parallel test
    // threads on macOS (clock granularity ~1us), clobbering fixtures.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let stamp = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64)
        ^ (SEQ.fetch_add(1, Ordering::SeqCst) << 1)
        ^ ((std::process::id() as u64) << 33);
    let ref_py = tmp.join(format!("lxml_diff_ref_{}.py", stamp));
    let ap_py = tmp.join(format!("lxml_diff_ap_{}.py", stamp));
    std::fs::write(&ref_py, body).unwrap();
    std::fs::write(&ap_py, body).unwrap();

    let out_ref = Command::new(REF_VENV_PY)
        .arg(&ref_py)
        .output()
        .expect("reference python failed");
    let bin = ap_bin();
    assert!(
        std::path::Path::new(&bin).exists(),
        "antipythony binary missing at {}",
        bin
    );
    let out_ap = Command::new(&bin)
        .args(["run", "--no-cache"])
        .arg(&ap_py)
        .output()
        .unwrap_or_else(|e| panic!("antipythony failed to spawn ({}): {}", bin, e));
    let _ = std::fs::remove_file(&ref_py);
    let _ = std::fs::remove_file(&ap_py);

    let ref_out = String::from_utf8_lossy(&out_ref.stdout).to_string();
    let ap_out = String::from_utf8_lossy(&out_ap.stdout).to_string();
    assert!(
        out_ap.status.success(),
        "antipythony exited with failure\n--- code ---\n{}\n--- stderr ---\n{}",
        body,
        String::from_utf8_lossy(&out_ap.stderr)
    );
    assert_eq!(
        ref_out, ap_out,
        "differential mismatch\n--- code ---\n{}\n--- ref ---\n{}\n--- ap ---\n{}",
        body, ref_out, ap_out
    );
}

#[test]
fn diff_etree_fromstring_tostring_roundtrip() {
    diff_check_main(
        r#"
from lxml import etree
r = etree.fromstring('<root><a x="1">t</a></root>')
print(etree.tostring(r, encoding='unicode'))
"#,
    );
}

#[test]
fn diff_etree_xpath_child_text() {
    diff_check_main(
        r#"
from lxml import etree
r = etree.fromstring('<root><item id="1">one</item><item id="2">two</item></root>')
print(r.xpath('item/text()'))
print(r.xpath('item/@id'))
"#,
    );
}

#[test]
fn diff_etree_xpath_predicates() {
    diff_check_main(
        r#"
from lxml import etree
r = etree.fromstring('<r><li>a</li><li class="c">b</li><li>c</li></r>')
print(r.xpath('//li[1]/text()'))
print(r.xpath("//li[@class='c']/text()"))
"#,
    );
}

#[test]
fn diff_etree_tag_attrib_text_tail() {
    diff_check_main(
        r#"
from lxml import etree
r = etree.fromstring('<root><a href="/x">link</a>tail</root>')
a = r[0]
print(a.tag, a.get('href'), a.text, a.tail)
print(dict(a.attrib))
"#,
    );
}

#[test]
fn diff_etree_subelement() {
    diff_check_main(
        r#"
from lxml import etree
r = etree.Element('root')
etree.SubElement(r, 'child', name='c').text = 'v'
print(etree.tostring(r, encoding='unicode'))
"#,
    );
}

#[test]
fn diff_etree_element_ctor() {
    diff_check_main(
        r#"
from lxml import etree
e = etree.Element('tag', attrib={'k': 'v'})
print(e.tag, e.get('k'))
"#,
    );
}

#[test]
fn diff_etree_iter() {
    diff_check_main(
        r#"
from lxml import etree
r = etree.fromstring('<r><a><b>t</b></a></r>')
print([e.tag for e in r.iter()])
print([e.tag for e in r.iter('b')])
"#,
    );
}

#[test]
fn diff_etree_find_findall() {
    diff_check_main(
        r#"
from lxml import etree
r = etree.fromstring('<r><a/><b/><a/></r>')
print(len(r.findall('a')), r.find('b').tag, r.find('.').tag)
"#,
    );
}

#[test]
fn diff_etree_xml_parser() {
    diff_check_main(
        r#"
from lxml import etree
p = etree.XMLParser(recover=False)
r = etree.fromstring('<x><y/></x>', p)
print(r.tag)
"#,
    );
}

#[test]
fn diff_etree_error_types() {
    diff_check_main(
        r#"
from lxml import etree
print(int(etree.ErrorTypes.ERR_INVALID_CHAR))
print(int(etree.ErrorTypes.ERR_INVALID_ENCODING))
"#,
    );
}

#[test]
fn diff_etree_xpatheval_class() {
    diff_check_main(
        r#"
from lxml import etree
r = etree.fromstring('<r><a>1</a><a>2</a></r>')
xp = etree.XPath('//a/text()')
print(xp(r))
"#,
    );
}

#[test]
fn diff_html_fromstring_basic() {
    diff_check_main(
        r#"
from lxml import html
r = html.fromstring('<html><body><div class="a b"><a href="/x">link</a></div></body></html>')
print(r.tag)
print(r.xpath('//a/text()'))
"#,
    );
}

#[test]
fn diff_html_tostring_method_html() {
    diff_check_main(
        r#"
from lxml import html, etree
r = html.fromstring('<html><body><input disabled type="checkbox"></body></html>')
print(etree.tostring(r, method='html', encoding='unicode', with_tail=False))
"#,
    );
}

#[test]
fn diff_html_attribute_lowercasing() {
    diff_check_main(
        r#"
from lxml import html
r = html.fromstring('<div DATA-X="1">t</div>')
print(r.xpath('//div/@data-x'))
"#,
    );
}

#[test]
fn diff_html_text_content() {
    diff_check_main(
        r#"
from lxml import html
r = html.fromstring('<div><p>hello <b>world</b></p></div>')
print(r.xpath('//p/text()'))
"#,
    );
}

#[test]
fn diff_html_void_elements() {
    diff_check_main(
        r#"
from lxml import html, etree
r = html.fromstring('<img src="x.png"><br>')
print(etree.tostring(r, method='html', encoding='unicode'))
"#,
    );
}

#[test]
fn diff_html_parser_class() {
    diff_check_main(
        r#"
from lxml import html
p = html.HTMLParser(recover=True)
r = html.fromstring('<p>x', parser=p)
print(r.tag)
"#,
    );
}

#[test]
fn diff_html_document_fromstring() {
    diff_check_main(
        r#"
from lxml import html
d = html.document_fromstring('<p>x</p>')
print(d.tag)
"#,
    );
}

#[test]
fn diff_html_fragment_wrap() {
    diff_check_main(
        r#"
from lxml import html
r = html.fromstring('plain text only')
print(r.tag, r.text)
"#,
    );
}

#[test]
fn diff_parsel_style_selector_path() {
    // The parsel 1.12.1 selector idiom: fromstring + xpath + text extraction.
    diff_check_main(
        r#"
from lxml import html
r = html.fromstring('<html><body><ul><li>a</li><li>b</li></ul></body></html>')
print(r.xpath('//ul/li/text()'))
print(r.xpath('//ul/li[2]/text()'))
"#,
    );
}

#[test]
fn diff_xpath_contains_predicate() {
    diff_check_main(
        r#"
from lxml import html
r = html.fromstring('<div><p class="x y">1</p><p class="z">2</p></div>')
print(r.xpath("//p[contains(@class, 'x')]/text()"))
"#,
    );
}

#[test]
fn diff_xpath_attr_paths() {
    diff_check_main(
        r#"
from lxml import html
r = html.fromstring('<a href="/h" name="n">t</a>')
print(r.xpath('//a/@href'))
print(r.xpath('//a/@name'))
"#,
    );
}

#[test]
fn diff_tostring_with_tail_false() {
    diff_check_main(
        r#"
from lxml import etree
r = etree.fromstring('<r><a/>tail</r>')
print(etree.tostring(r[0], encoding='unicode', with_tail=False))
"#,
    );
}

#[test]
fn diff_tostring_bytes_and_unicode() {
    diff_check_main(
        r#"
from lxml import etree
r = etree.fromstring('<root><a/></root>')
print(etree.tostring(r, encoding='unicode'))
print(etree.tostring(r).decode())
"#,
    );
}
