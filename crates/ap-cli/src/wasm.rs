//! `antipythony build --wasm` — the 04.04 wasm distribution verb.
//!
//! Pipeline (identical front end to the Plan 03 corpus gates): ap-parse →
//! ap-ir → [`ap_codegen_wasm::lower_module_ex`] → `.wasm` bytes written to
//! the output path. The emitted module is the SAME shape `ap-host` runs and
//! `examples/browser-smoke` embeds (WASI p1 fd_write/proc_exit + the 3-import
//! `aprt` surface, `aprt-version` stamp) — see docs/run-wasm-anywhere.md for
//! the runner matrix.
//!
//! Byte-exactness: `lower_module` is a pure function of the ap-ir module +
//! source name (no timestamps, no environment entropy), so identical input
//! yields identical bytes — the gate pins this against the Plan 03 dump-test
//! output rather than trusting the property.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::path::Path;

/// Build a `.wasm` module from Python source (the `build --wasm` verb body).
///
/// * `source` — the Python source text;
/// * `script_path` — invoked path (only its file stem feeds the emitter's
///   traceback/`__file__` name, mirroring the corpus gates passing the stem);
/// * `out_path` — where the module bytes land (conventionally `app.wasm`).
///
/// Errors are the CLI's fail-loud strings (SyntaxError / Unsupported /
/// io) — `cmd_build` returns them verbatim.
pub fn build_wasm(source: &str, script_path: &str, out_path: &Path) -> Result<(), String> {
    let parsed = ap_parse::parse(source)
        .map_err(|e| format!("antipythony: SyntaxError: {} (line {})", e.message, e.line))?;
    let (module, _scopes) =
        ap_parse::lower(&parsed).map_err(|e| format!("antipythony: lower: {e}"))?;

    // The emitter's source-name seam takes the FILE NAME (corpus gates pass
    // the case stem); a bare `app.py` has no parent component.
    let name = Path::new(script_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| script_path.to_string());

    let bytes = ap_codegen_wasm::lower_module_ex(&module, Some(&name)).map_err(|e| match e {
        ap_codegen_wasm::CodegenError::Unsupported(what) => format!(
            "antipythony: build --wasm: unsupported construct in the wasm backend: {what} \
                 (the construct runs under `antipythony run`; the wasm lowering does not serve \
                 it yet — see docs/run-wasm-anywhere.md for the supported slice)"
        ),
    })?;

    std::fs::write(out_path, &bytes)
        .map_err(|e| format!("antipythony: cannot write {}: {e}", out_path.display()))?;
    Ok(())
}
