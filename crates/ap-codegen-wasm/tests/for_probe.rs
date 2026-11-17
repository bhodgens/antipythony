// E2E probe: For-over-range / For-over-list / next-default (03.04c B slice).
#[test]
fn for_probe() {
    for (name, src) in [
        ("for-range", "for j in range(2):\n    print('j', j)\nprint('done')\n"),
        ("for-list", "for v in [10, 20, 30]:\n    print('v', v)\nprint('done')\n"),
        ("for-tuple-else", "for w in (1, 2):\n    print('w', w)\nprint('done')\n"),
        ("for-range-sum", "total = 0\nfor i in range(3):\n    total = total + i\nprint(total)\n"),
        ("for-break", "for i in range(5):\n    if i == 2:\n        break\n    print(i)\nprint('end')\n"),
    ] {
        let (module, _) = ap_parse::lower_source(src).expect("parse");
        let bytes = ap_codegen_wasm::lower_module(&module).expect("lower");
        match ap_host::run(&bytes, 256 * 1024) {
            Ok(out) => println!(
                "{name}: rc={} stdout={:?} stderr={:?}",
                out.exit_code, out.stdout, out.stderr
            ),
            Err(e) => println!("{name}: HOST {e:#.200}"),
        }
    }
}
