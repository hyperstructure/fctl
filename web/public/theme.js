// Applies the theme before the first paint (no flash). Kept outside the
// bundle so the Content-Security-Policy can stay at script-src 'self'.
try {
  var t = localStorage.getItem("fctl-theme");
  var dark = t ? t === "dark" : matchMedia("(prefers-color-scheme: dark)").matches;
  document.documentElement.dataset.theme = dark ? "dark" : "light";
} catch (e) {}
