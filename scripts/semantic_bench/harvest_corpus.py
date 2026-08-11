#!/usr/bin/env python3
"""Harvest a real corpus of Mnema anchor text for the offline embedding bake-off (issue #191).

Talks to the brokered `mnema mcp` stdio server over JSON-RPC:
  1. discovery -- `search` across a sweep of broad queries x time windows (the
     search tool has no cursor, so breadth comes from varying query + from/to);
  2. fetch     -- `show_text` for every unique opaque result id, in parallel.

Writes <out>.jsonl (one anchor per line) and <out>_stats.json next to it.

THE OUTPUT IS THE USER'S PERSONAL CAPTURE DATA. Never commit it, never write it
inside the repo -- point --out at a scratch directory outside the working tree.

Usage: harvest_corpus.py --out /tmp/scratch/corpus.jsonl --target 4000
"""

import argparse
import collections
import concurrent.futures
import datetime
import hashlib
import json
import os
import subprocess
import threading

QUERIES = [
    "the", "and", "that", "with", "you", "this", "for", "have", "what", "from",
    "code", "time", "one", "work", "about", "when", "they", "would", "think",
    "people", "new", "use", "like", "know", "just", "because", "there", "make",
    "yeah", "okay", "really", "actually", "going", "right", "well", "want",
    "file", "test", "data", "error", "search", "model", "user", "run", "build",
]


class Mcp:
    """One `mnema mcp` child process. Not thread-safe -- give each thread its own."""

    def __init__(self):
        self.p = subprocess.Popen(
            ["mnema", "mcp"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1,
        )
        self.n = 0
        self._rpc("initialize", {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "semantic-bench-harvest", "version": "0"},
        })
        self._notify("notifications/initialized")

    def _notify(self, method):
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method}) + "\n")
        self.p.stdin.flush()

    def _rpc(self, method, params):
        self.n += 1
        want = self.n
        self.p.stdin.write(json.dumps(
            {"jsonrpc": "2.0", "id": want, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("mnema mcp closed stdout")
            try:
                msg = json.loads(line)
            except ValueError:
                continue  # ponytail: the server occasionally logs to stdout; ignore non-JSON
            if msg.get("id") == want:
                return msg

    def tool(self, name, args):
        msg = self._rpc("tools/call", {"name": name, "arguments": args})
        if "error" in msg:
            raise RuntimeError(msg["error"])
        return json.loads(msg["result"]["content"][0]["text"])

    def close(self):
        try:
            self.p.stdin.close()
            self.p.wait(timeout=5)
        except Exception:
            self.p.kill()


_local = threading.local()
_clients = []
_clients_lock = threading.Lock()


def client():
    c = getattr(_local, "mcp", None)
    if c is None:
        c = _local.mcp = Mcp()
        with _clients_lock:
            _clients.append(c)
    return c


def rfc3339(d):
    return d.strftime("%Y-%m-%dT%H:%M:%SZ")


def probe_days(mcp):
    """Earliest/latest day that has any data, from a few unbounded searches."""
    days = set()
    for q in ("the", "and", "you"):
        for r in mcp.tool("search", {"query": q, "limit": 100})["results"]:
            days.add(r["startedAt"][:10])
    lo = datetime.date.fromisoformat(min(days))
    hi = datetime.date.fromisoformat(max(days)) + datetime.timedelta(days=1)
    return [lo + datetime.timedelta(days=i) for i in range((hi - lo).days + 1)]


def windows(days):
    """Whole-day windows first (cheap breadth), then 6-hour windows."""
    day = [(rfc3339(datetime.datetime.combine(d, datetime.time())),
            rfc3339(datetime.datetime.combine(d + datetime.timedelta(days=1), datetime.time())))
           for d in days]
    six = []
    for d in days:
        base = datetime.datetime.combine(d, datetime.time())
        for h in (0, 6, 12, 18):
            six.append((rfc3339(base + datetime.timedelta(hours=h)),
                        rfc3339(base + datetime.timedelta(hours=h + 6))))
    return day + six


def discover(tasks, target, workers):
    """Run search tasks in order until `target` unique ids are known."""
    found = {}
    lock = threading.Lock()

    def one(task):
        q, (a, b) = task
        try:
            res = client().tool("search", {"query": q, "limit": 100, "from": a, "to": b})
        except Exception as e:
            print("search failed", q, a, e)
            return
        with lock:
            for r in res["results"]:
                found.setdefault(r["id"], r)

    with concurrent.futures.ThreadPoolExecutor(workers) as pool:
        for i in range(0, len(tasks), workers * 4):
            batch = tasks[i:i + workers * 4]
            list(pool.map(one, batch))
            print("discovered %d unique ids (%d/%d searches)" % (len(found), i + len(batch), len(tasks)))
            if len(found) >= target:
                break
    return found


def fetch(found, out_path, min_chars, workers):
    """show_text every id, drop short ones, write jsonl."""
    lock = threading.Lock()
    kept, short, failed = [], 0, 0

    def one(item):
        rid, meta = item
        try:
            return rid, meta, client().tool("show_text", {"opaque_result_id": rid})
        except Exception:
            return rid, meta, None

    with open(out_path, "w") as f, concurrent.futures.ThreadPoolExecutor(workers) as pool:
        for rid, meta, doc in pool.map(one, list(found.items())):
            if doc is None:
                with lock:
                    failed += 1
                continue
            text = doc.get("text") or ""
            if len(text) < min_chars:
                with lock:
                    short += 1
                continue
            ctx = meta.get("context") or {}
            rec = {
                "id": rid,
                "kind": doc.get("kind") or meta.get("kind"),
                "text": text,
                "startedAt": meta.get("startedAt"),
                "endedAt": meta.get("endedAt"),
                "app": ctx.get("appName"),
                "windowTitle": ctx.get("windowTitle"),
                "url": ctx.get("url"),
                "textSha1": hashlib.sha1(text.encode()).hexdigest(),
            }
            with lock:
                f.write(json.dumps(rec) + "\n")
                kept.append(rec)
    print("kept %d, dropped %d short (<%d chars), %d failed" % (len(kept), short, min_chars, failed))
    return kept


def pct(xs, p):
    if not xs:
        return 0
    s = sorted(xs)
    return s[min(len(s) - 1, int(round(p / 100.0 * (len(s) - 1))))]


def stats(kept):
    by_kind = collections.Counter(r["kind"] for r in kept)
    lens = collections.defaultdict(list)
    for r in kept:
        lens[r["kind"]].append(len(r["text"]))
    sha = collections.Counter(r["textSha1"] for r in kept)
    dup_groups = {h: c for h, c in sha.items() if c > 1}
    return {
        "total": len(kept),
        "byKind": dict(by_kind),
        "byApp": dict(collections.Counter(r["app"] or "(none)" for r in kept).most_common(20)),
        "dateRange": [min((r["startedAt"] for r in kept), default=None),
                      max((r["startedAt"] for r in kept), default=None)],
        "charLength": {
            k: {"p10": pct(v, 10), "p50": pct(v, 50), "p90": pct(v, 90), "max": max(v)}
            for k, v in lens.items()
        },
        "approxTokens": {
            k: {
                "p10": pct(v, 10) // 4, "p50": pct(v, 50) // 4,
                "p90": pct(v, 90) // 4, "max": max(v) // 4,
                "fractionOver256": round(sum(1 for c in v if c / 4.0 > 256) / float(len(v)), 4),
                "fractionOver512": round(sum(1 for c in v if c / 4.0 > 512) / float(len(v)), 4),
            }
            for k, v in lens.items()
        },
        "exactDuplicateText": {
            "groups": len(dup_groups),
            "anchorsInDuplicateGroups": sum(dup_groups.values()),
        },
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True, help="output .jsonl path (outside the repo!)")
    ap.add_argument("--target", type=int, default=4000)
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--min-chars", type=int, default=120)
    args = ap.parse_args()

    # over-discover: some ids get dropped by the min-chars filter
    want = int(args.target * 1.3)
    days = probe_days(client())
    print("data days:", days[0], "->", days[-1])
    tasks = [(q, w) for q in QUERIES for w in windows(days)]
    found = discover(tasks, want, args.workers)
    kept = fetch(found, args.out, args.min_chars, args.workers)
    st = stats(kept)
    stats_path = os.path.splitext(args.out)[0] + "_stats.json"
    with open(stats_path, "w") as f:
        json.dump(st, f, indent=2)
    print(json.dumps(st, indent=2))
    for c in _clients:
        c.close()


if __name__ == "__main__":
    main()
