//! `ap-gc-eval` — Fix-C raw container-mint microbench (temporary probe bin).
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, Value};
use std::time::Instant;

fn bench(n: usize, label: &str, keep_alive: bool) {
    let before = ap_gc::stats();
    let mut keep: Vec<Gc<PyList>> = Vec::with_capacity(if keep_alive { n } else { 0 });
    let t0 = Instant::now();
    for i in 0..n {
        let l = PyList::from_vec(vec![
            Value::Int(Int::Smi(i as i64)),
            Value::Int(Int::Smi((i + 1) as i64)),
            Value::Int(Int::Smi((i + 2) as i64)),
        ]);
        if keep_alive {
            keep.push(l);
        }
    }
    let dt = t0.elapsed();
    let after = ap_gc::stats();
    eprintln!(
        "[{} END] minted={} hold_len={} tracked={} collections={} reclaimed={}",
        label,
        n,
        keep.len(),
        ap_gc::gc_tracked_len(),
        after.collections - before.collections,
        after.objects_reclaimed - before.objects_reclaimed
    );
    println!(
        "{{\"label\": \"{}\", \"n\": {}, \"per_mint_ns\": {}}}",
        label,
        n,
        dt.as_nanos() as f64 / n as f64
    );
}

fn main() {
    ap_obj::gc_gate::set_hook(|v| ap_gc::gc_register(v));
    bench(160_000, "A_raw_keep", true);
    bench(160_000, "A_raw_drop", false);
    bench(160_000, "A_raw_keep2", true);
    let _ = std::hint::black_box(());
}
