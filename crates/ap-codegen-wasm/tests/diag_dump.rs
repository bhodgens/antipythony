// Throwaway diagnostic: dump a function of a pre-lowered module file.
#[test]
fn diag_dump_file() {
    let bytes = std::fs::read(std::env::var("DIAG_FILE").unwrap_or("/tmp/dbg3.wasm".into())).expect("read wasm");
    let target: u32 = std::env::var("DIAG_FN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(36);
    let mut func_idx: u32 = 0;
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
            // code entries are locally-defined funcs; imports 0/1 shift by 2
            let real_idx = func_idx + 2;
            let range = body.range();
            if real_idx == target {
                eprintln!(
                    "function {} (bytes {:?}..{:?})",
                    real_idx, range.start, range.end
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
                        }
                        Err(e) => {
                            // normal end-of-body read; stop quietly
                            let _ = e;
                            break;
                        }
                    }
                }
            }
            func_idx += 1;
        }
    }
}
