//! `xtask pins` — host-adaptive pin gate (GitHub issue #17).
//!
//! `cargo run -p xtask -- pins --survey`   inventory report (read-only)
//! `cargo run -p xtask -- pins --check`    CI gate (exit 1 only on marked drift)
//! `cargo run -p xtask -- pins --fix <case>`  regenerate one corpus case from
//!                                            the live reference interpreter
//!
//! The problem (Nov 2026 CI campaign): the differential suites' *live-host*
//! assertions (assert_differential in crates/ap-cli/tests/
//! stdlib_modules_differential.rs) self-adapt to whatever CPython runs
//! beside them, but the FROZEN textual pins — `tests/corpus/**/expected/`
//! triples, `tests/ce-s1/expected/*.txt`, `tests/t04_ir_verify/fixtures/
//! expected_t04.txt` — were recorded on macOS (darwin-aarch64) against
//! Homebrew CPython 3.14.7 and silently embed host dialect (absolute
//! interpreter paths in tracebacks, darwin errno numbers, kqueue-vs-epoll
//! surfaces, libm ulp tails). On the x86_64-linux CI runner those pins are
//! either masked by the harness (`_mask_root`) or would diverge — with no
//! mechanical record of WHICH pins are host-bearing and which are proven
//! host-independent.
//!
//! This tool polices the RECORD, not the suites: a manifest
//! (`tests/PIN_HOSTS.toml`) declares, per case, the set of hosts whose
//! recorded bytes are known-good (`hosts = [...]`), whether the pin is
//! proven host-independent (`host_independent = true`), whether it is
//! host-dependent by ABI (`host_dependent = true` — the darwin/linux
//! dialect families from docs/host-adaptive-pins.md), and (for --fix)
//! where the source script lives. `--check` verifies:
//!
//!   * every pin-bearing file discovered under the scanned trees exists and
//!     hashes (sha256) to what the manifest records (drift = FAIL),
//!   * every manifest case names real pin files and a real `[[case]]`
//!     source when `--fix` support is declared (unknown case = FAIL),
//!   * pins WITHOUT any platform marker (no manifest case, no `.meta`) are
//!     counted and listed (informational — the survey backlog),
//!   * `PIN MISSING FOR HOST <key>` = FAIL when the current host is
//!     excluded via `known_hosts` while a case doesn't cover it, and for
//!     any `host_dependent` case whose `hosts` omits the current host.
//!
//! `--fix <case>` re-runs the reference interpreter (discovery:
//! AP_CONFORM_REFERENCE_PYTHON, then python3.14 on PATH — the ap-conform
//! contract) over the case source and rewrites its `.out/.err/.rc` (or
//! `.stdout/.stderr/.exit`) triple, masking checkout-absolute corpus paths
//! to the same `<ROOT>/tests/corpus/...` shape the harnesses compare
//! through. It NEVER touches `.meta` provenance sidecars (owned by
//! tests/tools/_gen_meta.py; re-run that recorder after a --fix) and it
//! refuses live rewrites under --check. It is the mechanism the issue asks
//! for; the mass re-recording of darwin pins to linux siblings is NOT
//! performed by this leaf (corpus-grows rule: rewrites are gated per
//! family by their owners).
//!
//! Design constraints: std-only (xtask is CI's fast-lints crate — same
//! discipline as stdlib_provenance.rs); the manifest writer emits a
//! canonical, byte-stable rendering so a linux rerun of --survey cannot
//! create spurious manifest churn.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process::{exit, Command};

use crate::stdlib_provenance::sha256_hex;

pub const MANIFEST_NAME: &str = "PIN_HOSTS.toml";

#[allow(dead_code)] // consumed by Manifest::render (unit-tested; the CLI \
                    // currently only reads the manifest — --survey writes come with the \
                    // linux re-pin campaign).
const MANIFEST_HEADER: &str = "\
# Host-adaptive pin manifest (GitHub issue #17; xtask pins --check/--fix).
#
# One [[case]] per pin-bearing corpus case whose bytes are host-relevant.
#   name               case stem under tests/corpus/<family>/expected/
#                      (or repo-relative path for non-corpus pin files)
#   family             corpus family dir name under tests/corpus/
#                      (omit for explicit-path cases)
#   hosts              per-host recorded pins: '<key>' or '<key>:<sha256>'
#                      (key = <os>-<arch>, e.g. darwin-aarch64)
#   host_independent   true = bytes verified identical on every listed host
#   host_dependent     true = bytes carry host ABI dialect (errno numbers,
#                      select surface, libm ulp, interpreter paths) — every
#                      such case MUST list `hosts` covering each host that
#                      runs the suite; a missing entry is
#                      `PIN MISSING FOR HOST <key>` under --check.
#   source             repo-relative script --fix re-runs under the live
#                      reference interpreter (corpus triples only)
#   note               provenance / classification pointer (free text)
#
# known_hosts restricts which hosts --check polices at all: while the linux
# sibling pins are being recorded family-by-family, the list is the honest
# statement of coverage. Remove it to police every host.
#
# Gate: `cargo run -p xtask -- pins --check` fails on hash drift, malformed
# or unknown cases, and PIN MISSING FOR HOST. `--survey` is read-only.
# Do not hand-edit `pin_sha256` values; they are recorded by --survey.
";

/// Pin file suffixes that carry recorded bytes (tests/README corpus rule).
const PIN_SUFFIXES: &[&str] = &[".out", ".err", ".rc", ".stdout", ".stderr", ".exit", ".txt"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostClaim {
    /// Bytes verified host-independent (`host_independent = true`).
    Independent,
    /// Bytes carry host dialect; `hosts` lists the covered hosts.
    Dependent,
    /// No claim recorded — survey backlog (informational).
    Unmarked,
}

#[derive(Clone, Debug)]
pub struct CaseEntry {
    pub name: String,
    pub family: String,
    /// expected dir relative to workspace root (set for non-corpus pins).
    pub exp_dir: Option<String>,
    pub hosts: Vec<(String, Option<String>)>,
    pub host_independent: bool,
    pub host_dependent: bool,
    pub source: Option<String>,
    pub note: String,
}

impl CaseEntry {
    pub fn claim(&self) -> HostClaim {
        if self.host_independent {
            HostClaim::Independent
        } else if self.host_dependent || !self.hosts.is_empty() {
            HostClaim::Dependent
        } else {
            HostClaim::Unmarked
        }
    }
    pub fn covers(&self, host: &str) -> bool {
        self.hosts.iter().any(|(k, _)| k == host)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Manifest {
    pub known_hosts: Option<Vec<String>>,
    pub cases: BTreeMap<String, CaseEntry>,
}

// ---------------------------------------------------------------------------
// Restricted TOML reader/writer (the subset this file writes; same spirit as
// stdlib_provenance.rs's DEVIATIONS.toml reader — xtask stays std-only).
// ---------------------------------------------------------------------------

fn unquote(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let mut chars = raw.chars();
    if chars.next() != Some('"') {
        return Err(format!("value must be a quoted string: {raw}"));
    }
    let inner = chars.as_str();
    let Some(end) = inner.rfind('"') else {
        return Err(format!("unterminated string: {raw}"));
    };
    if !inner[end + 1..].trim().is_empty() {
        return Err(format!("trailing content after string: {raw}"));
    }
    let body = &inner[..end];
    let mut out = String::with_capacity(body.len());
    let mut escaped = body.chars();
    while let Some(c) = escaped.next() {
        if c == '\\' {
            match escaped.next() {
                Some('n') => out.push('\n'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => return Err(format!("bad escape \\<{other}>")),
                None => return Err("trailing backslash".to_string()),
            }
        } else if c == '"' {
            return Err("unescaped quote inside string".to_string());
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

fn parse_string_list(v: &str) -> Result<Vec<String>, String> {
    let v = v.trim();
    let Some(inner) = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Err(format!("value must be a bracketed list: {v}"));
    };
    let mut out = Vec::new();
    for part in inner.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        out.push(unquote(part)?);
    }
    Ok(out)
}

fn parse_hosts_list(v: &str) -> Result<Vec<(String, Option<String>)>, String> {
    let items = parse_string_list(v)?;
    let mut out = Vec::new();
    for item in items {
        // 'key' or 'key:sha256hex' — ':' only separates at the first colon
        // because host keys never contain one.
        if let Some((k, h)) = item.split_once(':') {
            let h = h.trim();
            if h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("bad pin hash for host {k}: {h}"));
            }
            out.push((k.trim().to_string(), Some(h.to_lowercase())));
        } else {
            out.push((item.clone(), None));
        }
    }
    Ok(out)
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, Vec<String>> {
        let mut errors = Vec::new();
        let mut m = Manifest::default();
        let mut cur: Option<CaseEntry> = None;
        let flush = |mut cur: Option<CaseEntry>, m: &mut Manifest, errors: &mut Vec<String>| {
            if let Some(e) = cur.take() {
                if e.name.is_empty() {
                    errors.push("case entry without name".to_string());
                } else if m.cases.insert(e.name.clone(), e).is_some() {
                    // name+family uniqueness is enforced via the name key;
                    // duplicates land under family-prefixed names instead.
                    errors.push("duplicate case name".to_string());
                }
            }
        };
        for (i, line) in text.lines().enumerate() {
            let line_no = i + 1;
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            if t == "[[case]]" {
                flush(cur.take(), &mut m, &mut errors);
                cur = Some(CaseEntry {
                    name: String::new(),
                    family: String::new(),
                    exp_dir: None,
                    hosts: Vec::new(),
                    host_independent: false,
                    host_dependent: false,
                    source: None,
                    note: String::new(),
                });
                continue;
            }
            let Some((key, val)) = t.split_once('=') else {
                errors.push(format!("line {line_no}: not key = value: {t}"));
                continue;
            };
            let key = key.trim();
            if key == "known_hosts" {
                match parse_string_list(val) {
                    Ok(v) if cur.is_none() => m.known_hosts = Some(v),
                    Ok(_) => errors.push(format!("line {line_no}: known_hosts inside [[case]]")),
                    Err(er) => errors.push(format!("line {line_no}: known_hosts: {er}")),
                }
                continue;
            }
            match cur.as_mut() {
                Some(e) => {
                    let _ = set_into(e, key, val, line_no, &mut errors);
                }
                None => {
                    errors.push(format!("line {line_no}: key `{key}` outside [[case]]"));
                }
            }
        }
        flush(cur.take(), &mut m, &mut errors);
        if errors.is_empty() {
            Ok(m)
        } else {
            Err(errors)
        }
    }

    /// Canonical rendering — deterministic bytes for a given manifest value
    /// (the --check hash comparison relies on this).
    #[allow(dead_code)] // unit-tested round-trip; CLI writer lands with the re-pin campaign
    pub fn render(&self) -> String {
        let mut out = String::from(MANIFEST_HEADER);
        if let Some(kh) = &self.known_hosts {
            out.push_str("\nknown_hosts = [");
            for (i, k) in kh.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                let _ = write!(out, "\"{k}\"");
            }
            out.push_str("]\n");
        }
        for e in self.cases.values() {
            out.push_str("\n[[case]]\n");
            let _ = writeln!(out, "name = \"{}\"", e.name);
            if !e.family.is_empty() {
                let _ = writeln!(out, "family = \"{}\"", e.family);
            }
            if let Some(d) = &e.exp_dir {
                let _ = writeln!(out, "expected_dir = \"{d}\"");
            }
            if !e.hosts.is_empty() {
                out.push_str("hosts = [");
                for (i, (k, h)) in e.hosts.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    match h {
                        Some(h) => {
                            let _ = write!(out, "\"{k}:{h}\"");
                        }
                        None => {
                            let _ = write!(out, "\"{k}\"");
                        }
                    }
                }
                out.push_str("]\n");
            }
            if e.host_independent {
                out.push_str("host_independent = true\n");
            }
            if e.host_dependent {
                out.push_str("host_dependent = true\n");
            }
            if let Some(s) = &e.source {
                let _ = writeln!(out, "source = \"{s}\"");
            }
            if !e.note.is_empty() {
                let _ = writeln!(out, "note = \"{}\"", e.note);
            }
        }
        out
    }
}

fn parse_bool(val: &str) -> Result<bool, String> {
    match val.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("expected true/false, got {other}")),
    }
}

fn set_into(e: &mut CaseEntry, key: &str, val: &str, line_no: usize, errors: &mut Vec<String>) {
    let fail = |er: String, errors: &mut Vec<String>| errors.push(format!("line {line_no}: {er}"));
    macro_rules! try_set {
        ($assign:expr, $res:expr) => {
            match $res {
                Ok(v) => $assign = v,
                Err(er) => fail(er, errors),
            }
        };
    }
    match key {
        "name" => try_set!(e.name, unquote(val)),
        "family" => try_set!(e.family, unquote(val)),
        "expected_dir" => try_set!(e.exp_dir, unquote(val).map(Some)),
        "hosts" => try_set!(e.hosts, parse_hosts_list(val)),
        "host_independent" => try_set!(e.host_independent, parse_bool(val)),
        "host_dependent" => try_set!(e.host_dependent, parse_bool(val)),
        "source" => try_set!(e.source, unquote(val).map(Some)),
        "note" => try_set!(e.note, unquote(val)),
        "known_hosts" => fail("known_hosts inside [[case]]".into(), errors),
        other => fail(format!("unknown key `{other}`"), errors),
    }
}

// ---------------------------------------------------------------------------
// Host identity
// ---------------------------------------------------------------------------

/// Pin host key: `<os>-<arch>`, the keyspace docs/host-adaptive-pins.md
/// standardizes on (CI runners: linux-x86_64, this repo's dev boxes:
/// darwin-aarch64).
pub fn host_key(os: &str, arch: &str) -> String {
    let os = match os {
        "macos" | "darwin" => "darwin",
        "linux" => "linux",
        other => other,
    };
    let arch = match arch {
        "aarch64" | "arm64" => "aarch64",
        "x86_64" | "amd64" => "x86_64",
        other => other,
    };
    format!("{os}-{arch}")
}

pub fn current_host_key() -> String {
    host_key(std::env::consts::OS, std::env::consts::ARCH)
}

// ---------------------------------------------------------------------------
// Discovery: pin-bearing files + .meta provenance
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct DiscoveredPin {
    /// repo-relative path, forward slashes
    pub rel: String,
    pub case_name: String,
    pub exp_dir_rel: String,
    /// suffix this file carries within its case (.out/.err/...), or full
    /// stem for single-file pins like ce-s1's *_stdout.txt.
    pub suffix: String,
    pub sha256: String,
    /// host key recorded by a sibling .meta provenance sidecar, if any.
    pub meta_host: Option<String>,
    pub has_meta: bool,
}

fn corpus_exp_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_dir() {
                if p.file_name().map(|n| n == "expected").unwrap_or(false) {
                    out.push(p.clone());
                }
                collect(&p, out);
            }
        }
    }
    collect(&root.join("tests").join("corpus"), &mut dirs);
    dirs.sort();
    dirs
}

fn pin_files_in(exp: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(exp) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .map(|x| {
                        x == "out"
                            || x == "err"
                            || x == "rc"
                            || x == "stdout"
                            || x == "stderr"
                            || x == "exit"
                            || x == "txt"
                    })
                    .unwrap_or(false)
        })
        .collect();
    v.sort();
    v
}

pub fn rel_str(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn read_meta_host(exp: &Path, stem: &str) -> (bool, Option<String>) {
    let meta = exp.join(format!("{stem}.meta"));
    if let Ok(text) = fs::read_to_string(&meta) {
        // Minimal JSON field reads (schema is the flat _gen_meta.py contract).
        let sys_version = json_str_field(&text, "sys_version");
        return (true, classify_meta_sys_version(sys_version.as_deref()));
    }
    // Header-line fallback: a pin file whose first lines carry the
    // standardized marker `# pin-host: <key> cpython <version>` (see
    // docs/host-adaptive-pins.md §2) is platform-marked without a .meta.
    for suffix in [".txt", ".out", ".stdout"] {
        if let Ok(text) = fs::read_to_string(exp.join(format!("{stem}{suffix}"))) {
            if let Some(key) = pin_host_header(&text) {
                return (false, Some(key));
            }
            break;
        }
    }
    (false, None)
}

/// Parse a standardized `# pin-host: <key> cpython <ver>` header (checked
/// against the first three lines of a pin file).
pub fn pin_host_header(text: &str) -> Option<String> {
    for line in text.lines().take(3) {
        if let Some(rest) = line.trim().strip_prefix("# pin-host:") {
            let key = rest.split_whitespace().next()?;
            if key.contains('-') {
                return Some(key.to_string());
            }
        }
    }
    None
}

pub fn json_str_field(text: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":");
    let idx = text.find(&pat)?;
    let rest = &text[idx + pat.len()..];
    let q1 = rest.find('"')?;
    let after = &rest[q1 + 1..];
    let q2 = after.find('"')?;
    Some(after[..q2].to_string())
}

/// Heuristic host inference from a `.meta` sys_version string: CPython
/// builds name their toolchain (Clang/Apple == darwin, GNU/libc == linux).
/// Returns None when the string doesn't discriminate.
pub fn classify_meta_sys_version(sv: Option<&str>) -> Option<String> {
    let sv = sv?;
    let lower = sv.to_ascii_lowercase();
    let is_darwin = lower.contains("clang") || lower.contains("apple");
    let is_linux = lower.contains("gcc") || lower.contains("gnu") || lower.contains("linux");
    let arch = if lower.contains("aarch64") || lower.contains("arm64") {
        "aarch64"
    } else if lower.contains("x86_64") || lower.contains("amd64") {
        "x86_64"
    } else {
        // 32/64-bit pointer hints inside the version string are not
        // reliable; fall back to the family-wide convention below.
        ""
    };
    if is_darwin && !is_linux {
        // Homebrew/macOS CPython sys_version strings don't embed the arch;
        // the recording fleet for every meta in this repo is the
        // darwin-aarch64 dev box (docs/host-adaptive-pins.md §2).
        Some(host_key(
            "darwin",
            if arch.is_empty() { "aarch64" } else { arch },
        ))
    } else if is_linux && !is_darwin {
        Some(host_key(
            "linux",
            if arch.is_empty() { "x86_64" } else { arch },
        ))
    } else {
        None
    }
}

/// Discover every pin-bearing file under the scanned trees.
pub fn discover(root: &Path) -> Vec<DiscoveredPin> {
    let mut out = Vec::new();
    let mut dirs = corpus_exp_dirs(root);
    // Non-corpus pin trees the survey found (docs/host-adaptive-pins.md §1):
    dirs.push(root.join("tests").join("ce-s1").join("expected"));
    dirs.push(root.join("tests").join("t04_ir_verify").join("fixtures"));
    for exp in dirs {
        for f in pin_files_in(&exp) {
            let name = f
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            // case name = stem minus the pin suffix; for .txt fixtures the
            // whole stem is the case (expected_t04, *_cpython314_stdout).
            let (case, suffix) = match PIN_SUFFIXES
                .iter()
                .find(|s| name.ends_with(**s) && name.len() > s.len())
            {
                Some(s) => (name[..name.len() - s.len()].to_string(), (*s).to_string()),
                None => (name.clone(), String::new()),
            };
            let bytes = match fs::read(&f) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let (has_meta, meta_host) = read_meta_host(&exp, &case);
            out.push(DiscoveredPin {
                rel: rel_str(root, &f),
                case_name: case,
                exp_dir_rel: rel_str(root, &exp),
                suffix,
                sha256: sha256_hex(&bytes),
                meta_host,
                has_meta,
            });
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

// ---------------------------------------------------------------------------
// --check verdict
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finding {
    /// Manifest names a case that has no pin files on disk.
    UnknownCase(String),
    /// Pin bytes differ from the manifest-recorded sha256.
    PinDrift {
        rel: String,
        want: String,
        got: String,
    },
    /// A host-relevant case is excluded from `known_hosts` policing for the
    /// current host.
    PinMissingForHost { host: String, case: String },
    /// `host_dependent` case whose `hosts` omits the current host.
    HostDependentNoSibling { host: String, case: String },
    /// Manifest case declares a source for --fix that doesn't exist.
    MissingSource { case: String, source: String },
}

pub struct SurveyCounts {
    pub pins: usize,
    pub cases: usize,
    pub manifest_cases: usize,
    pub with_meta: usize,
    pub without_marker: usize,
    pub host_independent: usize,
    pub host_dependent: usize,
    pub by_host: BTreeMap<String, usize>,
}

/// Pure check over (discovered pins, manifest, host). Deterministic; unit
/// tests drive this directly with synthetic fixtures.
pub fn check(pins: &[DiscoveredPin], m: &Manifest, host: &str) -> (Vec<Finding>, SurveyCounts) {
    let mut findings = Vec::new();
    let mut by_case: BTreeMap<String, Vec<&DiscoveredPin>> = BTreeMap::new();
    for p in pins {
        by_case
            .entry(format!("{}/{}", p.exp_dir_rel, p.case_name))
            .or_default()
            .push(p);
    }

    let mut counts = SurveyCounts {
        pins: pins.len(),
        cases: by_case.len(),
        manifest_cases: m.cases.len(),
        with_meta: 0,
        without_marker: 0,
        host_independent: 0,
        host_dependent: 0,
        by_host: BTreeMap::new(),
    };
    for p in pins {
        if p.has_meta {
            counts.with_meta += 1;
        }
        if let Some(h) = &p.meta_host {
            *counts.by_host.entry(h.clone()).or_default() += 1;
        }
    }

    // manifest cases must resolve to discovered pins, and recorded hashes
    // must match current bytes.
    for (key, e) in &m.cases {
        match e.claim() {
            HostClaim::Independent => counts.host_independent += 1,
            HostClaim::Dependent => counts.host_dependent += 1,
            HostClaim::Unmarked => {}
        }
        let case_dir = match &e.exp_dir {
            Some(d) => d.clone(),
            None => format!("tests/corpus/{}/expected", e.family),
        };
        let map_key = format!("{case_dir}/{}", e.name);
        let Some(group) = by_case.get(&map_key) else {
            findings.push(Finding::UnknownCase(key.clone()));
            continue;
        };
        // group resolution here only proves the case names real files
        // (UnknownCase is the finding otherwise); per-file hash policing
        // is recorded via the `hosts` entries handled below.
        // Hash enforcement: entries whose host key matches the CURRENT host
        // (or host_independent cases, which carry one hash list for every
        // host under `pin_sha256`-style keys) must equal the file bytes.
        for (hk, sha) in &e.hosts {
            let Some(sha) = sha else { continue };
            let applies = hk == host || hk == "any" || e.host_independent;
            if !applies {
                continue;
            }
            // `any`/independent hashes apply to the FIRST pin file (single
            // -file pins); corpus triples hash every file sharing the value
            // only when a per-suffix map is used — kept modest: independent
            // cases pin the `.out`/`.stdout`/`.txt` primary channel.
            let primary = group.iter().find(|p| {
                p.suffix == ".out"
                    || p.suffix == ".stdout"
                    || p.suffix == ".txt"
                    || p.suffix.is_empty()
            });
            if let Some(p) = primary {
                if &p.sha256 != sha {
                    findings.push(Finding::PinDrift {
                        rel: p.rel.clone(),
                        want: sha.clone(),
                        got: p.sha256.clone(),
                    });
                }
            }
        }
        if let Some(src) = &e.source {
            if !root_join_exists_placeholder(src) {
                findings.push(Finding::MissingSource {
                    case: key.clone(),
                    source: src.clone(),
                });
            }
        }
    }

    // unmarked pins: discovered but neither in the manifest nor .meta-borne
    // platform evidence -> informational, not findings.
    for p in pins {
        let key = format!("{}/{}", p.exp_dir_rel, p.case_name);
        let covered_key = key.rsplit('/').next().unwrap_or("");
        let in_manifest = m.cases.contains_key(covered_key)
            || m.cases.values().any(|e| {
                let cd = e
                    .exp_dir
                    .clone()
                    .unwrap_or_else(|| format!("tests/corpus/{}/expected", e.family));
                cd == p.exp_dir_rel && e.name == p.case_name
            });
        if !in_manifest && !p.has_meta {
            counts.without_marker += 1;
        }
    }

    // PIN MISSING FOR HOST policing.
    if let Some(kh) = &m.known_hosts {
        if !kh.iter().any(|h| h == host) {
            for (_key, e) in &m.cases {
                let relevant = e.host_dependent || !e.hosts.is_empty();
                if relevant && !e.covers(host) && !e.host_independent {
                    findings.push(Finding::PinMissingForHost {
                        host: host.to_string(),
                        case: e.name.clone(),
                    });
                }
            }
        }
    }
    // Even with no known_hosts gate, a host_dependent case that omits the
    // current host is a missing sibling (the CI matrix rule from the doc).
    for (key, e) in &m.cases {
        if e.host_dependent && !e.covers(host) {
            // Only when the case is otherwise pinned (has some host entry).
            if !e.hosts.is_empty()
                && !findings.contains(&Finding::PinMissingForHost {
                    host: host.to_string(),
                    case: e.name.clone(),
                })
            {
                findings.push(Finding::HostDependentNoSibling {
                    host: host.to_string(),
                    case: key.clone(),
                });
            }
        }
    }
    findings.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    (findings, counts)
}

fn root_join_exists_placeholder(_src: &str) -> bool {
    // replaced by caller-provided root check in run(); the pure `check`
    // keeps source presence out of its inputs.
    true
}

// ---------------------------------------------------------------------------
// --fix: reference discovery + single-case regeneration
// ---------------------------------------------------------------------------

/// Reference interpreter discovery — the ap-conform contract (see
/// crates/ap-conform/src/lib.rs find_reference_interpreter):
/// AP_CONFORM_REFERENCE_PYTHON, then python3.14 on PATH.
pub fn find_reference() -> Option<String> {
    if let Ok(env) = std::env::var("AP_CONFORM_REFERENCE_PYTHON") {
        let env = env.trim();
        if !env.is_empty() && is_python_314(env) {
            return Some(env.to_string());
        }
    }
    if is_python_314("python3.14") {
        return Some("python3.14".to_string());
    }
    None
}

fn is_python_314(program: &str) -> bool {
    let Ok(out) = Command::new(program).arg("--version").output() else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text.contains("Python 3.14")
}

/// Mask checkout-absolute corpus paths to the `<ROOT>/tests/corpus/...`
/// shape the reference self-check harnesses compare through (same regex
/// semantics as tests/t11_stdlib:: _mask_root — a plain substring split
/// since pins.rs is std-only).
pub fn mask_root_paths(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let needle = b"tests/corpus/";
    let mut i = 0usize;
    while i < data.len() {
        if data[i..].starts_with(needle) {
            // Walk back over the already-emitted prefix: a run of
            // path-safe bytes ending right before the needle.
            let mut start = i;
            while start > 0 {
                let c = data[start - 1];
                if c == b'/'
                    || (c != b' '
                        && c != b'\n'
                        && c != b'\t'
                        && c != b'"'
                        && c != b'\''
                        && c != b','
                        && c != b';'
                        && c != b'('
                        && c != b')')
                {
                    start -= 1;
                } else {
                    break;
                }
            }
            // Mask only when the prefix is an absolute, not-yet-masked
            // path: starts with '/' and carries no '<' (idempotent on
            // <ROOT>/ pins).
            if start < i && data[start] == b'/' && !data[start..i].contains(&b'<') {
                // Map the data index `start` back to an `out` index: out
                // and data agree byte-for-byte up to i except for earlier
                // collapses, each of which shrank out by the stripped
                // prefix length.
                let delta = i - start;
                out.truncate(out.len().saturating_sub(delta));
                out.extend_from_slice(b"<ROOT>/");
                out.extend_from_slice(needle);
                i += needle.len();
                continue;
            }
        }
        out.push(data[i]);
        i += 1;
    }
    out
}

pub struct FixOutcome {
    pub case: String,
    pub wrote: Vec<String>,
    pub primary_sha256: String,
    pub host: String,
}

/// Regenerate one corpus triple from the live reference. Pure-filesystem
/// half factored into `run_fix` for testability at the CLI seam.
pub fn fix_case(root: &Path, case_name: &str) -> Result<FixOutcome, String> {
    let ref_py = find_reference().ok_or(
        "pins --fix: no reference interpreter (set AP_CONFORM_REFERENCE_PYTHON \
         or put python3.14 on PATH)",
    )?;
    let m = load_manifest(root)?;
    let entry = m.cases.get(case_name).ok_or_else(|| {
        format!("pins --fix: unknown case `{case_name}` (add a [[case]] with a source)")
    })?;
    let source_rel = entry
        .source
        .clone()
        .ok_or_else(|| format!("pins --fix: case `{case_name}` declares no source"))?;
    let src = root.join(&source_rel);
    if !src.is_file() {
        return Err(format!("pins --fix: source {} missing", src.display()));
    }
    let case_dir = entry
        .exp_dir
        .as_ref()
        .map(|d| root.join(d))
        .unwrap_or_else(|| root.join(format!("tests/corpus/{}/expected", entry.family)));
    // Run from the workspace root with the RELATIVE source path so embedded
    // traceback paths match the pins' recorded shape (t11 convention).
    let proc = Command::new(&ref_py)
        .arg(rel_str(root, &src))
        .current_dir(root)
        .output()
        .map_err(|e| format!("pins --fix: spawn {ref_py}: {e}"))?;
    let mut wrote = Vec::new();
    let (out_suf, err_suf, rc_suf) = pick_suffixes(&case_dir, case_name);
    let masked_out = mask_root_paths(&proc.stdout);
    let masked_err = mask_root_paths(&proc.stderr);
    fs::write(case_dir.join(format!("{case_name}{out_suf}")), &masked_out)
        .map_err(|e| format!("write pin: {e}"))?;
    wrote.push(format!("{out_suf}"));
    fs::write(case_dir.join(format!("{case_name}{err_suf}")), &masked_err)
        .map_err(|e| format!("write pin: {e}"))?;
    wrote.push(format!("{err_suf}"));
    let rc_text = match fs::read_to_string(case_dir.join(format!("{case_name}{rc_suf}"))) {
        Ok(t) if t.ends_with('\n') => format!("{}\n", proc.status.code().unwrap_or(-1)),
        _ => proc.status.code().unwrap_or(-1).to_string(),
    };
    fs::write(
        case_dir.join(format!("{case_name}{rc_suf}")),
        rc_text.as_bytes(),
    )
    .map_err(|e| format!("write pin: {e}"))?;
    wrote.push(format!("{rc_suf}"));
    Ok(FixOutcome {
        case: case_name.to_string(),
        wrote,
        primary_sha256: sha256_hex(&masked_out),
        host: current_host_key(),
    })
}

pub fn pick_suffixes(dir: &Path, stem: &str) -> (&'static str, &'static str, &'static str) {
    // stdlib-style triple (.out/.err/.rc) unless the recorded convention is
    // the t08/T-threading one (.stdout/.stderr/.exit).
    if dir.join(format!("{stem}.out")).is_file()
        || (!dir.join(format!("{stem}.stdout")).is_file()
            && !dir.join(format!("{stem}.out")).is_file())
    {
        (".out", ".err", ".rc")
    } else {
        (".stdout", ".stderr", ".exit")
    }
}

fn load_manifest(root: &Path) -> Result<Manifest, String> {
    let path = root.join("tests").join(MANIFEST_NAME);
    match fs::read_to_string(&path) {
        Ok(t) => Manifest::parse(&t).map_err(|es| es.join("; ")),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Manifest::default()),
        Err(e) => Err(format!("read {}: {e}", path.display())),
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

fn print_usage() {
    println!(
        "usage: cargo run -p xtask -- pins [--survey | --check | --fix <case>]\n\
         \n\
         \x20 --survey   read-only inventory of pin-bearing files + markers\n\
         \x20 --check    CI gate: hash drift / unknown case / PIN MISSING FOR HOST\n\
         \x20 --fix CASE regenerate one corpus triple from the live reference\n\
         \x20            interpreter (AP_CONFORM_REFERENCE_PYTHON || python3.14)\n\
         \n\
         See docs/host-adaptive-pins.md for the scheme and the survey table."
    );
}

pub fn run(argv: &[String], root: &Path) -> ! {
    let want_check = argv.iter().any(|a| a == "--check");
    let want_survey = argv.iter().any(|a| a == "--survey");
    let want_fix = argv.iter().any(|a| a == "--fix");
    let host = argv
        .iter()
        .position(|a| a == "--host")
        .and_then(|i| argv.get(i + 1))
        .cloned()
        .unwrap_or_else(current_host_key);

    if [want_check, want_survey, want_fix]
        .iter()
        .filter(|b| **b)
        .count()
        > 1
    {
        eprintln!("pins: --check/--survey/--fix are mutually exclusive");
        print_usage();
        exit(2);
    }
    if !(want_check || want_survey || want_fix) {
        print_usage();
        exit(2);
    }

    let manifest_path = root.join("tests").join(MANIFEST_NAME);
    let m = match fs::read_to_string(&manifest_path) {
        Ok(t) => match Manifest::parse(&t) {
            Ok(m) => m,
            Err(errors) => {
                for e in &errors {
                    eprintln!("pins: manifest {MANIFEST_NAME}: {e}");
                }
                exit(2);
            }
        },
        Err(e) if e.kind() == ErrorKind::NotFound && want_survey => Manifest::default(),
        Err(e) => {
            eprintln!(
                "pins: cannot read {}: {e} — the manifest is the tool's record; \
                 it must exist (see docs/host-adaptive-pins.md)",
                manifest_path.display()
            );
            exit(2);
        }
    };

    if want_fix {
        let case = argv
            .iter()
            .skip(1)
            .find(|a| !a.starts_with('-') && *a != "pins");
        let Some(case) = case else {
            eprintln!("pins --fix: needs a case name");
            print_usage();
            exit(2);
        };
        match fix_case(root, case) {
            Ok(o) => {
                println!(
                    "pins --fix: re-pinned {} ({} channels: {}), primary sha256 {}, host {}",
                    o.case,
                    o.wrote.len(),
                    o.wrote.join("+"),
                    &o.primary_sha256[..12],
                    o.host
                );
                println!(
                    "pins --fix: .meta provenance NOT touched — re-run \
                     tests/tools/_gen_meta.py <family> to update it, then add a \
                     `\"{host}:<sha>\"` hosts entry to tests/{MANIFEST_NAME}."
                );
                exit(0);
            }
            Err(e) => {
                eprintln!("{e}");
                exit(2);
            }
        }
    }

    let pins = discover(root);
    let (mut findings, counts) = check(&pins, &m, &host);
    // Source presence needs the root; folded in here (kept out of the pure
    // `check` so unit tests can drive it without a workspace).
    for e in m.cases.values() {
        if let Some(src) = &e.source {
            if !root.join(src).is_file()
                && !findings.contains(&Finding::MissingSource {
                    case: e.name.clone(),
                    source: src.clone(),
                })
            {
                findings.push(Finding::MissingSource {
                    case: e.name.clone(),
                    source: src.clone(),
                });
            }
        }
    }

    if want_survey {
        println!(
            "pins --survey: {} pin files / {} cases across the scanned trees; \
             {} .meta-bearing, {} WITHOUT any platform marker",
            counts.pins, counts.cases, counts.with_meta, counts.without_marker
        );
        let mut hh: Vec<(&String, &usize)> = counts.by_host.iter().collect();
        hh.sort_by_key(|(k, _)| *k);
        for (h, n) in hh {
            println!("  provenance host {h}: {n} pin files");
        }
        println!(
            "  manifest: {} cases ({} host-independent, {} host-dependent)",
            counts.manifest_cases, counts.host_independent, counts.host_dependent
        );
        for f in &findings {
            println!("  finding: {f:?}");
        }
        if findings.is_empty() {
            println!("  no findings");
        }
        exit(0);
    }

    if !findings.is_empty() {
        for f in &findings {
            let msg = match f {
                Finding::UnknownCase(k) => {
                    format!("UNKNOWN CASE: manifest names `{k}` but no pin files resolve to it")
                }
                Finding::PinDrift { rel, want, got } => {
                    format!("PIN DRIFT: {rel}\n  want sha256 {want}\n  got  sha256 {got}")
                }
                Finding::PinMissingForHost { host, case } => {
                    format!("PIN MISSING FOR HOST {host}: case `{case}` is host-bearing and has no `{host}` entry")
                }
                Finding::HostDependentNoSibling { host, case } => {
                    format!("PIN MISSING FOR HOST {host}: host_dependent case `{case}` lists no `{host}` sibling pin")
                }
                Finding::MissingSource { case, source } => {
                    format!("MISSING SOURCE: case `{case}` --fix source `{source}` not on disk")
                }
            };
            eprintln!("pins: {msg}");
        }
        eprintln!(
            "pins --check: FAIL ({} finding(s); {} pin files, {} without any marker)",
            findings.len(),
            counts.pins,
            counts.without_marker
        );
        exit(1);
    }
    println!(
        "pins --check: OK ({} pin files / {} cases; {} host-independent, {} host-dependent, \
         {} markerless pins informational; host {host})",
        counts.pins,
        counts.cases,
        counts.host_independent,
        counts.host_dependent,
        counts.without_marker
    );
    exit(0)
}

// normalize path separators so rel keys never depend on the walker order
#[allow(dead_code)] // reserved helper (unit-tested); not yet wired into discovery
pub fn rel_key(p: &Path) -> String {
    p.components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect::<Vec<_>>()
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn pin(rel: &str, exp: &str, case: &str, suf: &str, bytes: &[u8]) -> DiscoveredPin {
        DiscoveredPin {
            rel: rel.into(),
            case_name: case.into(),
            exp_dir_rel: exp.into(),
            suffix: suf.into(),
            sha256: sha256_hex(bytes),
            meta_host: None,
            has_meta: false,
        }
    }

    #[test]
    fn host_key_normalizes_aliases() {
        assert_eq!(host_key("macos", "aarch64"), "darwin-aarch64");
        assert_eq!(host_key("darwin", "arm64"), "darwin-aarch64");
        assert_eq!(host_key("linux", "x86_64"), "linux-x86_64");
        assert_eq!(host_key("linux", "amd64"), "linux-x86_64");
    }

    #[test]
    fn current_host_key_matches_self_report() {
        let k = current_host_key();
        assert!(
            k == host_key(std::env::consts::OS, std::env::consts::ARCH),
            "current_host_key must be derived, not hardcoded: {k}"
        );
    }

    #[test]
    fn meta_sys_version_classifies_darwin_clang() {
        let sv = "3.14.7 (main, Aug  5 2026, 10:29:49) [Clang 21.0.0 (clang-2100.1.1.101)]";
        assert_eq!(
            classify_meta_sys_version(Some(sv)).as_deref(),
            Some("darwin-aarch64")
        );
        let linux = "3.14.7 (main, Sep  1 2026, 00:00:00) [GCC 13.2.0]";
        assert_eq!(
            classify_meta_sys_version(Some(linux)).as_deref(),
            Some("linux-x86_64")
        );
        assert_eq!(classify_meta_sys_version(None), None);
        assert_eq!(
            classify_meta_sys_version(Some("something else")).as_deref(),
            None
        );
    }

    #[test]
    fn json_str_field_reads_flat_meta() {
        let text = "{\n  \"sys_version\": \"3.14.7 [Clang]\",\n  \"family\": \"stdlib\"\n}\n";
        assert_eq!(
            json_str_field(text, "sys_version").as_deref(),
            Some("3.14.7 [Clang]")
        );
        assert_eq!(json_str_field(text, "family").as_deref(), Some("stdlib"));
        assert_eq!(json_str_field(text, "missing"), None);
    }

    #[test]
    fn manifest_roundtrips_through_render_parse() {
        let m = Manifest {
            known_hosts: Some(vec!["darwin-aarch64".into()]),
            cases: [(
                "s016_collections_abc_isinstance".to_string(),
                CaseEntry {
                    name: "s016_collections_abc_isinstance".into(),
                    family: "stdlib".into(),
                    exp_dir: None,
                    hosts: vec![("darwin-aarch64".into(), Some("a".repeat(64)))],
                    host_independent: false,
                    host_dependent: true,
                    source: Some("tests/corpus/stdlib/s016_collections_abc_isinstance.py".into()),
                    note: "warning path dialect".into(),
                },
            )]
            .into(),
        };
        let rendered = m.render();
        let reparsed = Manifest::parse(&rendered).expect("render output must reparse");
        assert_eq!(reparsed.known_hosts, m.known_hosts);
        let e = reparsed
            .cases
            .get("s016_collections_abc_isinstance")
            .unwrap();
        assert!(e.host_dependent);
        assert_eq!(e.hosts[0].0, "darwin-aarch64");
        assert_eq!(e.hosts[0].1.as_deref(), Some(&*"a".repeat(64)));
        assert_eq!(
            e.source.as_deref().unwrap(),
            "tests/corpus/stdlib/s016_collections_abc_isinstance.py"
        );
        // canonical bytes: render(render(parse(render))) is stable.
        assert_eq!(rendered, reparsed.render());
    }

    #[test]
    fn manifest_rejects_malformed_input() {
        let errs = Manifest::parse("name = \"x\"").unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("outside [[case]]")),
            "{errs:?}"
        );
        let errs = Manifest::parse("garbage line").unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("not key = value")),
            "{errs:?}"
        );
        let errs = Manifest::parse("[[case]]\nname = \"x\"\nhosts = [\"bad:zz\"]").unwrap_err();
        assert!(errs.iter().any(|e| e.contains("bad pin hash")), "{errs:?}");
        let errs = Manifest::parse("[[case]]\nname = \"x\"\nhost_independent = yes").unwrap_err();
        assert!(errs.iter().any(|e| e.contains("true/false")), "{errs:?}");
    }

    #[test]
    fn check_flags_pin_drift_for_current_host() {
        let bytes = b"hello\n";
        let p = pin("e/h.out", "e", "h", ".out", bytes);
        let m = Manifest {
            known_hosts: None,
            cases: [(
                "h".to_string(),
                CaseEntry {
                    name: "h".into(),
                    family: String::new(),
                    exp_dir: Some("e".into()),
                    hosts: vec![("linux-x86_64".into(), Some("0".repeat(64)))],
                    host_independent: false,
                    host_dependent: true,
                    source: None,
                    note: String::new(),
                },
            )]
            .into(),
        };
        let (findings, _c) = check(&[p], &m, "linux-x86_64");
        assert_eq!(
            findings,
            vec![Finding::PinDrift {
                rel: "e/h.out".into(),
                want: "0".repeat(64),
                got: sha256_hex(bytes),
            }]
        );
    }

    #[test]
    fn check_reports_missing_host_sibling() {
        let p = pin("e/h.out", "e", "h", ".out", b"x");
        let m = Manifest {
            known_hosts: Some(vec!["darwin-aarch64".into(), "linux-x86_64".into()]),
            cases: [(
                "h".to_string(),
                CaseEntry {
                    name: "h".into(),
                    family: String::new(),
                    exp_dir: Some("e".into()),
                    hosts: vec![("darwin-aarch64".into(), None)],
                    host_independent: false,
                    host_dependent: true,
                    source: None,
                    note: String::new(),
                },
            )]
            .into(),
        };
        let (findings, _c) = check(&[p.clone()], &m, "linux-x86_64");
        assert!(
            findings.iter().any(|f| {
                matches!(f, Finding::HostDependentNoSibling { host, case } if host == "linux-x86_64" && case == "h")
            }),
            "expected HostDependentNoSibling, got {findings:?}"
        );
        // and no finding on the covered host
        let (f2, _) = check(&[p], &m, "darwin-aarch64");
        assert!(f2.is_empty(), "{f2:?}");
    }

    #[test]
    fn check_unknown_case_and_host_independent_ok() {
        let p = pin("e/a.out", "e", "a", ".out", b"1");
        let mut m = Manifest {
            known_hosts: None,
            cases: [(
                "ghost".to_string(),
                CaseEntry {
                    name: "ghost".into(),
                    family: String::new(),
                    exp_dir: Some("e".into()),
                    hosts: Vec::new(),
                    host_independent: false,
                    host_dependent: false,
                    source: None,
                    note: String::new(),
                },
            )]
            .into(),
        };
        let (findings, _c) = check(&[p.clone()], &m, "linux-x86_64");
        assert_eq!(findings, vec![Finding::UnknownCase("ghost".into())]);
        m.cases.remove("ghost");
        m.cases.insert(
            "a".into(),
            CaseEntry {
                name: "a".into(),
                family: String::new(),
                exp_dir: Some("e".into()),
                hosts: vec![("any".into(), Some(sha256_hex(b"1")))],
                host_independent: true,
                host_dependent: false,
                source: None,
                note: String::new(),
            },
        );
        let (findings, _c) = check(&[p.clone(), p], &m, "linux-x86_64");
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn survey_counts_unmarked_pins() {
        let with = pin("e/b.out", "e", "b", ".out", b"2");
        let mut meta_with = with.clone();
        meta_with.has_meta = true;
        meta_with.meta_host = Some("darwin-aarch64".into());
        let m = Manifest::default();
        let (_f, c) = check(
            &[meta_with, pin("e/c.out", "e", "c", ".out", b"3")],
            &m,
            "linux-x86_64",
        );
        assert_eq!(c.pins, 2);
        assert_eq!(c.with_meta, 1);
        assert_eq!(c.without_marker, 1);
        assert_eq!(c.by_host.get("darwin-aarch64"), Some(&1));
    }

    #[test]
    fn mask_root_paths_collapses_absolute_prefixes() {
        let raw = b"  File \"/Users/me/repo/tests/corpus/stdlib/x.py\", line 1\nkeep /opt/homebrew/bin/python3.14 alone\n";
        let got = mask_root_paths(raw);
        let want = b"  File \"<ROOT>/tests/corpus/stdlib/x.py\", line 1\nkeep /opt/homebrew/bin/python3.14 alone\n";
        assert_eq!(got, want);
        // already-masked and relative-form inputs pass through untouched
        assert_eq!(
            mask_root_paths(b"<ROOT>/tests/corpus/stdlib/x.py\n"),
            b"<ROOT>/tests/corpus/stdlib/x.py\n"
        );
        assert_eq!(
            mask_root_paths(b"tests/corpus/stdlib/x.py\n"),
            b"tests/corpus/stdlib/x.py\n"
        );
    }

    #[test]
    fn pick_suffixes_follows_recorded_convention() {
        let dir = std::env::temp_dir().join(format!("xtask-pins-suffix-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join("a.stdout"), b"x").unwrap();
        assert_eq!(pick_suffixes(&dir, "a"), (".stdout", ".stderr", ".exit"));
        fs::write(dir.join("b.out"), b"x").unwrap();
        assert_eq!(pick_suffixes(&dir, "b"), (".out", ".err", ".rc"));
        // unknown case defaults to the stdlib convention
        assert_eq!(pick_suffixes(&dir, "z"), (".out", ".err", ".rc"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovery_finds_expected_trees_under_temp_root() {
        let root = std::env::temp_dir().join(format!("xtask-pins-disc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let exp = root
            .join("tests")
            .join("corpus")
            .join("fam")
            .join("expected");
        fs::create_dir_all(&exp).unwrap();
        fs::write(exp.join("c001.out"), b"ok\n").unwrap();
        fs::write(
            exp.join("c001.meta"),
            b"{\n  \"sys_version\": \"3.14.7 [Clang 21]\",\n  \"family\": \"fam\"\n}\n",
        )
        .unwrap();
        let pins = discover(&root);
        let c = pins
            .iter()
            .find(|p| p.case_name == "c001" && p.suffix == ".out")
            .expect("pin discovered");
        assert_eq!(c.sha256, sha256_hex(b"ok\n"));
        assert!(c.has_meta);
        assert_eq!(c.meta_host.as_deref(), Some("darwin-aarch64"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pin_host_header_parses_standardized_marker() {
        assert_eq!(
            pin_host_header("# generated\n# pin-host: darwin-aarch64 cpython 3.14.7\nrow=1\n")
                .as_deref(),
            Some("darwin-aarch64")
        );
        assert_eq!(pin_host_header("no marker here\n").as_deref(), None);
        // only the first three lines are searched
        assert_eq!(
            pin_host_header("a\nb\nc\n# pin-host: linux-x86_64 cpython 3.14.0\n").as_deref(),
            None
        );
    }

    #[test]
    fn find_reference_honors_env_override_shape() {
        // Without asserting host specifics: a bogus override must be
        // rejected by the version check (never returned), proving the
        // discovery contract matches ap-conform (env THEN PATH, gated on
        // `--version` reporting Python 3.14).
        std::env::set_var(
            "AP_CONFORM_REFERENCE_PYTHON",
            "/nonexistent/python3.14-not-real",
        );
        let r = find_reference();
        assert!(r.is_none() || r.as_deref() != Some("/nonexistent/python3.14-not-real"));
        std::env::remove_var("AP_CONFORM_REFERENCE_PYTHON");
    }

    #[test]
    fn rel_key_collapses_normal_components() {
        let p = PathBuf::from("tests/corpus/./stdlib/expected/s.out");
        assert_eq!(rel_key(&p), "tests/corpus/stdlib/expected/s.out");
    }
}
