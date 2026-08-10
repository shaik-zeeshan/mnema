"""Paired bootstrap + sign test on per-query nDCG@10, every candidate vs a
baseline model (default nomic; pass another model name to compare tiers, e.g.
`significance.py results.json e5-small-ml` for the multilingual tier)."""
import json, random, sys, statistics as st
r = json.load(open(sys.argv[1]))
baseline = sys.argv[2] if len(sys.argv) > 2 else "nomic"
by = {m["model"]: {q["id"]: q for q in m["per_query"]} for m in r["results"]}
base = by[baseline]; ids = list(base)
random.seed(7)
print(f"corpus={r['corpus_size']} queries={r['query_count']} baseline={baseline}\n")
for name, cur in by.items():
    if name == baseline: continue
    for scope in ("all", "screenText", "audioTranscript"):
        sub = [i for i in ids if scope == "all" or base[i]["kind"] == scope]
        d = [cur[i]["ndcg@10"] - base[i]["ndcg@10"] for i in sub]
        if not d:
            # A single-kind query set (screen-only or audio-only) has no rows for the
            # other scope. Without this the division below raises mid-report, after
            # some rows have already printed — so the median-rank line and every
            # REMAINING model are silently lost behind a traceback that looks like a
            # finished table.
            print(f"{name:15s} {scope:15s} n=  0 (no queries of this kind)")
            continue
        mean = sum(d)/len(d)
        boots = sorted(sum(random.choices(d, k=len(d)))/len(d) for _ in range(5000))
        lo, hi = boots[125], boots[4874]
        win = sum(1 for x in d if x > 0); loss = sum(1 for x in d if x < 0)
        print(f"{name:15s} {scope:15s} n={len(sub):3d} dNDCG={mean:+.4f} "
              f"95%CI=[{lo:+.4f},{hi:+.4f}] {'SIG' if lo>0 or hi<0 else 'ns '} "
              f"win/loss/tie={win}/{loss}/{len(d)-win-loss}")
    # rank-of-source median (how deep the true doc sits)
    mr = st.median([cur[i]["rank_of_source"] for i in ids])
    mb = st.median([base[i]["rank_of_source"] for i in ids])
    print(f"{'':15s} median rank of true anchor: {name}={mr:.0f} vs {baseline}={mb:.0f}\n")
