/* app.js — tiny, dependency-free wiring for the Mnema Insights mockups.
 * - Applies the persisted theme on load.
 * - Flips dark/light on any [data-theme-toggle] element and persists it.
 * - Cosmetic active-state toggling for the surface toggle and sub-nav tabs. */
(function () {
  "use strict";

  var KEY = "mnema-mock-theme";
  var root = document.documentElement;

  function glyphFor(theme) {
    return theme === "light" ? "☾" : "☀"; // ☾ in light, ☀ in dark
  }

  function applyTheme(theme) {
    if (theme === "light") {
      root.dataset.theme = "light";
    } else {
      delete root.dataset.theme;
    }
    var g = glyphFor(theme);
    var toggles = document.querySelectorAll("[data-theme-toggle]");
    for (var i = 0; i < toggles.length; i++) toggles[i].textContent = g;
  }

  // ---- Initial theme from storage ----
  var stored = null;
  try { stored = localStorage.getItem(KEY); } catch (e) {}
  applyTheme(stored === "light" ? "light" : "dark");

  document.addEventListener("DOMContentLoaded", function () {
    // re-apply so toggle glyphs render once buttons exist
    applyTheme(root.dataset.theme === "light" ? "light" : "dark");

    // ---- Theme toggle ----
    document.addEventListener("click", function (ev) {
      var t = ev.target.closest("[data-theme-toggle]");
      if (!t) return;
      var next = root.dataset.theme === "light" ? "dark" : "light";
      applyTheme(next);
      try { localStorage.setItem(KEY, next); } catch (e) {}
    });

    // ---- Surface toggle (Timeline / Insights) ----
    // On main-shell.html (which HAS #panel-timeline + its own inline script that
    // swaps the panels locally), DO NOT hijack the toggle — that script owns it.
    // On a normal Insights content page (no #panel-timeline), clicking a segment
    // navigates to the matching surface so the toggle genuinely switches views.
    var hasTimelinePanel = !!document.getElementById("panel-timeline");
    var surfaceBtns = document.querySelectorAll(".surface-toggle button");
    surfaceBtns.forEach(function (btn) {
      btn.addEventListener("click", function () {
        // Cosmetic active move within the group (both surfaces).
        surfaceBtns.forEach(function (b) {
          b.classList.remove("active");
          b.removeAttribute("aria-current");
        });
        btn.classList.add("active");
        btn.setAttribute("aria-current", "page");

        // Surface navigation only on Insights content pages.
        if (!hasTimelinePanel) {
          var label = (btn.textContent || "").trim().toLowerCase();
          if (label === "timeline") {
            window.location.href = "main-shell.html";
          } else if (label === "insights") {
            window.location.href = "overview.html";
          }
        }
      });
    });

    // ---- Sub-nav tabs (cosmetic). Tabs are real <a href> links, so the
    //      browser navigates between static pages; we only intercept activation
    //      styling for non-link tabs. Real active state is set per-page via the
    //      .active class + aria-current in markup. ----
    var subnavTabs = document.querySelectorAll(".subnav-tab");
    subnavTabs.forEach(function (tab) {
      tab.addEventListener("click", function () {
        var isLink = tab.tagName === "A" && tab.getAttribute("href");
        if (isLink) return; // let the browser navigate
        subnavTabs.forEach(function (t) {
          t.classList.remove("active");
          t.removeAttribute("aria-current");
        });
        tab.classList.add("active");
        tab.setAttribute("aria-current", "page");
      });
    });

    // ---- Date range (Day / Week / Month), cosmetic. Moves .active within the
    //      group so the control feels responsive. ----
    var rangeBtns = document.querySelectorAll(".date-range button");
    rangeBtns.forEach(function (btn) {
      btn.addEventListener("click", function () {
        rangeBtns.forEach(function (b) { b.classList.remove("active"); });
        btn.classList.add("active");
      });
    });
  });
})();
