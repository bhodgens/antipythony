//! Dump ALL operators of the body containing DIAG_OFFSET (default 8794).
#[ignore = "developer scratch tool: hard-reads a /tmp/*.wasm no CI step writes"]
#[test]
fn diag_offset() {
    let bytes = std::fs::read("/tmp/min.wasm").unwrap();
    let target: u64 = std::env::var("DIAG_OFFSET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8794);
    use wasmparser::Payload;
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let Payload::CodeSectionEntry(f) = payload.unwrap() {
            let a = f.range().start as u64;
            if !(a..=f.range().end as u64).contains(&target) {
                continue;
            }
            println!("body range {}..{}", a, f.range().end);
            let mut ops = f.get_operators_reader().unwrap();
            loop {
                match ops.read_with_offset() {
                    Ok((op, off)) => println!("  @{} {:?}", off as u64, op),
                    Err(_) => break,
                }
            }
        }
    }
}
