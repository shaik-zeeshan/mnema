"""Deterministically sample query-source anchors, stratified by kind, avoiding near-dups."""
import json, random, sys, re
from pathlib import Path
COR = Path("corpus.jsonl"); OUT = Path("query_sources")
def grams(t, n=5):
    t = re.sub(r"\s+", " ", t.lower())
    return {t[i:i+n] for i in range(0, max(0, len(t)-n+1), 3)}
rows = [json.loads(l) for l in COR.open()]
random.seed(1901)
picked, sig = [], []
for kind, want in (("screenText", 100), ("audioTranscript", 100)):
    pool = [r for r in rows if r["kind"] == kind and len(r["text"]) >= 300]
    random.shuffle(pool)
    got = 0
    for r in pool:
        if got >= want: break
        g = grams(r["text"])
        if any(len(g & s) / max(1, len(g | s)) >= 0.35 for s in sig): continue
        sig.append(g); picked.append(r); got += 1
    print(kind, "pool", len(pool), "picked", got)
OUT.mkdir(exist_ok=True)
random.shuffle(picked)
chunks = 4
for i in range(chunks):
    part = picked[i::chunks]
    (OUT / f"sources_{i}.json").write_text(json.dumps(part, indent=1))
    print(f"sources_{i}.json", len(part))
