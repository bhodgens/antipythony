// for-range module dump + validate+locate (the range() emit-fix driver).
#[test]
fn dump_range_min() {
    let src = "for j in range(3):\n    print(j)\nprint('after')\n";
    let (module, _) = ap_parse::lower_source(src).expect("parse");
    let bytes = ap_codegen_wasm::lower_module(&module).expect("lower");
    std::fs::write("/tmp/min.wasm", &bytes).expect("write");
    println!("bytes {}", bytes.len());
}
