// The PWDS Collection is the only dashboard. Migrate only the obsolete style preference.
(function () {
  "use strict";
  try {
    if (window.localStorage.getItem("gamestrike-style") !== "collection-orange") {
      window.localStorage.setItem("gamestrike-style", "collection-orange");
    }
  } catch (_) { /* The fixed default also works without optional storage. */ }
  if (window.location.pathname === "/classic.html") {
    document.documentElement.setAttribute("data-leaving", "collection");
    window.location.replace("/index.html");
  }
})();
