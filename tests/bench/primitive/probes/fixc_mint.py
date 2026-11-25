"""Fix-C container-mint probe: json.loads dict-heavy shape + Gc::new/registry floor.
Usage: <interp> fixc_mint.py [scale]
scale=1 -> ~16000 records (P10 shape); scale=0 -> small quick pass.
"""
import sys, time, json, gc

def build_corpus(records):
    # P10 record-heavy shape: dicts of 3 lists of 3 strings + int fields
    return [{"id": i, "name": "row%d" % i, "tags": ["a%d" % i, "b%d" % i, "c%d" % i],
             "vals": [i, i + 1, i + 2], "meta": ["x", "y", "z"]}
            for i in range(records)]

def timeit(fn):
    best = None
    for _ in range(3):
        t0 = time.perf_counter()
        fn()
        dt = time.perf_counter() - t0
        if best is None or dt < best:
            best = dt
    return best

def main():
    records = 16000 if (len(sys.argv) < 2 or sys.argv[1] != "0") else 2000
    big = build_corpus(records)
    payload = json.dumps(big)
    n = len(payload)

    t_loads = timeit(lambda: json.loads(payload))

    # container-mint floor: dict/list mints in a loop (no json)
    def mint_dicts():
        out = []
        for i in range(records):
            out.append({"a": [i, i+1, i+2], "b": ["x", "y", "z"]})
        return out
    t_mint = timeit(mint_dicts)

    # Gc::new floor: list mints alone
    def mint_lists():
        out = []
        for i in range(records):
            out.append([i, i+1, i+2])
        return out
    t_list = timeit(mint_lists)

    # dict getitem loop over int keys (P09 shape)
    d = {}
    for i in range(32768):
        d[i] = i
    ks = list(d.keys())
    def getitem_loop():
        c = 0
        for k in ks:
            c += d[k]
        return c
    t_getitem = timeit(getitem_loop)

    # dict build loop (dictcomp equivalent)
    def dict_build():
        d2 = {}
        for k in ks:
            d2[k] = k
        return d2
    t_dictbuild = timeit(dict_build)

    print(json.dumps({"records": records, "payload_bytes": n,
                      "loads_s": t_loads, "mint_dicts_s": t_mint,
                      "mint_lists_s": t_list, "getitem_32k_s": t_getitem,
                      "dictbuild_32k_s": t_dictbuild}), flush=True)

main()
