// Full wat dump of DIAG_SRC to /tmp/full.wat (offsets prefixed).
#[ignore = "developer scratch tool: reads a local scratch wasm file ($DIAG_SRC/$DIAG_FILE)"]
#[test]
fn dump_wat_full() {
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
    match wasmprinter::Config::new().print(&bytes, &mut sink) {
        Ok(()) => {}
        Err(e) => println!("PRINT-ERR (partial dump): {e}"),
    }
    std::fs::write("/tmp/full.wat", &sink.0).unwrap();
    println!("wrote /tmp/full.wat ({} lines)", sink.0.lines().count());
}
