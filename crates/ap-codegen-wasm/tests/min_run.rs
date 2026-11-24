//! End-to-end differential probes, isolating features.
#[test]
fn hello_still_runs() {
    let b = ap_ir::Builder::new();
    let mut m = ap_ir::Module::new();
    m.items.push(ap_ir::Item::Stmt(b.expr_stmt(
        b.call(b.name("print"), vec![b.arg_pos(b.str_lit("hello, wasm"))]),
    )));
    let bytes = ap_codegen_wasm::lower_module(&m).unwrap();
    match ap_host::run(&bytes, 64 * 1024) {
        Ok(o) => println!("HELLO={:?} rc={}", o.stdout, o.exit_code),
        Err(e) => println!("HELLO-ERR={}", e),
    }
}

#[test]
fn min_run() {
    let src = std::env::var("PROBE").unwrap_or_else(|_| {
        r#"
i = 0
while i < 3:
    i = i + 1
"#
        .to_string()
    });
    let (m, _) = ap_parse::lower_source(&src).unwrap();
    let bytes = ap_codegen_wasm::lower_module(&m).unwrap();
    std::fs::write("/tmp/min.wasm", &bytes).unwrap();
    eprintln!("PROBE: compiling wasm ({} bytes)", bytes.len());
    match ap_host::compile_only(&bytes) {
        Ok(_) => eprintln!("PROBE: compiled ok"),
        Err(e) => eprintln!("PROBE: compile err {}", e),
    }
    eprintln!("PROBE: running wasm");
    match ap_host::run(&bytes, 64 * 1024) {
        Ok(o) => println!(
            "MINOUT={:?} ERR={:?} rc={}",
            o.stdout, o.stderr, o.exit_code
        ),
        Err(e) => println!("MINERR={}", e),
    }
    eprintln!("PROBE: done");
}
