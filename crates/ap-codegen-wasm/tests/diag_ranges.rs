// List all function byte ranges in the module.
#[test]
fn diag_ranges() {
    let path = std::env::var("DIAG_FILE").unwrap_or("/tmp/min.wasm".into());
    let bytes = std::fs::read(path).unwrap();
    let mut func_idx: u32 = 0;
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
            eprintln!(
                "code {} (fn {}) bytes {:?}",
                func_idx,
                func_idx + 2,
                body.range()
            );
            func_idx += 1;
        }
    }
}
