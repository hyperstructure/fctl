// Docs pages: styles, plus a copy button on every code block.

import "./site.css";

for (const pre of document.querySelectorAll<HTMLPreElement>(".doc pre")) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "copy doc-copy";
  button.textContent = "Copy";
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(pre.innerText.replace(/^\$ /gm, ""));
      button.textContent = "Copied";
      setTimeout(() => {
        button.textContent = "Copy";
      }, 1500);
    } catch {
      button.textContent = "Select and copy";
    }
  });
  pre.classList.add("has-copy");
  pre.append(button);
}
