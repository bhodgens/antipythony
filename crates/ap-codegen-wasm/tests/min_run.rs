#[test]
fn min_run() {
    let bytes = std::fs::read("/tmp/min.wasm").unwrap();
    match ap_host::run(&bytes, 64 * 1024) {
        Ok(o) => println!("MINOUT={:?} rc={}", o.stdout, o.exit_code),
        Err(e) => println!("MINERR={}", e),
    }
}
