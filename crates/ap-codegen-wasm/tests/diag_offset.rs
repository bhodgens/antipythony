// Throwaway diagnostic: locate the function containing the byte offset
// reported by wasmtime validation, and dump its locals + operator trace.
#[test]
fn diag_locate_offset() {
    let b = ap_ir::Builder::new();
    let mut module = ap_ir::Module::new();
    module.items.push(ap_ir::Item::Stmt(b.assign(
        vec![b.target_name("n")],
        b.bin_op(ap_ir::expr::BinOp::Pow, b.int_lit(2), b.int_lit(100)),
    )));
    module.items.push(ap_ir::Item::Stmt(b.expr_stmt(
        b.call(b.name("print"), vec![b.arg_pos(b.name("n"))]),
    )));
    let bytes = ap_codegen_wasm::lower_module(&module).expect("lower");
    eprintln!("module bytes: {}", bytes.len());

    let target: u64 = std::env::var("DIAG_OFF")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5163);

    let mut func_idx: u32 = 0;
    let mut found = false;
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
            let range = body.range();
            let (start, end) = (range.start, range.end);
            if !found && start <= target && target <= end {
                eprintln!(
                    "offset {} inside function index {} ({}..{})",
                    target, func_idx, start, end
                );
                let mut locals = body.get_locals_reader().unwrap();
                let n = locals.get_count();
                for _ in 0..n {
                    let (count, ty) = locals.read().unwrap();
                    eprintln!("  local group: {} x {:?}", count, ty);
                }
                let mut opreader = body.get_operators_reader().unwrap();
                loop {
                    match opreader.read() {
                        Ok(op) => {
                            let off = opreader.original_position();
                            eprintln!("  @{:06x} {:?}", off, op);
                            if let wasmparser::Operator::End = op {
                                break;
                            }
                        }
                        Err(e) => {
                            eprintln!("  OPERATOR READ ERROR: {}", e);
                            break;
                        }
                    }
                }
                found = true;
            }
            func_idx += 1;
        }
    }
    if !found {
        eprintln!("offset {} not inside any code entry", target);
    }
}
