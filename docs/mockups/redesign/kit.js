// Mnema redesign kit — shared behavior. Load in <head> WITHOUT defer:
// the saved theme is applied before first paint; everything else waits for DOMContentLoaded.
(() => {
  const root = document.documentElement;
  const KEY = "mnema-mock-theme";
  const saved = new URLSearchParams(location.search).get("theme") || localStorage.getItem(KEY); // ?theme=light for headless renders
  if (saved) root.dataset.theme = saved;

  // Slide an indicator under the active child of a group.
  const slide = (thumb, el) => {
    if (!thumb || !el) return;
    thumb.style.width = `${el.offsetWidth}px`;
    thumb.style.transform = `translateX(${el.offsetLeft}px)`;
  };

  // ---------------------------------------------------------------- dates
  const DAY_MS = 864e5;
  const parse = (s) => { const [y, m, d] = s.split("-").map(Number); return new Date(y, m - 1, d, 12); };
  const iso = (d) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  const add = (d, n) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n, 12);
  const days = (a, b) => Math.round((b - a) / DAY_MS);
  const monday = (d) => add(d, -((d.getDay() + 6) % 7));
  const monthOf = (d, k = 0) => [new Date(d.getFullYear(), d.getMonth() + k, 1, 12), new Date(d.getFullYear(), d.getMonth() + k + 1, 0, 12)];
  const fmt = (d, o) => d.toLocaleDateString("en-US", o);
  const span = (a, b) => +a === +b ? fmt(a, { month: "short", day: "numeric" }) : a.getMonth() === b.getMonth()
    ? `${fmt(a, { month: "short", day: "numeric" })} – ${b.getDate()}`
    : `${fmt(a, { month: "short", day: "numeric" })} – ${fmt(b, { month: "short", day: "numeric" })}`;
  // ponytail: deterministic fake capture density for mockups (weekends lighter); pages override el.mxHeat.
  const fakeHeat = (d) => {
    const r = Math.abs(Math.sin((d.getFullYear() * 372 + d.getMonth() * 31 + d.getDate()) * 12.9898) * 43758.5453) % 1;
    return r < (d.getDay() % 6 === 0 ? 0.55 : 0.1) ? 0 : r < 0.45 ? 1 : r < 0.8 ? 2 : 3;
  };
  const ICON_CAL = '<svg class="mx-date__icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round"><rect x="3" y="5" width="18" height="16" rx="2"/><path d="M16 3v4M8 3v4M3 10h18"/></svg>';
  const chev = (dir) => `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="${dir < 0 ? "m15 6-6 6 6 6" : "m9 6 6 6-6 6"}"/></svg>`;

  const isoWeek = (d) => { const t = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate())); t.setUTCDate(t.getUTCDate() + 4 - (t.getUTCDay() || 7));
    return Math.ceil(((t - Date.UTC(t.getUTCFullYear(), 0, 1)) / DAY_MS + 1) / 7); };
  const UNITS = ["day", "week", "month"];
  const cap = (s) => s[0].toUpperCase() + s.slice(1);

  function dateInput(host, n) {
    const range = host.dataset.dateInput === "range";
    const period = host.dataset.dateInput === "period"; // one whole day / week / month, stepped as a unit
    const today = parse(host.dataset.today || iso(new Date()));
    const min = host.dataset.min ? parse(host.dataset.min) : add(today, -120);
    const max = host.dataset.max ? parse(host.dataset.max) : today;
    let [a, b] = (host.dataset.value || iso(today)).split("..").map(parse);
    b ||= a;
    let unit = range || period ? host.dataset.unit || "week" : "day"; // day | week | month | span
    const periodOf = (d, u = unit) => (u === "week" ? [monday(d), add(monday(d), 6)] : u === "month" ? monthOf(d) : [d, d]);
    if (period) [a, b] = periodOf(a);
    const heat = (d) => (d < min || d > max ? -1 : (host.mxHeat || fakeHeat)(d));
    const periodHeat = (s, u) => { // mean density of the captured days in the period
      let t = 0, k = 0;
      for (let d = s, e = periodOf(s, u)[1]; d <= e; d = add(d, 1)) { const h = heat(d); if (h >= 0) { t += h; k++; } }
      return k ? Math.round(t / k) : -1;
    };
    const id = `${host.id || `mx-date-${n}`}-pop`;
    const unitBtns = (short) => UNITS.map((u) => `<button type="button" data-unit="${u}" aria-pressed="${u === unit}"${short ? ` aria-label="${cap(u)}" data-tip="${cap(u)}"` : ""}>${short ? u[0].toUpperCase() : cap(u)}</button>`).join("");
    host.classList.add("mx-date");
    host.dataset.mode = range ? "range" : period ? "period" : "day";
    host.innerHTML = `<button type="button" class="mx-date__btn" data-pop="${id}" aria-haspopup="dialog" aria-expanded="false">
        ${ICON_CAL}<span class="mx-date__text"><b></b><small></small></span>
        <span class="mx-date__ruler" aria-hidden="true"><span class="mx-date__tape"></span></span></button>
      ${period ? `<span class="mx-date__units" role="group" aria-label="Period">${unitBtns(true)}</span>` : ""}
      <div class="mx-pop mx-date__pop" id="${id}" role="dialog" aria-label="Choose ${range ? "a range" : period ? "a period" : "a day"}"></div>`;
    const btn = host.firstElementChild, pop = host.lastElementChild;
    const label = btn.querySelector("b"), rel = btn.querySelector("small");
    const ruler = btn.querySelector(".mx-date__ruler"), tape = btn.querySelector(".mx-date__tape");
    const PITCH = 7; // tick 2px + gap 5px
    // One tick per day (per week / month in period mode), height = capture density.
    let starts = [], ticks = [];
    const buildTape = () => {
      const u = period ? unit : "day";
      starts = [];
      for (let d = periodOf(min, u)[0]; d <= max; d = u === "month" ? monthOf(d, 1)[0] : add(d, u === "week" ? 7 : 1)) starts.push(d);
      ticks = starts.map(() => document.createElement("i"));
      ticks.forEach((t, i) => (t.dataset.h = period ? periodHeat(starts[i], u) : heat(starts[i])));
      tape.replaceChildren(...ticks);
    };
    buildTape();

    const presets = () => {
      const m0 = monday(today);
      return [
        ["Today", today, today, "day"],
        ["Yesterday", add(today, -1), add(today, -1), "day"],
        ["This week", m0, add(m0, 6), "week"],
        ["Last week", add(m0, -7), add(m0, -1), "week"],
        ["Last 7 days", add(today, -6), today, "span"],
        ["Last 30 days", add(today, -29), today, "span"],
        ["This month", ...monthOf(today), "month"],
        ["Last month", ...monthOf(today, -1), "month"],
      ];
    };
    // How many whole periods back from the current one (0 = this period).
    const ago = () => unit === "month"
      ? (today.getFullYear() - a.getFullYear()) * 12 + today.getMonth() - a.getMonth()
      : Math.round(days(a, periodOf(today)[0]) / (unit === "week" ? 7 : 1));
    const resetLabel = () => (unit === "day" ? "Today" : `This ${unit}`);
    const relLabel = () => {
      if (period) {
        const k = ago();
        return k === 0 ? resetLabel().toLowerCase() : k === 1 ? (unit === "day" ? "yesterday" : `last ${unit}`) : `${k} ${unit}s ago`;
      }
      if (!range) {
        const k = days(a, today);
        return k === 0 ? "today" : k === 1 ? "yesterday" : `${k} days ago`;
      }
      const p = presets().find(([, s, e]) => +s === +a && +e === +b);
      if (p) return p[0];
      if (unit === "month") return fmt(a, { month: "long" });
      if (unit === "week") return `week of ${fmt(a, { month: "short", day: "numeric" })}`;
      const n = days(a, b) + 1;
      return n === 1 ? "1 day" : `${n} days`;
    };

    const paint = (emit = true) => {
      label.textContent = !period ? (range ? span(a, b) : fmt(a, { weekday: "short", month: "short", day: "numeric" }))
        : unit === "month" ? fmt(a, { month: "long", year: "numeric" }) : unit === "week" ? span(a, b)
        : fmt(a, { month: "short", day: "numeric", ...(a.getFullYear() !== today.getFullYear() && { year: "numeric" }) });
      rel.textContent = relLabel();
      btn.setAttribute("aria-label", `${range ? "Range" : period ? cap(unit) : "Day"}: ${label.textContent}, ${rel.textContent}. Arrow keys or drag to step, Enter for calendar.`);
      host.querySelectorAll(":scope > .mx-date__units button").forEach((u) => u.setAttribute("aria-pressed", String(u.dataset.unit === unit)));
      const k = starts.findIndex((s) => +s === +a);
      const [i0, i1] = period ? [k, k] : [days(min, a), days(min, b)];
      ticks.forEach((t, i) => t.toggleAttribute("data-sel", i >= i0 && i <= i1));
      const center = ((i0 + i1) / 2) * PITCH + 1;
      tape.style.transform = `translateX(${(ruler.clientWidth || 84) / 2 - center}px)`;
      if (pop.hasAttribute("data-open")) { view = monthOf(a)[0]; renderPop(); }
      if (emit) host.dispatchEvent(new CustomEvent("mx-change", { bubbles: true, detail: { start: iso(a), end: iso(b), unit } }));
    };

    // Step by one unit; refuses to leave [min, max] entirely.
    const step = (k) => {
      let na, nb;
      if (unit === "day") na = nb = add(a, k);
      else if (unit === "week") { na = add(a, 7 * k); nb = add(b, 7 * k); }
      else if (unit === "month") [na, nb] = monthOf(a, k);
      else { const len = days(a, b) + 1; na = add(a, len * k); nb = add(b, len * k); }
      if (unit === "day" ? na < min || na > max : na > max || nb < min) return false;
      a = na; b = nb; paint();
      return true;
    };
    const latest = () => {
      if (unit === "day") a = b = max;
      else if (unit === "week") { a = monday(max); b = add(a, 6); }
      else if (unit === "month") [a, b] = monthOf(max);
      else { const len = days(a, b); b = max; a = add(max, -len); }
      paint();
    };
    // Period mode: switch day / week / month. Stays on "now" if the current period was showing,
    // otherwise lands on the period containing the old start.
    const setUnit = (u) => {
      if (!period || u === unit || !UNITS.includes(u)) return;
      const focus = ago() === 0 ? today : a < min ? min : a;
      unit = u; [a, b] = periodOf(focus); buildTape();
      view = monthOf(a)[0];
      paint();
    };
    host.querySelector(":scope > .mx-date__units")?.addEventListener("click", (e) => setUnit(e.target.closest("button")?.dataset.unit));

    // --- drag / wheel / keys on the capsule
    const stepPx = period ? { day: PITCH, week: PITCH, month: PITCH } : { day: PITCH, week: 24, month: 36, span: 24 };
    let drag = null, dragged = false;
    btn.addEventListener("pointerdown", (e) => {
      if (e.button) return;
      drag = { x: e.clientX, applied: 0 }; dragged = false;
      btn.setPointerCapture(e.pointerId);
    });
    btn.addEventListener("pointermove", (e) => {
      if (!drag) return;
      const dx = e.clientX - drag.x;
      if (!dragged && Math.abs(dx) > 3) { dragged = true; host.dataset.dragging = ""; pop.removeAttribute("data-open"); }
      // dragging the tape right pulls earlier days under the notch
      const want = -Math.trunc(dx / stepPx[unit]);
      while (drag.applied !== want) {
        const dir = Math.sign(want - drag.applied);
        if (!step(dir)) { drag.x = e.clientX + drag.applied * stepPx[unit]; break; } // pinned at an edge: no debt
        drag.applied += dir;
      }
    });
    const endDrag = () => { drag = null; delete host.dataset.dragging; };
    btn.addEventListener("pointerup", endDrag);
    btn.addEventListener("pointercancel", endDrag);
    btn.addEventListener("click", (e) => {
      if (dragged) { e.stopPropagation(); dragged = false; return; } // a drag is not a click
      view = new Date(a.getFullYear(), a.getMonth(), 1, 12);
      anchor = null;
      renderPop();
    });
    let acc = 0;
    host.addEventListener("wheel", (e) => {
      if (pop.contains(e.target)) return;
      e.preventDefault();
      acc += Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
      if (Math.abs(acc) >= 40) { step(Math.sign(acc)); acc = 0; }
    }, { passive: false });
    btn.addEventListener("keydown", (e) => {
      const k = { ArrowLeft: -1, ArrowRight: 1, ArrowDown: -1, ArrowUp: 1 }[e.key];
      if (k && !e.metaKey) { e.preventDefault(); step(k); }
      else if (e.key === "Home" || e.key === "End") { e.preventDefault(); latest(); }
    });

    // --- popover: calendar (+ presets in range mode)
    let view = new Date(a.getFullYear(), a.getMonth(), 1, 12);
    let anchor = null, hover = null, selecting = false;
    const sel = () => (anchor ? [anchor, hover || anchor].sort((x, y) => x - y) : [a, b]);
    const renderPop = () => {
      const weeks = period && unit === "week", months = period && unit === "month";
      const start = monday(view);
      const cal = [];
      const cells = Math.ceil((days(start, view) + monthOf(view)[1].getDate()) / 7) * 7; // only weeks touching the month
      for (let i = 0; i < cells; i++) {
        const d = add(start, i), h = heat(d);
        const attrs = `${d.getMonth() !== view.getMonth() ? " data-out" : ""}${+d === +today ? " data-today" : ""}`;
        const cell = `<span>${d.getDate()}</span><i data-h="${Math.max(h, 0)}"></i>`;
        if (!weeks) { cal.push(`<button type="button" class="mx-cal__day" data-d="${iso(d)}"${attrs}${h < 0 ? " disabled" : ""} aria-label="${fmt(d, { weekday: "long", month: "long", day: "numeric" })}">${cell}</button>`); continue; }
        // week mode: each row is one button that selects Mon–Sun; hover lights the whole row
        if (i % 7 === 0) {
          const off = d > max || add(d, 6) < min;
          cal.push(`<button type="button" class="mx-cal__week" data-w="${iso(d)}"${+d === +a ? " data-sel" : ""}${off ? " disabled" : ""} aria-label="Week ${isoWeek(d)}, ${span(d, add(d, 6))}"><span class="mx-cal__wn">${isoWeek(d)}</span>`);
        }
        cal.push(`<span class="mx-cal__day"${attrs}${h < 0 ? " data-off" : ""}>${cell}</span>`);
        if (i % 7 === 6) cal.push("</button>");
      }
      const nav = (k, dis) => `<button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" data-nav="${k}" aria-label="${k < 0 ? "Previous" : "Next"} ${months ? "year" : "month"}"${dis ? " disabled" : ""}>${chev(k)}</button>`;
      const head = months
        ? `<span class="mx-cal__month">${view.getFullYear()}</span>${nav(-1, view.getFullYear() <= min.getFullYear())}${nav(1, view.getFullYear() >= max.getFullYear())}`
        : `<span class="mx-cal__month">${fmt(view, { month: "long", year: "numeric" })}</span>${nav(-1, view <= monthOf(min)[0])}${nav(1, view >= monthOf(max)[0])}`;
      // month mode: the year as 12 months, density = mean capture of each month
      const grid = months
        ? `<div class="mx-cal__months">${Array.from({ length: 12 }, (_, m) => {
            const m0 = new Date(view.getFullYear(), m, 1, 12), h = periodHeat(m0, "month");
            return `<button type="button" class="mx-cal__day mx-cal__mo" data-m="${iso(m0)}"${+m0 === +a ? " data-sel" : ""}${m === today.getMonth() && m0.getFullYear() === today.getFullYear() ? " data-today" : ""}${h < 0 ? " disabled" : ""} aria-label="${fmt(m0, { month: "long", year: "numeric" })}"><span>${fmt(m0, { month: "short" })}</span><i data-h="${Math.max(h, 0)}"></i></button>`;
          }).join("")}</div>`
        : `<div class="mx-cal__grid">${weeks ? '<span class="mx-cal__wd">wk</span>' : ""}${["M", "T", "W", "T", "F", "S", "S"].map((w) => `<span class="mx-cal__wd">${w}</span>`).join("")}${cal.join("")}</div>`;
      const foot = range ? `<span>${anchor ? "pick an end day" : "drag across days"}</span>`
        : `<button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" data-latest${period && ago() === 0 ? " disabled" : ""}>${period ? resetLabel() : "Today"}</button>`;
      const p = presets();
      pop.innerHTML = (range
        ? `<div class="mx-date__presets">${p.map(([l, s, e], i) => `<button type="button" data-p="${i}" aria-pressed="${+s === +a && +e === +b}"><span>${l}</span><small>${span(s, e)}</small></button>`).join("")}</div>`
        : "") + (period ? `<div class="mx-date__units" role="group" aria-label="Period">${unitBtns(false)}</div>` : "")
        + `<div class="mx-cal${weeks ? " mx-cal--weeks" : ""}">
          <div class="mx-cal__head">${head}</div>
          ${grid}
          <div class="mx-cal__foot"><span class="mx-cal__legend"><i></i><i></i><i></i>captured</span><span class="mx-spacer"></span>${foot}</div></div>`;
      paintSel();
    };
    const paintSel = () => {
      const [lo, hi] = sel();
      pop.querySelectorAll("button.mx-cal__day[data-d]").forEach((c) => {
        const t = +parse(c.dataset.d);
        c.toggleAttribute("data-sel", !range && t === +a);
        c.toggleAttribute("data-in", range && t >= +lo && t <= +hi);
        c.toggleAttribute("data-start", range && t === +lo);
        c.toggleAttribute("data-end", range && t === +hi);
      });
    };
    const commit = (na, nb, u) => {
      a = na; b = nb; unit = u; anchor = null; selecting = false;
      pop.removeAttribute("data-open"); btn.setAttribute("aria-expanded", "false");
      paint(); btn.focus();
    };
    const unitOf = (s, e) => {
      if (+s === +e) return "day";
      if (+s === +monday(s) && days(s, e) === 6) return "week";
      const [m0, m1] = monthOf(s);
      return +s === +m0 && +e === +m1 ? "month" : "span";
    };
    pop.addEventListener("click", (e) => {
      const t = e.target.closest("button");
      if (!t) return;
      if (t.dataset.nav) {
        view = period && unit === "month" ? new Date(view.getFullYear() + +t.dataset.nav, view.getMonth(), 1, 12) : monthOf(view, +t.dataset.nav)[0];
        renderPop();
      }
      else if (t.dataset.unit) setUnit(t.dataset.unit);
      else if (t.hasAttribute("data-latest")) period ? commit(...periodOf(max), unit) : commit(max, max, "day");
      else if (t.dataset.p) { const [, s, en, u] = presets()[+t.dataset.p]; commit(s, en, u); }
      else if (t.dataset.w) commit(...periodOf(parse(t.dataset.w), "week"), "week");
      else if (t.dataset.m) commit(...monthOf(parse(t.dataset.m)), "month");
      else if (!range && t.dataset.d) commit(parse(t.dataset.d), parse(t.dataset.d), "day");
    });
    if (range) {
      // drag across days, or click a start then an end
      pop.addEventListener("pointerdown", (e) => {
        const c = e.target.closest(".mx-cal__day:not(:disabled)");
        if (!c) return;
        const d = parse(c.dataset.d);
        if (anchor && !selecting) { const [lo, hi] = [anchor, d].sort((x, y) => x - y); return commit(lo, hi, unitOf(lo, hi)); }
        anchor = hover = d; selecting = true; paintSel();
      });
      pop.addEventListener("pointerover", (e) => {
        const c = e.target.closest(".mx-cal__day:not(:disabled)");
        if (c && anchor) { hover = parse(c.dataset.d); paintSel(); }
      });
      document.addEventListener("pointerup", () => {
        if (!selecting) return;
        selecting = false;
        if (+hover !== +anchor) { const [lo, hi] = sel(); commit(lo, hi, unitOf(lo, hi)); }
        else pop.querySelector(".mx-cal__foot > span:last-child").textContent = "pick an end day";
      });
    }

    paint(false);
    new ResizeObserver(() => paint(false)).observe(ruler);
    if (host.hasAttribute("data-date-open")) { renderPop(); pop.setAttribute("data-open", ""); btn.setAttribute("aria-expanded", "true"); }
    host.mxDate = { get: () => ({ start: iso(a), end: iso(b), unit }), step, latest, setUnit };
  }

  // Toast: mx.toast("Deleted 3 moments", { tone: "ok"|"danger", action: "Undo", onAction, ttl: 5000, host }).
  // Stacks in .mx-toasts inside the window (bottom-centre, above the status bar). An action toast shows its drain bar.
  const ICON_TONE = { ok: "M5 12.5 10 17 19 7", danger: "M12 8v5M12 16.5h.01" };
  window.mx = {
    toast(msg, { tone = "ok", action, onAction, ttl = action ? 5000 : 4000, host = document.querySelector(".mx-window") || document.body } = {}) {
      let box = host.querySelector(":scope > .mx-toasts");
      if (!box) host.append((box = Object.assign(document.createElement("div"), { className: "mx-toasts" })));
      const t = document.createElement("div");
      t.className = "mx-toast"; t.dataset.tone = tone; t.setAttribute("role", tone === "danger" ? "alert" : "status");
      t.innerHTML = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${tone === "danger" ? '<circle cx="12" cy="12" r="9"/>' : ""}<path d="${ICON_TONE[tone] || ICON_TONE.ok}"/></svg><span></span>`
        + (action ? `<button type="button" class="mx-btn mx-btn--ghost mx-btn--sm">${action}</button><i class="mx-toast__timer" style="--ttl:${ttl}ms"></i>` : "");
      t.querySelector("span").textContent = msg;
      const close = () => t.remove();
      t.querySelector(".mx-btn")?.addEventListener("click", () => { onAction?.(); close(); });
      box.append(t);
      setTimeout(close, ttl);
      return close;
    },
  };

  document.addEventListener("DOMContentLoaded", () => {
    // Theme: any [data-theme-toggle] (Settings → Appearance, kit gallery) or ⇧⌘L anywhere (mockup shortcut).
    // [data-theme-invert] scopes always show the other theme.
    const syncInvert = () =>
      document.querySelectorAll("[data-theme-invert]").forEach((el) => {
        el.dataset.theme = root.dataset.theme === "light" ? "dark" : "light";
      });
    const flipTheme = () => {
      const next = root.dataset.theme === "light" ? "dark" : "light";
      root.dataset.theme = next;
      localStorage.setItem(KEY, next);
      syncInvert();
    };
    syncInvert();
    document.querySelectorAll("[data-theme-toggle]").forEach((btn) => btn.addEventListener("click", flipTheme));

    // Current page: surface switch thumb + titlebar icon doors (chat, settings) get aria-current.
    const page = location.pathname.split("/").pop() || "index.html";
    document.querySelectorAll(".mx-titlebar__trail a").forEach((a) => a.getAttribute("href") === page && a.setAttribute("aria-current", "page"));
    document.querySelectorAll(".mx-nav").forEach((nav) => {
      const links = [...nav.querySelectorAll("a")];
      const cur = links.find((a) => a.getAttribute("href") === page);
      if (cur) links.forEach((a) => (a === cur ? a.setAttribute("aria-current", "page") : a.removeAttribute("aria-current")));
      let ink = nav.querySelector(".mx-nav__ink");
      if (!ink) nav.prepend((ink = Object.assign(document.createElement("span"), { className: "mx-nav__ink" })));
      const place = () => slide(ink, nav.querySelector('[aria-current="page"]'));
      place();
      new ResizeObserver(place).observe(nav);
    });

    // Segmented controls: single-select buttons with a sliding thumb.
    // Emits "mx-change" on the .mx-seg with detail = button value/text.
    document.querySelectorAll(".mx-seg").forEach((seg) => {
      const thumb = Object.assign(document.createElement("span"), { className: "mx-seg__thumb" });
      seg.prepend(thumb);
      const btns = [...seg.querySelectorAll("button")];
      const select = (b) => {
        btns.forEach((x) => x.setAttribute("aria-pressed", String(x === b)));
        slide(thumb, b);
        seg.dispatchEvent(new CustomEvent("mx-change", { detail: b.value || b.textContent.trim() }));
      };
      btns.forEach((b) => b.addEventListener("click", () => select(b)));
      const place = () => slide(thumb, btns.find((b) => b.getAttribute("aria-pressed") === "true") || btns[0]);
      place();
      new ResizeObserver(place).observe(seg); // web fonts swap in after first layout
    });

    // Feedback: [data-dismiss] removes its notification / banner / toast / notice; [data-clear-all] empties a
    // .mx-notifs list (CSS then shows its empty state); the bell's badge goes with the last row.
    // Registered before the popover handler so a removed row never reads as an outside click.
    document.addEventListener("click", (e) => {
      const t = e.target.closest("[data-dismiss], [data-clear-all]");
      if (!t) return;
      const pop = t.closest(".mx-notifs");
      if (t.hasAttribute("data-clear-all")) pop?.querySelectorAll(".mx-notif").forEach((n) => n.remove());
      else t.closest(".mx-notif, .mx-banner, .mx-toast, .mx-notice")?.remove();
      if (pop && !pop.querySelector(".mx-notif")) document.querySelector(`[data-pop="${pop.id}"] .mx-badge`)?.remove();
    });
    // Stopped-state source lanes are toggles (pick sources for the next recording).
    document.querySelectorAll("button.mx-lane[aria-pressed]").forEach((b) =>
      b.addEventListener("click", () => b.setAttribute("aria-pressed", String(b.getAttribute("aria-pressed") !== "true"))));

    // Date inputs (before popovers so their trigger click handlers run first).
    document.querySelectorAll("[data-date-input]").forEach(dateInput);

    // Popovers: <button data-pop="id"> toggles #id.mx-pop; outside click / Esc closes.
    const closeAll = (except) =>
      document.querySelectorAll(".mx-pop[data-open]").forEach((p) => {
        if (p === except) return;
        p.removeAttribute("data-open");
        document.querySelector(`[data-pop="${p.id}"]`)?.setAttribute("aria-expanded", "false");
      });
    document.addEventListener("click", (e) => {
      const trigger = e.target.closest("[data-pop]");
      if (trigger) {
        const pop = document.getElementById(trigger.dataset.pop);
        closeAll(pop);
        pop?.toggleAttribute("data-open");
        trigger.setAttribute("aria-expanded", String(!!pop?.hasAttribute("data-open")));
        return;
      }
      // a click that re-rendered its own popover leaves a detached target: not "outside"
      if (e.target.isConnected && !e.target.closest(".mx-pop")) closeAll();
    });
    document.addEventListener("keydown", (e) => e.key === "Escape" && closeAll());

    // Command field: the one recall/ask door. Tab flips mode, ↵ runs it, ⌘↵ always asks.
    const cmd = document.querySelector(".mx-cmd");
    const q = cmd?.querySelector("input");
    const setMode = (m) => {
      if (!cmd) return;
      cmd.dataset.mode = m;
      q.placeholder = m === "ask" ? "Ask about anything you’ve seen or heard…" : "Recall anything…";
    };
    const ask = () => {
      const t = q?.value.trim();
      go(t ? `chat.html?q=${encodeURIComponent(t)}` : "chat.html");
    };
    if (cmd) {
      setMode(cmd.dataset.mode || "recall");
      cmd.querySelector(".mx-cmd__mode")?.addEventListener("click", (e) => {
        e.preventDefault();
        setMode(cmd.dataset.mode === "ask" ? "recall" : "ask");
        q.focus();
      });
      q.addEventListener("keydown", (e) => {
        if (e.key === "Tab" && !e.shiftKey) { e.preventDefault(); setMode(cmd.dataset.mode === "ask" ? "recall" : "ask"); }
        else if (e.key === "Enter" && (e.metaKey || cmd.dataset.mode === "ask")) { e.preventDefault(); ask(); }
        else if (e.key === "Enter" && !e.defaultPrevented && q.value.trim()) {
          e.preventDefault(); console.log("opens Quick Recall pre-filled:", q.value.trim()); // §8 #2: recall ↵ is a launcher, not in-page search
        } else if (e.key === "Escape") { setMode("recall"); q.blur(); }
      });
    }

    // Shell shortcuts: ⌘K focus (recall) · ⌘J focus (ask) · ⌘1/⌘2 surfaces · ⌘, settings · ⇧⌘L theme.
    // Not ⌥Space / ⌥⌘Space: ⌥⌘Space is the global Quick Recall window (keyboard_bindings.rs), never the in-window field.
    const go = (href) => (location.href = href);
    document.addEventListener("keydown", (e) => {
      if (!e.metaKey) return;
      const k = e.key.toLowerCase();
      if (k === "k") { e.preventDefault(); setMode("recall"); (q || document.querySelector(".mx-search input"))?.focus(); }
      else if (k === "j") { e.preventDefault(); if (q) { setMode("ask"); q.focus(); } else go("chat.html"); }
      else if (k === "1") { e.preventDefault(); go("timeline.html"); }
      else if (k === "2") { e.preventDefault(); go("insights.html"); }
      else if (k === ",") { e.preventDefault(); go("settings.html"); }
      else if (k === "l" && e.shiftKey) { e.preventDefault(); flipTheme(); }
    });
  });
})();
