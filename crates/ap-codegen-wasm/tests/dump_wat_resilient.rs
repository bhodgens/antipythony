// Full wat dump with @offset prefixes to a file (survives printer errors:
// prints as much as wasmprinter can). DIAG_SRC=/tmp/x.wasm WAT_OUT=/tmp/x.wat
#[test]
fn dump_wat_resilient() {
    use std::io;
    let path = std::env::var("DIAG_SRC").unwrap_or("/tmp/min.wasm".into());
    let out = std::env::var("WAT_OUT").unwrap_or("/tmp/full.wat".into());
    let bytes = std::fs::read(&path).unwrap();
    struct Off(std::fs::File);
    impl wasmprinter::Print for Off {
        fn write_str(&mut self, s: &str) -> io::Result<()> {
            use std::io::Write;
            self.0.write_all(s.as_bytes())
        }
        fn newline(&mut self) -> io::Result<()> {
            use std::io::Write;
            self.0.write_all(b"\n")
        }
        fn start_line(&mut self, binary_offset: Option<u64>) {
            if let Some(off) = binary_offset {
                use std::io::Write;
                let _ = write!(self.0, "@{off:x} ");
            }
        }
    }
    let mut sink = Off(std::fs::File::create(&out).unwrap());
    match wasmprinter::Config::new().print(&bytes, &mut sink) {
        Ok(_) => println!("wat written to {out}"),
        Err(e) => println!("printer stopped: {e} (partial written to {out})"),
    }
}
