//! `antipythany new` scaffold + project-dir resolution for `add`/`run`.
//!
//! `new <name>` creates:
//! ```text
//! <name>/
//!   antipythany.toml     (the manifest schema, [project]+[build])
//!   <name>.py            ("hello, antipythany" seed — the T-14 app bytes)
//! ```

use crate::manifest::{self, Manifest};

/// The scaffolded app body. T-14 asserts `b"hello, antipythany"` in `run`
/// stdout — the seed prints exactly that line.
pub const APP_SEED: &str = "print(\"hello, antipythany\")\n";

/// Scaffold a project dir under `parent/<name>`; errors when it exists.
pub fn new_project(parent: &std::path::Path, name: &str) -> Result<std::path::PathBuf, String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "antipythany: invalid project name `{name}` (use [A-Za-z0-9_-])"
        ));
    }
    let dir = parent.join(name);
    if dir.exists() {
        return Err(format!("antipythany: `{}` already exists", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("antipythany: cannot create {name}: {e}"))?;
    let m = Manifest {
        project_name: name.to_string(),
        project_version: "0.1.0".to_string(),
        requires_python: ">=3.14".to_string(),
        backend: "native".to_string(),
        opt_level: 2,
        dependencies: Vec::new(),
    };
    manifest::write_manifest(&dir, &m)?;
    let app = dir.join(format!("{name}.py"));
    std::fs::write(&app, APP_SEED)
        .map_err(|e| format!("antipythany: cannot write {}: {e}", app.display()))?;
    Ok(dir)
}
