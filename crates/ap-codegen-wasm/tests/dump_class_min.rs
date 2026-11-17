// Dump a module for diag_fn: EXW009 shape (super() two-arg — fn[75] fails).
#[test]
fn dump_class_min() {
    let src = r#"
class SA:
    def __init__(self):
        super(SA, 5)
SA()
"#;
    let (module, _) = ap_parse::lower_source(src).expect("parse");
    let bytes = ap_codegen_wasm::lower_module(&module).expect("lower");
    std::fs::write("/tmp/min.wasm", &bytes).expect("write");
    println!("bytes {}", bytes.len());
}
