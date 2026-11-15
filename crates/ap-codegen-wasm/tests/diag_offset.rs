// Dump the function containing a byte offset: print a window of ops around it.
#[test]
fn diag_offset() {
    let bytes = std::fs::read("/tmp/dbg3.wasm").expect("read /tmp/dbg3.wasm");
    let target: usize = std::env::var("DIAG_OFF")
        .ok()
        .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        .map(|v| v as usize)
        .unwrap_or(0xe71);
    let mut func_idx: u32 = 0;
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
            let real_idx = func_idx + 2;
            let range = body.range();
            let (rs, re) = (range.start as usize, range.end as usize);
            if rs <= target && target <= re {
                eprintln!(
                    "== offset {:#x} is in function {} (bytes {:?}..{:?})",
                    target, real_idx, range.start, range.end
                );
                let mut opreader = body.get_operators_reader().unwrap();
                loop {
                    match opreader.read() {
                        Ok(op) => {
                            let off = opreader.original_position();
                            if off.abs_diff(target as u64) <= 60 {
                                eprintln!("  @{:06x} {:?}", off, op);
                            }
                            if let wasmparser::Operator::End = op {
                                if off > target as u64 {
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("  OPERATOR READ ERROR: {}", e);
                            break;
                        }
                    }
                }
            }
            func_idx += 1;
        }
    }
}
