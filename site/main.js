// Install tabs, the copy button and the language switch; the page works
// without this script, in English.
for (const box of document.querySelectorAll("[data-tabs]")) {
  const tabs = box.querySelectorAll("[data-tab]");
  const panels = box.querySelectorAll("[data-panel]");
  const copy = box.querySelector(".copy");
  for (const tab of tabs) {
    tab.addEventListener("click", () => {
      for (const t of tabs) t.setAttribute("aria-selected", String(t === tab));
      for (const p of panels) p.hidden = p.dataset.panel !== tab.dataset.tab;
      // A panel of download links has nothing to copy.
      if (copy) copy.hidden = [...panels].some((p) => !p.hidden && p.hasAttribute("data-nocopy"));
    });
  }
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

// Copy buttons outside the install box copy the text of their window.
for (const btn of document.querySelectorAll("[data-copy]")) {
  btn.addEventListener("click", async () => {
    const body = btn.closest("[data-copyable]")?.querySelector("pre");
    if (!body) return;
    try {
      await navigator.clipboard.writeText(body.textContent.trim());
      btn.classList.add("done");
      setTimeout(() => btn.classList.remove("done"), 1500);
    } catch {
      // Clipboard access can be refused; the text stays selectable.
    }
  });
}

// Languages. English is the page as served; the other eleven come from
// i18n/<tag>.js, loaded when chosen, and replace elements marked data-i18n.
// The choice comes from ?lang=, then the last choice on this device, then the
// browser's languages, with the same rules as the CLI (docs/i18n.md).
const LANGS = ["en", "ko", "ja", "zh-Hans", "zh-Hant", "es", "fr", "de", "pt-BR", "vi", "id", "ru"];
const resolve = (locale) => {
  const parts = String(locale ?? "").split(/[.@]/)[0].trim().replace(/_/g, "-").toLowerCase()
    .split("-").filter(Boolean);
  const [language, ...rest] = parts;
  if (!language) return null;
  if (language === "zh") {
    const traditional = rest.some((p) => ["tw", "hk", "mo"].includes(p) || p.startsWith("hant"));
    return traditional ? "zh-Hant" : "zh-Hans";
  }
  if (language === "pt") return "pt-BR";
  if (language === "in") return "id";
  return LANGS.find((tag) => tag.toLowerCase() === language) ?? null;
};
const switcher = document.querySelector("[data-lang-switch]");
if (switcher) {
  const english = new Map();
  for (const el of document.querySelectorAll("[data-i18n]")) english.set(el, el.innerHTML);
  const meta = document.querySelector('meta[name="description"]');
  const en = { title: document.title, description: meta.content };
  const flow = document.querySelector("[data-flow]");
  const flowEn = flow && { src: flow.getAttribute("src"), alt: flow.alt };

  const apply = (lang) => {
    const dict = lang === "en" ? {} : window.ITDA_I18N?.[lang] ?? {};
    document.documentElement.lang = lang;
    for (const [el, html] of english) el.innerHTML = dict[el.dataset.i18n] ?? html;
    document.title = dict["meta.title"] ?? en.title;
    meta.content = dict["meta.description"] ?? en.description;
    if (flow) {
      flow.src = dict["flow.src"] ?? flowEn.src;
      flow.alt = dict["flow.alt"] ?? flowEn.alt;
    }
    switcher.value = lang;
  };

  // A language's strings load once, from its own file, then apply.
  const show = (lang) => {
    if (lang === "en" || window.ITDA_I18N?.[lang]) return apply(lang);
    const script = document.createElement("script");
    script.src = `i18n/${lang}.js`;
    script.onload = () => apply(lang);
    script.onerror = () => apply("en");
    document.head.append(script);
  };

  const fromUrl = new URLSearchParams(location.search).get("lang");
  let saved = null;
  try { saved = localStorage.getItem("itda-lang"); } catch {}
  const browser = (navigator.languages ?? [navigator.language]).map(resolve).find(Boolean);
  const initial = LANGS.includes(fromUrl) ? fromUrl
    : LANGS.includes(saved) ? saved
    : browser ?? "en";
  if (initial !== "en") show(initial);

  switcher.addEventListener("change", () => {
    const next = LANGS.includes(switcher.value) ? switcher.value : "en";
    show(next);
    try { localStorage.setItem("itda-lang", next); } catch {}
    const url = new URL(location.href);
    url.searchParams.set("lang", next);
    history.replaceState(null, "", url);
  });
}

// The desktop: windows come to the front when touched and can be dragged
// around on wide screens. The clock shows the viewer's own time.
const clock = document.querySelector("[data-clock]");
if (clock) {
  const tick = () => { clock.textContent = new Date().toLocaleTimeString("en-GB"); };
  tick();
  setInterval(tick, 1000);
}
const wins = [...document.querySelectorAll("[data-win]")];
let zTop = 10;
const drag = window.matchMedia("(min-width: 961px) and (pointer: fine)");
for (const win of wins) {
  win.addEventListener("pointerdown", (e) => {
    win.style.zIndex = String(++zTop);
    if (!drag.matches || e.target.closest("a, button, pre")) return;
    const sx = e.clientX, sy = e.clientY;
    const ox = parseFloat(win.style.getPropertyValue("--dx")) || 0;
    const oy = parseFloat(win.style.getPropertyValue("--dy")) || 0;
    win.classList.add("dragging");
    const move = (ev) => {
      win.style.setProperty("--dx", `${ox + ev.clientX - sx}px`);
      win.style.setProperty("--dy", `${oy + ev.clientY - sy}px`);
    };
    const up = () => {
      win.classList.remove("dragging");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
    window.addEventListener("pointermove", move, { passive: true });
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  });
}
