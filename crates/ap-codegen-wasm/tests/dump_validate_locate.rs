// Validate + locate: run wasmparser validation on DIAG_SRC and print the
// exact wat lines around the failing offset (drives the 03.04c emit-fix loop).
#[test]
fn dump_validate_locate() {
    use std::io;
    let path = std::env::var("DIAG_SRC").unwrap_or("/tmp/min.wasm".into());
    let bytes = std::fs::read(&path).unwrap();
    use wasmparser::Validator;
    let feats = wasmparser::WasmFeatures::default()
        | wasmparser::WasmFeatures::GC
        | wasmparser::WasmFeatures::FUNCTION_REFERENCES
        | wasmparser::WasmFeatures::EXCEPTIONS;
    let off = match Validator::new_with_features(feats).validate_all(&bytes) {
        Ok(_) => {
            println!("DUMP VALIDATES");
            return;
        }
        Err(e) => {
            println!("DUMP VALIDATE-ERR: {}", e);
            // wasmparser error offset: parse the trailing "(at offset 0x...)"
            let s = format!("{}", e);
            s.rsplit("(at offset 0x")
                .next()
                .map(|_| ())
                .unwrap_or(());
            let hex = s
                .split("(at offset 0x")
                .nth(1)
                .and_then(|t| t.split(')').next())
                .and_then(|t| u64::from_str_radix(t, 16).ok());
            hex.unwrap_or(0)
        }
    };
    println!("-- wat around @{off:x} --");
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
    if wasmprinter::Config::new().print(&bytes, &mut sink).is_err() {
        // printer may fail on the same malformed module; still dump what we have
    }
    let lines: Vec<&str> = sink.0.lines().collect();
    let mut start_idx = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if let Some(o) = line
            .strip_prefix("@")
            .and_then(|t| t.split(' ').next())
            .and_then(|t| u64::from_str_radix(t, 16).ok())
        {
            if o >= off.saturating_sub(400) {
                start_idx = i;
                break;
            }
        }
    }
    for line in &lines[start_idx..(start_idx + 80).min(lines.len())] {
        println!("{line}");
    }
}
