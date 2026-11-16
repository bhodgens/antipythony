
#[test]
fn print_fns() {
    let bytes = std::fs::read("/tmp/min.wasm").unwrap();
    let out = wasmprinter::print_bytes(&bytes).unwrap();
    for (i, line_block) in out.split("(func ").enumerate() {
        if i >= 1 {
            let head = line_block.lines().take(80).collect::<Vec<_>>().join("\n");
            println!("=== func-block {i} ===\n{}", &head[..head.len().min(2600)]);
        }
    }
}
