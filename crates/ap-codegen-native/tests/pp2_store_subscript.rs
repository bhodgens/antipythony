//! pp2-tier1 bridge part 2: Subscript STORE (`d[i] = v`) was Unsupported in
//! cfg::lower_fn's assign_target — the dict.build loop shape (`d[i] = i`)
//! never reached ANY compiled path (lower-reject → pure tree-walk fallback,
//! compiles=0). This adds Inst::StoreSubscript + the h_set_subscript
//! runtime helper + the emit arm, mirroring h_subscript's host-fn pattern
//! (SUBSCRIPT_SET_FN installed by ap-run alongside SUBSCRIPT_GET_FN).
//!
//! Boxed-protocol only: never inside a typed region (dict traffic is
//! unboxable), so tier-1 declines the block but the BASELINE TWIN (worth
//! 1.2-1.7x per docs/tier1-bench.md) now covers the shape.

use ap_codegen_native::cfg;
use ap_codegen_native::clif::{compile_baseline, compile_baseline_with_osr};
use ap_codegen_native::runtime::{h_set_subscript, set_subscript_set_fn, ExecCtx};
use ap_parse::lowering::lower_source;

const DICT_BUILD: &str = r#"
def probe():
    d = {}
    i = 0
    while i < 64:
        d[i] = i
        i = i + 1
    return len(d)
"#;

fn lowered(src: &str) -> Result<cfg::Func, String> {
    let (module, _scope) = lower_source(src).map_err(|e| format!("lower_source: {e:?}"))?;
    let fdef = module
        .items
        .iter()
        .find_map(|i| match i {
            ap_ir::Item::FnDef(ff) if ff.name == "probe" => Some(ff),
            _ => None,
        })
        .ok_or("no fn probe")?;
    cfg::lower_fn(fdef).map_err(|e| format!("lower_fn: {e:?}"))
}

#[test]
fn dict_build_loop_lower_and_compile() {
    let func = lowered(DICT_BUILD).expect("Subscript store must lower");
    compile_baseline(&func).expect("baseline must compile");
    compile_baseline_with_osr(&func, 42).expect("baseline-with-osr must compile");
}

#[test]
fn set_subscript_helper_fails_closed_without_host() {
    // Unit context: no host fn installed (ap-run installs the real one via
    // set_subscript_set_fn at the JIT seam). The helper must fail-closed
    // with a non-zero HelperStatus (never a silent no-op store).
    set_subscript_set_fn(None);
    let host = ap_codegen_native::runtime::HostCallbacks {
        load_global: |_c, _s, _n, _d| Err(ap_obj::PyException::new("SystemError", "no host")),
        call: |_c, _s, _cs, _a, _ac, _d| Err(ap_obj::PyException::new("SystemError", "no host")),
        load_attr: |_c, _s, _o, _n, _d| Err(ap_obj::PyException::new("SystemError", "no host")),
        store_attr: |_c, _s, _n, _v| Err(ap_obj::PyException::new("SystemError", "no host")),
    };
    let mut ctx = ExecCtx::new(4, host);
    let rc = h_set_subscript(&mut ctx, 0, 1, 2);
    assert_ne!(rc, 0, "setitem without a host fn must fail-closed");
}
