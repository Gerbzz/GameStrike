// Runs first on the Collection page (index.html), before anything draws: if the saved dashboard style is
// one of the earlier ones (Blades, Original, PWDS Orange), the launcher opens classic.html instead.
// Both pages are bundled files on the same trusted origin; nothing else is ever navigated to.
(function () {
  "use strict";
  var style = null;
  try { style = window.localStorage.getItem("gamestrike-style"); } catch (_) { style = null; }
  if (style === "blades" || style === "original" || style === "pwds") {
    document.documentElement.setAttribute("data-leaving", "classic");
    window.location.replace("/classic.html");
  }
})();
