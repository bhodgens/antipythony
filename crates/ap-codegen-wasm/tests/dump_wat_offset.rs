// Print the wat with per-line @offset prefixes — the 03.04b tooling recipe
// (PROGRESS 312-317), wasmprinter 0.263 Print trait (write_str/newline API).
#[ignore = "developer scratch tool: reads a local scratch wasm file ($DIAG_SRC/$DIAG_FILE)"]
#[test]
fn dump_wat_offset() {
    use std::io;
    let path = std::env::var("DIAG_SRC").unwrap_or("/tmp/min.wasm".into());
    let bytes = std::fs::read(&path).unwrap();
    struct Off(String);
    impl wasmprinter::Print for Off {
        fn write_str(&mut self, s: &str) -> io::Result<()> {
            self.0.push_str(s);
            Ok(())
        }
        fn newline(&mut self) -> io::Result<()> {
            self.0.push('\n');
            Ok(())
        }
        fn start_line(&mut self, binary_offset: Option<u64>) {
            if let Some(off) = binary_offset {
                self.0.push_str(&format!("@{off:x} "));
            }
        }
    }
    let mut sink = Off(String::new());
    wasmprinter::Config::new().print(&bytes, &mut sink).unwrap();
    for line in sink.0.lines() {
        println!("{line}");
    }
}
