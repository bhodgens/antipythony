// Throwaway diagnostic: dump type-section func entries (DIAG_TY=N..N+6)
// of the pre-lowered module at /tmp/dbg3.wasm, via a local re-walk that
// avoids the iterator/count API mismatch.
#[test]
fn diag_types() {
    let bytes = std::fs::read("/tmp/dbg3.wasm").expect("read /tmp/dbg3.wasm");
    let want: u32 = std::env::var("DIAG_TY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(48);
    // collect ALL type-section entries first
    let mut entries: Vec<String> = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let wasmparser::Payload::TypeSection(ts) = payload.unwrap() {
            let r: Result<Vec<_>, _> = ts.into_iter().collect();
            for t in r.unwrap() {
                entries.push(format!("{:?}", t));
            }
        }
    }
    eprintln!("total types: {}", entries.len());
    for i in want as usize..(want as usize + 6).min(entries.len()) {
        eprintln!("type[{}] = {}", i, entries[i]);
    }
}
