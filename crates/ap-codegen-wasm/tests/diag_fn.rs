//! Dump a single wasm function's operators (DIAG_FN=abs fn idx, DIAG_SRC=src
//! file; default fn 69, /tmp/min.wasm) — the 03.04c emit-fix tool.
#[ignore = "developer scratch tool: reads a local scratch wasm file ($DIAG_SRC/$DIAG_FILE)"]
#[test]
fn diag_fn() {
    let path = std::env::var("DIAG_SRC").unwrap_or("/tmp/min.wasm".into());
    let bytes = std::fs::read(&path).unwrap();
    let n: usize = std::env::var("DIAG_FN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(69);
    use wasmparser::Payload;
    let mut i = 0usize;
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let Payload::CodeSectionEntry(f) = payload.unwrap() {
            if i == n - 3 {
                println!(
                    "=== wasm function[{}] body range {}..{}",
                    i + 3,
                    f.range().start,
                    f.range().end
                );
                let mut ops = f.get_operators_reader().unwrap();
                let mut count = 0;
                loop {
                    match ops.read() {
                        Ok(op) => {
                            println!("  {:?}", op);
                            count += 1;
                            if count > 400 {
                                println!("  ...truncated");
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                break;
            }
            i += 1;
        }
    }
}
