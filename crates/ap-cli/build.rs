//! Link flags: export the Py* C-API symbols from the antipythony binary so
//! dlopen'd extensions resolve them against the running process (the
//! source-shim loading model, leaf 10.02).
//!
//! GOTCHA (CI, 2026-11-24): the spellings are ld-flavor-specific and BOTH
//! break the other platform:
//! * darwin ld: `-Wl,-export_dynamic` (single dash);
//! * GNU ld (linux): `-Wl,--export-dynamic` (two dashes, underscore NOT
//!   hyphen — `--export_dynamic` is rejected);
//! The darwin spelling `-Wl,-export_dynamic` handed to GNU ld SEGFAULTS the
//! linked binary AT STARTUP on linux (process dies before main, SIGSEGV at
//! pc=0 inside the panic machinery; reproduced in an ubuntu container with a
//! 2-crate probe binary — ap-cli was fully innocent). The GNU spelling
//! handed to darwin ld fails the link outright ("ld: unknown options").
//! Hence the cfg below. Verified: linux probe + ap-cli bin + its 2 unit
//! tests green in an ubuntu container; macOS links + `--version` runs.

fn main() {
    let flag = if cfg!(target_os = "macos") {
        "-Wl,-export_dynamic"
    } else {
        "-Wl,--export-dynamic"
    };
    println!("cargo:rustc-link-arg-bins={flag}");
}
