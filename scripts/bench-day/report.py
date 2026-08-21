#!/usr/bin/env python3
"""Turn a bench-day run directory into the rows the transparency page needs.

    python3 scripts/bench-day/report.py ~/mnema-bench-runs/defaults-20260818-0900

Prints markdown: the method-of-record header (hardware, build, the settings that
were actually in force) followed by measured rows with dispersion and sample
counts. Nothing is inferred or extrapolated silently -- a value we could not
parse is printed as `-`, and unparsed input lines are counted out loud, because
a quietly-zeroed row is worse than a missing one.
"""

from __future__ import annotations

import json
import re
import statistics
import sys
from pathlib import Path

# ---------------------------------------------------------------- parsing

SAMPLE_RE = re.compile(r"^\*\*\* Sampled system activity \((?P<when>.+?)\) \((?P<ms>[\d.]+)ms elapsed\)")
POWER_RE = re.compile(r"^(?P<key>[A-Za-z][A-Za-z ()+/-]*?Power[A-Za-z ()+/-]*?):\s*(?P<val>[\d.]+)\s*(?P<unit>mW|W)\b")
RESIDENCY_RE = re.compile(r"^GPU HW active residency:\s*(?P<val>[\d.]+)%")
FREQ_RE = re.compile(r"^GPU HW active frequency:\s*(?P<val>[\d.]+)\s*MHz")
TEMP_RE = re.compile(r"^(?P<key>[A-Za-z]+ die temperature):\s*(?P<val>[\d.]+)")
COALITION_HDR_RE = re.compile(r"^\s*Name\s+ID\s")
PROC_AVG_RE = re.compile(r"^\[\s*\d+\s*\]")
DISKIO_RE = re.compile(r"^(?P<dir>read|write):\s*(?P<ops>[\d.]+)\s*ops/s\s*(?P<kbs>[\d.]+)\s*KBytes/s")
# name (may contain spaces) then the integer coalition id then numeric columns
COALITION_RE = re.compile(r"^(?P<name>\S.*?)\s+(?P<id>\d+)\s+(?P<rest>[\d.,%()\s+-]+)$")

WATCH = ("mnema", "WindowServer", "mediaanalysisd", "speech", "Speech", "WebContent", "powermetrics")


def _floats(text: str) -> list[float]:
    return [float(t) for t in re.findall(r"\d+\.\d+|\d+", text)]


def parse_power_log(path: Path) -> tuple[dict[str, list[float]], int, int]:
    """-> (series, sample_count, unparsed_coalition_lines)"""
    series: dict[str, list[float]] = {}
    samples = 0
    unparsed = 0

    def add(key: str, value: float) -> None:
        series.setdefault(key, []).append(value)

    for raw in path.read_text(errors="replace").splitlines():
        line = raw.strip()
        if not line:
            continue
        if SAMPLE_RE.match(line):
            samples += 1
            continue
        m = POWER_RE.match(line)
        if m:
            val = float(m.group("val"))
            if m.group("unit") == "W":
                val *= 1000.0
            add("power:" + m.group("key").strip(), val)
            continue
        m = DISKIO_RE.match(line)
        if m:
            add(f"diskio:{m.group('dir')} KB/s", float(m.group("kbs")))
            add(f"diskio:{m.group('dir')} ops/s", float(m.group("ops")))
            continue
        m = RESIDENCY_RE.match(line)
        if m:
            add("gpu_residency_pct", float(m.group("val")))
            continue
        m = FREQ_RE.match(line)
        if m:
            add("gpu_freq_mhz", float(m.group("val")))
            continue
        m = TEMP_RE.match(line)
        if m:
            add("temp:" + m.group("key"), float(m.group("val")))
            continue
        if COALITION_HDR_RE.match(raw):
            continue
        # `[pid]name  <numbers>  <calculating>` — powermetrics' periodic per-process
        # energy average, not a coalition row. Skipped explicitly so the unparsed
        # counter keeps meaning "real parse failure"; a counter that always shows
        # noise is a counter nobody reads.
        if PROC_AVG_RE.match(line):
            continue
        if any(w in line for w in WATCH):
            m = COALITION_RE.match(line)
            if not m:
                unparsed += 1
                continue
            nums = _floats(m.group("rest"))
            if not nums:
                unparsed += 1
                continue
            name = m.group("name").strip()
            # Verified against real output on macOS 26.5.2. Columns are:
            #   Name  ID  CPU ms/s  User%  Deadlines(<2ms, 2-5ms)  Wakeups(Intr, Pkg idle)  GPU ms/s  Energy Impact
            # Coalition rows leave User%/Deadlines BLANK while child process rows
            # fill them, so only the ends are reliable: CPU ms/s is first, and
            # Energy Impact is last with GPU ms/s immediately before it. Reading
            # the last column as GPU ms/s — as this parser originally did — silently
            # reports Energy Impact as GPU time.
            add(f"cpu_ms_s:{name}", nums[0])
            if len(nums) >= 3:
                add(f"gpu_ms_s:{name}", nums[-2])
                add(f"energy_impact:{name}", nums[-1])
    return series, samples, unparsed


MACMON_RAILS = [
    ("sys_power", "system total (all rails)", 1000.0),
    ("all_power", "CPU + GPU + ANE", 1000.0),
    ("cpu_power", "CPU", 1000.0),
    ("gpu_power", "GPU", 1000.0),
    ("ane_power", "ANE", 1000.0),
    ("ram_power", "RAM", 1000.0),
    ("gpu_ram_power", "GPU RAM", 1000.0),
]


def parse_macmon(path: Path) -> tuple[dict[str, list[float]], int, int]:
    """macmon `pipe` NDJSON -> (series, sample_count, unparsed_lines).

    macmon reports watts; power series are converted to mW so they line up with
    powermetrics. Utilisations arrive as 0..1 fractions and become percentages.
    """
    series: dict[str, list[float]] = {}
    samples = 0
    unparsed = 0

    def add(key: str, value: float) -> None:
        series.setdefault(key, []).append(float(value))

    for line in path.read_text(errors="replace").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            unparsed += 1
            continue
        samples += 1
        for key, label, scale in MACMON_RAILS:
            if isinstance(obj.get(key), (int, float)):
                add(f"mm_power:{label}", obj[key] * scale)
        temp = obj.get("temp") or {}
        for key, label in (("cpu_temp_avg", "CPU die (avg)"), ("gpu_temp_avg", "GPU die (avg)")):
            if isinstance(temp.get(key), (int, float)):
                add(f"mm_temp:{label}", temp[key])
        if isinstance(obj.get("cpu_usage_pct"), (int, float)):
            add("mm_util:CPU total (%)", obj["cpu_usage_pct"] * 100.0)
        for key, label in (("ecpu_usage", "E-cluster"), ("pcpu_usage", "P-cluster"), ("gpu_usage", "GPU")):
            pair = obj.get(key)
            if isinstance(pair, list) and len(pair) == 2:
                add(f"mm_freq:{label} (MHz)", pair[0])
                add(f"mm_util:{label} active (%)", pair[1] * 100.0)
        mem = obj.get("memory") or {}
        if isinstance(mem.get("ram_usage"), (int, float)):
            add("mm_mem:machine RAM used (GB)", mem["ram_usage"] / 1073741824.0)
        if isinstance(mem.get("swap_usage"), (int, float)):
            add("mm_mem:swap used (GB)", mem["swap_usage"] / 1073741824.0)
    return series, samples, unparsed


def parse_samples(path: Path) -> list[dict[str, str]]:
    rows: list[dict[str, str]] = []
    lines = path.read_text().splitlines()
    if not lines:
        return rows
    header = lines[0].split(",")
    for line in lines[1:]:
        parts = line.split(",")
        if len(parts) != len(header):
            continue
        rows.append(dict(zip(header, parts)))
    return rows


# ---------------------------------------------------------------- stats


def stats(values: list[float]) -> str:
    if not values:
        return "-"
    if len(values) == 1:
        return f"{values[0]:.1f} (n=1)"
    p95 = statistics.quantiles(values, n=20)[-1] if len(values) >= 20 else max(values)
    return (
        f"mean {statistics.mean(values):.1f} · p50 {statistics.median(values):.1f} · "
        f"p95 {p95:.1f} · max {max(values):.1f} · n={len(values)}"
    )


def split_half(values: list[float]) -> str:
    """Did this metric settle, or is the run too short to quote it?

    Run 1 showed both failure modes. Cumulative metrics (disk) were still
    climbing at hour 8: the MB/active-hour estimate read 36 at 3 h and 73 at
    8.3 h, so a short run under-reports by up to 50%. Whole-machine rate metrics
    were no better -- CPU p95 was off by 74% at the 3 h mark and only landed
    within 5% by hour 12. A per-coalition rate metric should behave far better,
    because the app's work is periodic (one frame per 2 s, OCR per frame, 60 s
    rotation) rather than bursty like a developer's day, but that has to be
    demonstrated per run rather than assumed.
    """
    if len(values) < 20:
        return "too few samples to judge"
    mid = len(values) // 2
    a, b = values[:mid], values[mid:]
    ma, mb = statistics.mean(a), statistics.mean(b)
    if ma == 0:
        return "first half is zero"
    # Relative drift is meaningless on a series that is essentially zero: an
    # idle daemon wobbling between 0.05 and 0.11 ms/s reads as "109% drift" and
    # trains the reader to ignore the whole column. Anything under 1 ms/s (0.1%
    # of one core) is reported as negligible instead of flagged.
    if max(abs(ma), abs(mb)) < 1.0:
        return f"{ma:.2f} → {mb:.2f} (negligible, below 0.1% of one core)"
    drift = abs(mb - ma) / abs(ma) * 100
    verdict = "stable" if drift < 10 else ("drifting" if drift < 30 else "NOT converged")
    return f"{ma:.1f} → {mb:.1f} ({drift:.0f}% drift, {verdict})"


def slope_per_day(xs: list[float], ys: list[float]) -> float | None:
    """Least-squares slope of ys over xs (seconds), scaled to a day."""
    if len(xs) < 3:
        return None
    mx, my = statistics.mean(xs), statistics.mean(ys)
    denom = sum((x - mx) ** 2 for x in xs)
    if denom == 0:
        return None
    slope = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / denom
    return slope * 86400.0


def num(row: dict[str, str], key: str) -> float | None:
    try:
        return float(row[key])
    except (KeyError, ValueError):
        return None


# ---------------------------------------------------------------- report


SETTINGS_OF_RECORD = [
    ("captureScreen", "screen"),
    ("screenFrameRate", "frame rate (fps)"),
    ("screenResolution", "resolution"),
    ("videoBitrate", "bitrate preset"),
    ("segmentDurationSeconds", "segment (s)"),
    ("captureMicrophone", "microphone"),
    ("captureSystemAudio", "system audio"),
    ("pauseCaptureOnInactivity", "pause on inactivity"),
    ("idleTimeoutSeconds", "idle timeout (s)"),
    ("retentionPolicy", "retention"),
    ("ocr", "OCR"),
    ("transcription", "transcription"),
    ("speakerAnalysis", "speaker analysis"),
    ("semanticSearch", "semantic search"),
]


def flatten(value: object) -> str:
    if isinstance(value, dict):
        keep = {k: v for k, v in value.items() if not isinstance(v, (dict, list)) and v is not None}
        return ", ".join(f"{k}={v}" for k, v in keep.items())
    return str(value)


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    run = Path(sys.argv[1]).expanduser()
    if not run.is_dir():
        print(f"not a run directory: {run}", file=sys.stderr)
        return 1

    meta = (run / "meta.txt").read_text() if (run / "meta.txt").exists() else ""
    series, pm_samples, unparsed = ({}, 0, 0)
    if (run / "power.log").exists():
        series, pm_samples, unparsed = parse_power_log(run / "power.log")
    mm, mm_samples, mm_unparsed = ({}, 0, 0)
    if (run / "power.ndjson").exists():
        mm, mm_samples, mm_unparsed = parse_macmon(run / "power.ndjson")
    rows = parse_samples(run / "samples.csv") if (run / "samples.csv").exists() else []

    print(f"# Bench day — {run.name}\n")
    print("## Method of record\n")
    print("```")
    print(meta.strip())
    print("```\n")

    start = run / "settings-start.json"
    end = run / "settings-end.json"
    if start.exists():
        cfg = json.loads(start.read_text())
        print("### Settings in force (print these next to every number)\n")
        print("| Setting | Value |")
        print("|---|---|")
        for key, label in SETTINGS_OF_RECORD:
            if key in cfg:
                print(f"| {label} | `{flatten(cfg[key])}` |")
        print()
        if end.exists() and end.read_text() != start.read_text():
            print("> **The settings changed during the run.** Diff `settings-start.json` "
                  "against `settings-end.json` before trusting any row below.\n")

    print("## Measured\n")
    print(f"macmon samples: **{mm_samples}**"
          + (f" · unparsed lines: **{mm_unparsed}**" if mm_unparsed else "")
          + f" · powermetrics samples: **{pm_samples}**"
          + (f" · unparsed coalition lines: **{unparsed}** (check power.log against "
             "powermetrics-format-sample.txt)" if unparsed else ""))
    if pm_samples == 0:
        print("\n> No powermetrics data, so **nothing here attributes cost to Mnema specifically** — "
              "every power row below is whole-machine. Re-run without `--no-coalitions` for the "
              "per-coalition split.")
    print()

    # ---- disk
    print("### Disk\n")
    if rows:
        xs = [num(r, "elapsed_s") or 0.0 for r in rows]
        first, last = rows[0], rows[-1]
        hours = (xs[-1] - xs[0]) / 3600.0 if len(xs) > 1 else 0.0
        print("| Path | Observed growth | Slope → per day | Absolute at end |")
        print("|---|---|---|---|")
        for col, label in (
            ("disk_total_kb", "save dir (all)"),
            ("disk_recordings_kb", "recordings/"),
            ("disk_db_kb", "db/"),
        ):
            ys = [num(r, col) for r in rows]
            pairs = [(x, y) for x, y in zip(xs, ys) if y is not None]
            if not pairs:
                print(f"| {label} | - | - | - |")
                continue
            gx, gy = [p[0] for p in pairs], [p[1] for p in pairs]
            grown_mb = (gy[-1] - gy[0]) / 1024.0
            per_day = slope_per_day(gx, gy)
            per_day_mb = f"{per_day / 1024.0:.0f} MB/day" if per_day is not None else "-"
            print(f"| `{label}` | {grown_mb:.0f} MB over {hours:.1f} h | {per_day_mb} | {gy[-1] / 1024.0:.0f} MB |")
        print()

        vs, ve = num(first, "video_segments"), num(last, "video_segments")
        as_, ae = num(first, "audio_segments"), num(last, "audio_segments")
        seg_s = 60.0
        if start.exists():
            seg_s = float(json.loads(start.read_text()).get("segmentDurationSeconds", 60))
        if vs is not None and ve is not None:
            made = ve - vs
            active_h = made * seg_s / 3600.0
            print(f"Video segments written: **{made:.0f}** (≈ **{active_h:.1f} active hours** at "
                  f"{seg_s:.0f} s/segment) over {hours:.1f} h of wall clock — "
                  f"the pause-on-inactivity duty cycle is {100 * active_h / hours:.0f}% if that ratio holds.")
            if as_ is not None and ae is not None:
                print(f"Audio segments written: **{ae - as_:.0f}**.")
            ys = [num(r, "disk_total_kb") for r in rows]
            if active_h > 0 and ys[0] is not None and ys[-1] is not None:
                print(f"\n**Normalised: {((ys[-1] - ys[0]) / 1024.0) / active_h:.0f} MB per active hour.** "
                      "Use this, not MB/day, to compare two runs on different days — a quiet day and a "
                      "busy day are not the same workload.")
            print()

    # ---- cpu / gpu per coalition
    print("### CPU and GPU, per coalition\n")
    print("`ms/s ÷ 10 = % of one core`. Coalition, not process: the app's own work plus the "
          "helpers it induces.\n")
    print("| Coalition | CPU ms/s | GPU ms/s | Energy Impact |")
    print("|---|---|---|---|")
    names = sorted({k.split(":", 1)[1] for k in series if k.startswith("cpu_ms_s:")},
                   key=lambda n: -statistics.mean(series[f"cpu_ms_s:{n}"]))
    for name in names:
        cpu = series.get(f"cpu_ms_s:{name}", [])
        gpu = series.get(f"gpu_ms_s:{name}", [])
        ei = series.get(f"energy_impact:{name}", [])
        print(f"| `{name}` | {stats(cpu)} | {stats(gpu) if gpu else '-'} | {stats(ei) if ei else '-'} |")
    if not names:
        print("| - | no coalition rows parsed | - | - |")
    print()
    if names:
        print("> Energy Impact is Apple's own relative index — the number a user reads off "
              "Activity Monitor and quotes at you. Publishing it alongside watts pre-empts the "
              "comparison rather than being surprised by it.\n")
    dio = {k: v for k, v in series.items() if k.startswith("diskio:")}
    if dio:
        print("### Whole-machine disk I/O — powermetrics\n")
        print("| Direction | Value |")
        print("|---|---|")
        for key in sorted(dio):
            print(f"| {key.split(':', 1)[1]} | {stats(dio[key])} |")
        print("\n> Whole-machine, not ours — but sustained write KB/s is the check on the "
              "write-churn finding, where 1.3 GB was written to land 579 MB of growth.\n")

    # ---- convergence: is this run long enough to quote?
    print("### Did the run last long enough? (first half → second half)\n")
    print("A short run is only publishable if its numbers stopped moving. Anything marked "
          "**NOT converged** is a measurement of the run length, not of the app.\n")
    print("| Metric | First half → second half |")
    print("|---|---|")
    for name in sorted({k.split(":", 1)[1] for k in series if k.startswith("cpu_ms_s:")}):
        print(f"| coalition CPU ms/s — `{name}` | {split_half(series[f'cpu_ms_s:{name}'])} |")
    for key, label in (("mm_power:CPU + GPU + ANE", "CPU+GPU+ANE mW (whole machine)"),
                       ("mm_util:CPU total (%)", "CPU total % (whole machine)"),
                       ("mm_temp:CPU die (avg)", "CPU die °C")):
        if key in mm:
            print(f"| {label} | {split_half(mm[key])} |")
    if rows:
        fp = [v for v in (num(r, "footprint_mb") for r in rows) if v is not None]
        if fp:
            print(f"| app footprint MB | {split_half(fp)} |")
        disk_series = [v for v in (num(r, "disk_total_kb") for r in rows) if v is not None]
        if len(disk_series) > 3:
            deltas = [b - a for a, b in zip(disk_series, disk_series[1:])]
            print(f"| disk growth per interval (KB) | {split_half(deltas)} |")
    print()

    # ---- power (macmon primary, powermetrics as the cross-check)
    if mm:
        print("### Power, whole machine — macmon\n")
        print("| Rail | mW |")
        print("|---|---|")
        for _, label, _ in MACMON_RAILS:
            key = f"mm_power:{label}"
            if key in mm:
                print(f"| {label} | {stats(mm[key])} |")
        print()
        cross = sorted(k for k in series if k.startswith("power:"))
        if cross:
            print("Independent cross-check from powermetrics (two routes converging is what makes a "
                  "published wattage believable):\n")
            print("| Rail | mW |")
            print("|---|---|")
            for key in cross:
                print(f"| {key.split(':', 1)[1]} | {stats(series[key])} |")
            print()
    else:
        print("### Power, whole machine — powermetrics\n")
        print("| Rail | mW |")
        print("|---|---|")
        for key in sorted(k for k in series if k.startswith("power:")):
            print(f"| {key.split(':', 1)[1]} | {stats(series[key])} |")
        if not any(k.startswith("power:") for k in series):
            print("| - | no power lines parsed |")
        print()

    print("> Whole-machine power is **not** the app's cost unless nothing else was recording. "
          "Check the 'other recorders running at start' block in the method of record, and read it "
          "against the per-coalition table above.\n")

    # ---- utilisation and frequency
    util = {k: v for k, v in mm.items() if k.startswith(("mm_util:", "mm_freq:"))}
    if util:
        print("### Utilisation and frequency — macmon\n")
        print("| Metric | Value |")
        print("|---|---|")
        for key in sorted(util):
            print(f"| {key.split(':', 1)[1]} | {stats(util[key])} |")
        print()

    # ---- thermals
    mm_temps = {k: v for k, v in mm.items() if k.startswith("mm_temp:")}
    pm_temps = {k: v for k, v in series.items() if k.startswith("temp:")}
    if mm_temps or pm_temps:
        print("### Thermals\n")
        print("| Sensor | °C | Source |")
        print("|---|---|---|")
        for key in sorted(mm_temps):
            print(f"| {key.split(':', 1)[1]} | {stats(mm_temps[key])} | macmon |")
        for key in sorted(pm_temps):
            print(f"| {key.split(':', 1)[1]} | {stats(pm_temps[key])} | powermetrics |")
        print("\n> On a fanless machine a rising max is a lower bound, not a plateau. Only quote "
              "the plateau if the last hour is flat — and `pmset -g therm` is useless here, it "
              "reports no thermal level on Apple Silicon.\n")

    # ---- machine memory pressure
    mem_series = {k: v for k, v in mm.items() if k.startswith("mm_mem:")}
    if mem_series:
        print("### Machine-wide memory (context for the app's footprint)\n")
        print("| Metric | GB |")
        print("|---|---|")
        for key in sorted(mem_series):
            print(f"| {key.split(':', 1)[1]} | {stats(mem_series[key])} |")
        print("\n> Swap growth during a run is a finding in itself: it means the config does not fit "
              "this machine, which is the honest way to report a memory cost.\n")

    # ---- memory
    if rows:
        print("### Memory (`vmmap` physical footprint)\n")
        print("| Process | MB |")
        print("|---|---|")
        for col, label in (("footprint_mb", "app main process"),
                           ("webcontent_mb", "WebKit WebContent (sum)")):
            vals = [v for v in (num(r, col) for r in rows) if v is not None]
            print(f"| {label} | {stats(vals)} |")
        print()
        wc = [v for v in (num(r, "webcontent_mb") for r in rows) if v is not None]
        if len(wc) >= 3 and wc[-1] > wc[0]:
            per_day = slope_per_day([num(r, "elapsed_s") or 0.0 for r in rows], wc)
            if per_day and per_day > 0:
                print(f"> WebContent footprint is trending **+{per_day:.0f} MB/day**. That is the "
                      "IOSurface strand signature — confirm against the run length before "
                      "publishing any memory row.\n")

    # ---- battery
    if rows:
        runs: list[list[tuple[float, float]]] = []
        cur: list[tuple[float, float]] = []
        for r in rows:
            pct, elapsed, state = num(r, "batt_pct"), num(r, "elapsed_s"), r.get("batt_state", "")
            if pct is None or elapsed is None or "discharging" not in state:
                if len(cur) > 1:
                    runs.append(cur)
                cur = []
                continue
            cur.append((elapsed, pct))
        if len(cur) > 1:
            runs.append(cur)
        print("### Battery\n")
        if runs:
            longest = max(runs, key=lambda seg: seg[-1][0] - seg[0][0])
            span_h = (longest[-1][0] - longest[0][0]) / 3600.0
            drop = longest[0][1] - longest[-1][1]
            if span_h > 0 and drop > 0:
                rate = drop / span_h
                print(f"Longest continuous discharge: **{span_h:.1f} h**, {longest[0][1]:.0f}% → "
                      f"{longest[-1][1]:.0f}% = **{rate:.1f} %/hour** "
                      f"(≈ {100 / rate:.1f} h to empty at this rate)")
            else:
                print(f"Longest continuous discharge: {span_h:.1f} h, no net drop.")
            print("\n> This is an observational rate under one real workload, **not** the "
                  "capture-on vs capture-off A/B. It is not publishable as 'what Mnema costs you' "
                  "until the paired run exists.\n")
        else:
            print("No discharge stretch recorded — the machine was on AC. A battery number needs "
                  "a run on battery, and the claim needs a paired capture-off run.\n")

    return 0


if __name__ == "__main__":
    sys.exit(main())
