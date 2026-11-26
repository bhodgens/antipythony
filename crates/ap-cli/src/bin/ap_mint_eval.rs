#![forbid(unsafe_code)] // issue #18 wave 3 ratchet: measured zero-unsafe target (scripts/unsafe_report.sh + clippy undocumented_unsafe_blocks = 0)
//! `ap-mint-eval` — hp-lazydunders raw mint microbench (temporary probe bin).
//! Decomposes the PyObject (BoundMethod) mint floor:
//!   A_list:      PyList mint alone (registry + GC amortized)
//!   B_pyobject:  PyObject::new alone (attrs dict + dunders + ~15 locks)
//!   C_bound:     full BoundMethod mint via with_self (the per-attr-read cost)
//!   D_setget:    dict set+get probe (interned-key table cost reference)
use ap_obj::int::{Gc, Int};
use ap_obj::object::{ObjKind, PyObject};
use ap_obj::value::{PyList, PyString, Value};
use std::time::Instant;

fn bench(n: usize, label: &str, f: impl Fn(usize)) {
    let before = ap_gc::stats();
    let t0 = Instant::now();
    for i in 0..n {
        f(i);
    }
    let dt = t0.elapsed();
    let after = ap_gc::stats();
    eprintln!(
        "[{} END] tracked={} collections={} reclaimed={}",
        label,
        ap_gc::gc_tracked_len(),
        after.collections - before.collections,
        after.objects_reclaimed - before.objects_reclaimed
    );
    println!(
        "{{\"label\": \"{}\", \"n\": {}, \"per_op_ns\": {}}}",
        label,
        n,
        dt.as_nanos() as f64 / n as f64
    );
}

fn main() {
    ap_obj::gc_gate::set_hook(|v| ap_gc::gc_register(v));
    let n = 160_000;
    // Warmup + hold one PyObject so the registry shape is realistic.
    let hold = PyObject::new(ObjKind::BoundMethod, "warm");
    bench(n, "A_list", |i| {
        let l = PyList::from_vec(vec![
            Value::Int(Int::Smi(i as i64)),
            Value::Int(Int::Smi((i + 1) as i64)),
        ]);
        std::hint::black_box(&l);
    });
    bench(n, "B_pyobject", |i| {
        let o = PyObject::new(ObjKind::BoundMethod, "method");
        std::hint::black_box(&o);
    });
    let recv = Value::Int(Int::Smi(7));
    bench(n, "C_bound", |_i| {
        let o = PyObject::with_self(ObjKind::BoundMethod, "append", recv.clone());
        std::hint::black_box(&o);
    });
    // Reference: one interned-key dict set+get (the fast-path table shape).
    let table = ap_obj::dict::PyDict::new();
    bench(n, "D_dict_setget", |i| {
        let k = Value::Str(PyString::new("append"));
        let _ = table.set(k.clone(), Value::Int(Int::Smi(i as i64)));
        let _ = table.get(&k);
    });
    bench(n, "E_dict_get_only", |_i| {
        let k = Value::Str(PyString::new("append"));
        let _ = table.get(&k);
    });
    let _ = hold;
    let _ = std::hint::black_box(());
}
