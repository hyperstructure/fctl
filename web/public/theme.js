// Applies the theme before the first paint (no flash), and colors the browser's
// and the installed app's title bar to match. Kept outside the bundle so the
// Content-Security-Policy can stay at script-src 'self'.
try {
  var t = localStorage.getItem("fctl-theme");
  var dark = t ? t === "dark" : matchMedia("(prefers-color-scheme: dark)").matches;
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  // Each theme's canvas color; src/lib/theme.ts sets it again on a switch.
  document.querySelector('meta[name="theme-color"]').content = dark ? "#171717" : "#fafafa";
} catch (e) {}
