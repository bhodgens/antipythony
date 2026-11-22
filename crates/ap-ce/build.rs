//! Compile the C shim (PyArg_ParseTuple varargs + spec walker + init
//! registry) into the ap-ce staticlib so the extension links one object.

fn main() {
    let src = std::path::Path::new("c/apx_shim.c");
    println!("cargo:rerun-if-changed=c/apx_shim.c");
    println!("cargo:rerun-if-changed=../../include/apx_python.h");
    if !src.exists() {
        return;
    }
    let mut b = cc::Build::new();
    b.file(src)
        .include("../../include")
        .flag_if_supported("-std=c11")
        .flag_if_supported("-fno-stack-check")
        .warnings(false);
    b.compile("apx_shim");
}
