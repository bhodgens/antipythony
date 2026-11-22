//! Link flags: export the Py* C-API symbols from the antipythony binary so
//! dlopen'd extensions resolve them against the running process (the
//! source-shim loading model, leaf 10.02).

fn main() {
    // Cargo-native per-target rustflag: -export_dynamic makes the whole
    // binary's #[no_mangle] symbols visible to dlopen'd dylibs on macOS.
    println!("cargo:rustc-link-arg-bins=-Wl,-export_dynamic");
}
