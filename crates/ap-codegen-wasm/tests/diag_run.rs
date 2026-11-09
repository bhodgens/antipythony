// Throwaway diagnostic: run the emitted module under wasmtime (ap-host) and
// print the precise validation error.
use ap_codegen_wasm::lower_module;

#[test]
fn diag_run_host() {
    let b = ap_ir::Builder::new();
    let mut module = ap_ir::Module::new();
    module.items.push(ap_ir::Item::Stmt(b.assign(
        vec![b.target_name("n")],
        b.bin_op(ap_ir::expr::BinOp::Pow, b.int_lit(2), b.int_lit(100)),
    )));
    module.items.push(ap_ir::Item::Stmt(
        b.expr_stmt(b.call(b.name("print"), vec![b.arg_pos(b.name("n"))])),
    ));
    let bytes = lower_module(&module).expect("lower");
    match ap_host::run(&bytes, 1 << 20) {
        Ok(r) => {
            eprintln!(
                "RC={} STDOUT={:?} STDERR={:?}",
                r.exit_code,
                r.stdout.replace('\n', "\\n"),
                r.stderr.replace('\n', "\\n")
            );
        }
        Err(e) => eprintln!("HOST ERROR: {}", e),
    }
}
