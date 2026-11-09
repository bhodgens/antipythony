//! Throwaway diagnostic binary: run the emitted module under ap-host and
//! print the precise validation error (SPDX-License-Identifier:
//! Apache-2.0 WITH LLVM-exception).
fn main() {
    let b = ap_ir::Builder::new();
    let mut module = ap_ir::Module::new();
    module.items.push(ap_ir::Item::Stmt(b.assign(
        vec![b.target_name("n")],
        b.bin_op(ap_ir::expr::BinOp::Pow, b.int_lit(2), b.int_lit(100)),
    )));
    module.items.push(ap_ir::Item::Stmt(
        b.expr_stmt(b.call(b.name("print"), vec![b.arg_pos(b.name("n"))])),
    ));
    let bytes = ap_codegen_wasm::lower_module(&module).expect("lower");
    std::fs::write("/tmp/dbg2.wasm", &bytes).unwrap();
    match ap_host::run(&bytes, 1 << 20) {
        Ok(r) => eprintln!("RC={} STDOUT={:?} STDERR={:?}", r.exit_code, r.stdout, r.stderr),
        Err(e) => eprintln!("HOST ERROR: {:?}", e),
    }
}
