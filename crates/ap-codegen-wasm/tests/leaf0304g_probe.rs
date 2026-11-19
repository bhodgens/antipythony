// 03.04g discriminators: does the close-chain no-yield arm run at all?
// Each case prints a DIFFERENT line per hypothesized path, so the observed
// output names the broken link without any stepping.
#[test]
fn leaf0304g_probe() {
    let cases: &[(&str, &str)] = &[
    // d1: no-yield handler SWAPS the exception. Arm runs -> 'V from-handler';
    // match fails -> KeyError escapes -> 'K escaped'; tt misses -> crash.
    (
        "d1-swap",
        "def h():\n    try:\n        yield 0\n        raise KeyError('b')\n    except KeyError:\n        raise ValueError('from-handler')\ntry:\n    for v in h():\n        print('B', v)\nexcept ValueError as e:\n    print('V', e)\nexcept KeyError:\n    print('K escaped')\n",
    ),
    // d2: no-yield handler SWALLOWS (pass). Arm runs -> gen finishes -> 'after';
    // match fails -> KeyError escapes -> crash/traceback.
    (
        "d2-swallow",
        "def h():\n    try:\n        yield 0\n        raise KeyError('b')\n    except KeyError:\n        pass\nfor v in h():\n    print('B', v)\nprint('after')\n",
    ),
    // d3: close-time no-yield handler swaps GE for ValueError.
    // Arm runs -> 'noisy caught'; match fails -> silent close -> 'closed ok'.
    (
        "d3-close-swap",
        "def c():\n    try:\n        yield 1\n    except GeneratorExit:\n        raise ValueError('noisy')\nc = c()\nnext(c)\ntry:\n    c.close()\n    print('closed ok')\nexcept ValueError as e:\n    print('noisy caught', e)\n",
    ),
    // d4: YfNext SI arm — after `yield from`, raise instead of print.
    // Inline-bind+trampoline works -> 'V 42'; SI escapes -> 'SI'.
    (
        "d4-yf-after",
        "def sub():\n    yield 1\n    return 42\ndef delegate():\n    r = yield from sub()\n    raise ValueError(r)\nd = delegate()\nnext(d)\ntry:\n    next(d)\nexcept ValueError as e:\n    print('V', e)\nexcept StopIteration:\n    print('SI')\n",
    ),
    // d5: gen state across next(): gen = (x for x in range(4)); next; list.
    (
        "d5-gen-state",
        "def g():\n    yield 1\n    yield 2\ngen = g()\nprint(next(gen))\nprint(list(gen))\n",
    ),
    ];
    for (name, src) in cases {
        println!("=== {name} ===");
        let (module, _) = match ap_parse::lower_source(src) {
            Ok(m) => m,
            Err(e) => {
                println!("PARSE-ERR {e:?}");
                continue;
            }
        };
        let bytes = match ap_codegen_wasm::lower_module_ex(&module, Some(name)) {
            Ok(b) => b,
            Err(e) => {
                println!("LOWER-ERR {e:?}");
                continue;
            }
        };
        if std::env::var("PROBE_DUMP").is_ok() {
            std::fs::write(format!("/tmp/{name}.wasm"), &bytes).unwrap();
        }
        {
            use wasmparser::Validator;
            let feats = wasmparser::WasmFeatures::default()
                | wasmparser::WasmFeatures::GC
                | wasmparser::WasmFeatures::FUNCTION_REFERENCES
                | wasmparser::WasmFeatures::EXCEPTIONS;
            if let Err(e) = Validator::new_with_features(feats).validate_all(&bytes) {
                println!("VALIDATE-ERR {e}");
                continue;
            }
        }
        match ap_host::run(&bytes, 256 * 1024) {
            Ok(out) => {
                println!("stdout {:?}", out.stdout);
                if !out.stderr.is_empty() {
                    println!("stderr {:?}", out.stderr);
                }
                println!("exit {}", out.exit_code);
            }
            Err(e) => println!("HOST-ERR {e:?}"),
        }
    }
}
