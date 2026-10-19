// Install tabs and the copy button; the page works without this script.
for (const box of document.querySelectorAll("[data-tabs]")) {
  const tabs = box.querySelectorAll("[data-tab]");
  const panels = box.querySelectorAll("[data-panel]");
  for (const tab of tabs) {
    tab.addEventListener("click", () => {
      for (const t of tabs) t.setAttribute("aria-selected", String(t === tab));
      for (const p of panels) p.hidden = p.dataset.panel !== tab.dataset.tab;
    });
  }
  const copy = box.querySelector(".copy");
  copy?.addEventListener("click", async () => {
    const shown = [...panels].find((p) => !p.hidden);
    try {
      await navigator.clipboard.writeText(shown.textContent.trim());
      copy.classList.add("done");
      setTimeout(() => copy.classList.remove("done"), 1500);
    } catch {
      // Clipboard access can be refused; the command stays selectable.
    }
  });
}
