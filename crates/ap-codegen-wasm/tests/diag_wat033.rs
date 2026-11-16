// dump full WAT of /tmp/min033.wasm to /tmp/min033.wat (leaf 03.03 diag)
#[test]
fn dump_wat() {
    let bytes = std::fs::read("/tmp/min033.wasm").unwrap();
    let out = wasmprinter::print_bytes(&bytes).unwrap();
    std::fs::write("/tmp/min033.wat", out).unwrap();
    println!("dumped");
}
