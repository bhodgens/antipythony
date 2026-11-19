// 03.04f bisect: GE003/GE004 minimal shapes, one per PROBE_CASE pick.
#[test]
fn ge0304f_bisect() {
    let cases: &[(&str, &str)] = &[
        // GE004 bisect chain (no hasattr/getattr — unsupported builtins trap on NULL closure)
        (
            "f1-sub",
            "def sub():\n    yield 1\n    return 42\ns = sub()\nprint(next(s))\ntry:\n    next(s)\nexcept StopIteration:\n    print('si done')\n",
        ),
        (
            "f2-yf-nobind",
            "def sub():\n    yield 1\n    yield 2\n    return 42\ndef delegate():\n    yield from sub()\nd = delegate()\nprint(next(d))\nprint(next(d))\nprint(next(d, 'done'))\n",
        ),
        (
            "f3-yf-bind",
            "def sub():\n    yield 1\n    yield 2\n    return 42\ndef delegate():\n    r = yield from sub()\n    print('r', r)\nd = delegate()\nprint(next(d))\nprint(next(d))\nprint(next(d, 'done'))\n",
        ),
        (
            "f4-yf-for",
            "def sub():\n    yield 1\n    yield 2\n    return 42\ndef delegate():\n    r = yield from sub()\n    print('r', r)\n    yield 99\nfor v in delegate():\n    print('D', v)\n",
        ),
        (
            "f5-yf-listlit",
            "def deep():\n    yield from [10, 20]\nprint(list(deep()))\n",
        ),
        // GE003 bisect chain
        (
            "g1-try-nonyield",
            "def h():\n    try:\n        yield 0\n    except KeyError:\n        yield 'r'\nit = h()\nprint(next(it))\nprint(next(it, 'ex'))\n",
        ),
        (
            "g1b-try-for",
            "def h():\n    try:\n        yield 0\n    except KeyError:\n        yield 'r'\nfor v in h():\n    print('B', v)\n",
        ),
        (
            "g2-try-raise",
            "def h():\n    try:\n        yield 0\n        raise KeyError('b')\n    except KeyError:\n        yield 'r'\nfor v in h():\n    print('B', v)\n",
        ),
        (
            "g3-try-post",
            "def h():\n    try:\n        yield 0\n        raise KeyError('b')\n    except KeyError:\n        yield 'r'\n    yield 'end'\nfor v in h():\n    print('B', v)\n",
        ),
        (
            "g4-raisemid",
            "def raiser():\n    yield 1\n    raise ValueError('mid')\n    yield 2\nit = raiser()\nprint(next(it))\ntry:\n    next(it)\nexcept ValueError as e:\n    print('caught', e)\n",
        ),
        (
            "g5-close",
            "def closer():\n    try:\n        yield 1\n        yield 2\n    except GeneratorExit:\n        print('closing')\n        raise\nc = closer()\nprint(next(c))\nc.close()\nprint('closed ok')\n",
        ),
        (
            "g7-firstnext",
            "def h():\n    try:\n        yield 0\n    except KeyError:\n        yield 'r'\nit = h()\nprint(next(it))\nprint('mid')\n",
        ),
        (
            "g8-park-resume",
            "def h():\n    try:\n        yield 0\n        yield 1\n    except KeyError:\n        yield 'r'\nit = h()\nprint(next(it))\nprint(next(it))\nprint('end')\n",
        ),
        (
            "g9-nologic",
            "def h():\n    yield 0\nit = h()\nprint(next(it))\nprint(next(it, 'ex'))\n",
        ),
        (
            "g10-marks",
            "def h():\n    try:\n        yield 0\n    except KeyError:\n        yield 'r'\nit = h()\nprint('A', next(it))\nprint('B', next(it, 'EX'))\nprint('C')\n",
        ),
        (
            "g2b-handler-mark",
            "def h():\n    try:\n        yield 0\n        raise KeyError('b')\n    except KeyError:\n        print('H')\n        yield 'r'\nfor v in h():\n    print('B', v)\n",
        ),
        (
            "g2c-bare",
            "def h():\n    try:\n        yield 0\n        raise KeyError('b')\n    except:\n        print('H')\n        yield 'r'\nfor v in h():\n    print('B', v)\n",
        ),
        (
            "g11-firstraise",
            "def h():\n    try:\n        raise KeyError('x')\n    except KeyError:\n        yield 'first'\nfor v in h():\n    print('B', v)\n",
        ),
        (
            "g2d-noyield-h",
            "def h():\n    try:\n        yield 0\n        raise KeyError('b')\n    except KeyError:\n        print('H')\nfor v in h():\n    print('B', v)\n",
        ),
        (
            "g5b-baseexc",
            "def c():\n    try:\n        yield 1\n    except BaseException:\n        print('caught-in-gen')\nc = c()\nnext(c)\nc.close()\nprint('after')\n",
        ),
        (
            "f6-yf-seqs",
            "def sub():\n    yield 1\n    yield 2\n    return 42\ndef delegate():\n    r = yield from sub()\n    print('r', r)\n    print('pre99')\n    yield 99\nfor v in delegate():\n    print('D', v)\n",
        ),
        (
            "e1-comp-prefix",
            "squares = [x * x for x in range(5)]\nprint(squares)\nevens = [x for x in range(10) if x % 2 == 0]\nprint(evens)\npair = [y for x in [(1, 2), (3, 4)] for y in x]\nprint(pair)\n",
        ),
        (
            "e2-walrus",
            "w = [y := x * 10 for x in range(3)]\nprint(w, y)\n",
        ),
        (
            "e3-genexp-next",
            "gen = (x for x in range(4))\nprint(next(gen))\nprint(list(gen))\n",
        ),
        (
            "g6-prethrow",
            "def pre():\n    yield 1\np = pre()\ntry:\n    p.throw(ValueError('pre-start'))\nexcept ValueError as e:\n    print('pre-start throw', e)\n",
        ),
    ];
    let pick = std::env::var("PROBE_CASE").unwrap_or_default();
    for (name, src) in cases {
        if !pick.is_empty() && *name != pick {
            continue;
        }
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
