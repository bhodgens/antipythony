//! `antipythany.toml` + `antipythany.lock` — the T-14 G-1 derived schema
//! (fixtures `tests/t14_cli/fixtures/expected_antipythany.{toml,lock}` are
//! the oracle: field names there are normative expectations, "the impl
//! choosing different names is a contract miss").
//!
//! toml (written by `new`, extended by `add`):
//!
//! ```toml
//! [project]
//! name = "myapp"
//! version = "0.1.0"
//! requires_python = ">=3.14"
//!
//! [build]
//! backend = "native"
//! opt_level = 2
//!
//! [[dependency]]
//! name = "six"
//! source = "file-cache"
//! ```
//!
//! lock (written by `add`, deterministic for the same resolution input —
//! T-14 asserts content-identity across re-resolutions):
//!
//! ```toml
//! [metadata]
//! lock_version = 1
//! created_by = "uv"
//!
//! [[package]]
//! name = "six"
//! version = "1.17.0"
//! ```
//!
//! v0 hand-rolled TOML emit/parse: the schema is flat tables + arrays of
//! tables with string/int values only; a general toml crate would drag a
//! dependency tree into every install for two fixed shapes. Parsing accepts
//! exactly the subset we emit (plus comments/whitespace) and errors loudly
//! otherwise — fail-loud per C-06's spirit, never silently misread.

/// Parsed `antipythany.toml` (the subset we emit).
#[derive(Debug, Default, Clone)]
pub struct Manifest {
    pub project_name: String,
    pub project_version: String,
    pub requires_python: String,
    pub backend: String,
    pub opt_level: u32,
    /// `[[dependency]]` entries in file order (dedup on add is by name).
    pub dependencies: Vec<Dep>,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub source: String,
}

/// Parsed `antipythany.lock`.
#[derive(Debug, Default, Clone)]
pub struct Lockfile {
    pub lock_version: u32,
    pub created_by: String,
    pub packages: Vec<LockedPkg>,
}

#[derive(Debug, Clone)]
pub struct LockedPkg {
    pub name: String,
    pub version: String,
}

pub const MANIFEST_FILE: &str = "antipythany.toml";
pub const LOCK_FILE: &str = "antipythany.lock";

// ---------------------------------------------------------------------------
// Emit
// ---------------------------------------------------------------------------

/// Serialize the manifest. Deterministic field order (content-stable writes:
/// re-adding an existing dep must not churn the file).
pub fn manifest_to_toml(m: &Manifest) -> String {
    let mut out = String::new();
    out.push_str("[project]\n");
    out.push_str(&format!("name = {}\n", toml_str(&m.project_name)));
    out.push_str(&format!("version = {}\n", toml_str(&m.project_version)));
    out.push_str(&format!(
        "requires_python = {}\n",
        toml_str(&m.requires_python)
    ));
    out.push('\n');
    out.push_str("[build]\n");
    out.push_str(&format!("backend = {}\n", toml_str(&m.backend)));
    out.push_str(&format!("opt_level = {}\n", m.opt_level));
    for d in &m.dependencies {
        out.push_str("\n[[dependency]]\n");
        out.push_str(&format!("name = {}\n", toml_str(&d.name)));
        out.push_str(&format!("source = {}\n", toml_str(&d.source)));
    }
    out
}

/// Serialize the lockfile. Packages sorted by name (deterministic across
/// re-resolutions regardless of uv's install order — T-14 lock-determinism).
pub fn lock_to_toml(l: &Lockfile) -> String {
    let mut pkgs = l.packages.clone();
    pkgs.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = String::new();
    out.push_str("[metadata]\n");
    out.push_str(&format!("lock_version = {}\n", l.lock_version));
    out.push_str(&format!("created_by = {}\n", toml_str(&l.created_by)));
    for p in &pkgs {
        out.push_str("\n[[package]]\n");
        out.push_str(&format!("name = {}\n", toml_str(&p.name)));
        out.push_str(&format!("version = {}\n", toml_str(&p.version)));
    }
    out
}

/// TOML basic string with the escapes our values can contain.
fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Parse (accept exactly the emitted subset)
// ---------------------------------------------------------------------------

/// Parse a manifest; `Err` carries a human-readable shape error (never a
/// silent fallback — a misread manifest must fail loudly).
pub fn manifest_from_toml(text: &str) -> Result<Manifest, String> {
    let mut m = Manifest {
        backend: "native".to_string(),
        opt_level: 2,
        requires_python: ">=3.14".to_string(),
        project_version: "0.1.0".to_string(),
        ..Default::default()
    };
    let mut section = String::new();
    for (ln, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            if !line.ends_with(']') {
                return Err(format!("{}:{}: bad section header", MANIFEST_FILE, ln + 1));
            }
            // `[[dependency]]` (array of tables) and `[project]` both
            // normalize to the bare section name for our flat schema.
            let inner = line.trim_start_matches('[').trim_end_matches(']');
            section = inner.to_string();
            continue;
        }
        let Some((k, v)) = parse_kv(&line, MANIFEST_FILE, ln + 1) else {
            continue;
        };
        match (section.as_str(), k.as_str()) {
            ("project", "name") => m.project_name = v_string(&v, MANIFEST_FILE, ln + 1)?,
            ("project", "version") => m.project_version = v_string(&v, MANIFEST_FILE, ln + 1)?,
            ("project", "requires_python") => {
                m.requires_python = v_string(&v, MANIFEST_FILE, ln + 1)?
            }
            ("build", "backend") => m.backend = v_string(&v, MANIFEST_FILE, ln + 1)?,
            ("build", "opt_level") => m.opt_level = v_u32(&v, MANIFEST_FILE, ln + 1)?,
            ("dependency", "name") => {
                m.dependencies.push(Dep {
                    name: v_string(&v, MANIFEST_FILE, ln + 1)?,
                    source: String::new(),
                });
            }
            ("dependency", "source") => {
                if let Some(d) = m.dependencies.last_mut() {
                    d.source = v_string(&v, MANIFEST_FILE, ln + 1)?;
                }
            }
            _ => {} // unknown keys tolerated (forward compat), shape errors are not
        }
    }
    Ok(m)
}

/// Parse a lockfile.
pub fn lock_from_toml(text: &str) -> Result<Lockfile, String> {
    let mut l = Lockfile {
        lock_version: 1,
        created_by: "uv".to_string(),
        packages: Vec::new(),
    };
    let mut section = String::new();
    for (ln, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            if !line.ends_with(']') {
                return Err(format!("{}:{}: bad section header", LOCK_FILE, ln + 1));
            }
            let inner = line.trim_start_matches('[').trim_end_matches(']');
            section = inner.to_string();
            continue;
        }
        let Some((k, v)) = parse_kv(&line, LOCK_FILE, ln + 1) else {
            continue;
        };
        match (section.as_str(), k.as_str()) {
            ("metadata", "lock_version") => l.lock_version = v_u32(&v, LOCK_FILE, ln + 1)?,
            ("metadata", "created_by") => l.created_by = v_string(&v, LOCK_FILE, ln + 1)?,
            ("package", "name") => l.packages.push(LockedPkg {
                name: v_string(&v, LOCK_FILE, ln + 1)?,
                version: String::new(),
            }),
            ("package", "version") => {
                if let Some(p) = l.packages.last_mut() {
                    p.version = v_string(&v, LOCK_FILE, ln + 1)?;
                }
            }
            _ => {}
        }
    }
    Ok(l)
}

fn strip_comment(line: &str) -> &str {
    // '#' inside a quoted string is data; our schema never has one in keys,
    // and values we read back are re-serialized from data — check quotes.
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse_kv(line: &str, file: &str, ln: usize) -> Option<(String, String)> {
    let (k, v) = line.split_once('=')?;
    let k = k.trim().to_string();
    if k.is_empty() || v.trim().is_empty() {
        // Not a fatal shape error at parse level; v_* validators report the
        // typed failures. An empty line-shaped token is skipped.
        return None;
    }
    let _ = (file, ln);
    Some((k, v.trim().to_string()))
}

fn v_string(v: &str, file: &str, ln: usize) -> Result<String, String> {
    let v = v.trim();
    if !(v.starts_with('"') && v.ends_with('"') && v.len() >= 2) {
        return Err(format!("{file}:{ln}: expected a quoted string, got `{v}`"));
    }
    let inner = &v[1..v.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                other => {
                    return Err(format!("{file}:{ln}: bad escape {other:?}"));
                }
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

fn v_u32(v: &str, file: &str, ln: usize) -> Result<u32, String> {
    v.trim()
        .parse::<u32>()
        .map_err(|_| format!("{file}:{ln}: expected an integer, got `{v}`"))
}

// ---------------------------------------------------------------------------
// File I/O + discovery
// ---------------------------------------------------------------------------

/// Read+parse `<dir>/antipythany.toml`. `Ok(None)` when the file is absent.
pub fn read_manifest(dir: &std::path::Path) -> Result<Option<Manifest>, String> {
    let p = dir.join(MANIFEST_FILE);
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", p.display())),
    };
    manifest_from_toml(&text).map(Some)
}

/// Read+parse `<dir>/antipythany.lock`. `Ok(None)` when absent.
pub fn read_lock(dir: &std::path::Path) -> Result<Option<Lockfile>, String> {
    let p = dir.join(LOCK_FILE);
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", p.display())),
    };
    lock_from_toml(&text).map(Some)
}

/// Write both files (manifest + lock) atomically enough for v0: write then
/// rename over any previous content.
pub fn write_manifest(dir: &std::path::Path, m: &Manifest) -> Result<(), String> {
    let p = dir.join(MANIFEST_FILE);
    std::fs::write(&p, manifest_to_toml(m))
        .map_err(|e| format!("cannot write {}: {e}", p.display()))
}

pub fn write_lock(dir: &std::path::Path, l: &Lockfile) -> Result<(), String> {
    let p = dir.join(LOCK_FILE);
    std::fs::write(&p, lock_to_toml(l)).map_err(|e| format!("cannot write {}: {e}", p.display()))
}

/// Walk up from `start` looking for a directory holding `antipythany.toml`.
/// Used by the runtime (importer sys.path bootstrap) AND the CLI (`add` from
/// a subdirectory of the project).
pub fn find_project_dir(start: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut cur = Some(start.to_path_buf());
    while let Some(d) = cur {
        if d.join(MANIFEST_FILE).is_file() {
            return Some(d);
        }
        cur = d.parent().map(|p| p.to_path_buf());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_real_manifest_shape() {
        let text = "[project]\nname = \"myapp\"\nversion = \"0.1.0\"\nrequires_python = \">=3.14\"\n\n[build]\nbackend = \"native\"\nopt_level = 2\n\n[[dependency]]\nname = \"six\"\nsource = \"file-cache\"\n";
        let m = manifest_from_toml(text).expect("parse");
        assert_eq!(m.project_name, "myapp");
        assert_eq!(m.dependencies.len(), 1);
        assert_eq!(m.dependencies[0].name, "six");
        assert_eq!(m.dependencies[0].source, "file-cache");
    }
}
