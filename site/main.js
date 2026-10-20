// Install tabs, the copy button and the language switch; the page works
// without this script, in English.
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

// English and Korean. English is the page as served; Korean strings come from
// i18n.js and replace elements marked data-i18n. The choice comes from ?lang=,
// then the last choice on this device, then the browser's language.
const ko = window.ITDA_KO;
const switcher = document.querySelector("[data-lang-switch]");
if (ko && switcher) {
  const english = new Map();
  for (const el of document.querySelectorAll("[data-i18n]")) english.set(el, el.innerHTML);
  const meta = document.querySelector('meta[name="description"]');
  const en = { title: document.title, description: meta.content };
  const flow = document.querySelector("[data-src-ko]");
  const flowEn = flow && { src: flow.getAttribute("src"), alt: flow.alt };

  const apply = (lang) => {
    const isKo = lang === "ko";
    document.documentElement.lang = lang;
    for (const [el, html] of english) el.innerHTML = isKo ? ko[el.dataset.i18n] ?? html : html;
    document.title = isKo ? ko["meta.title"] : en.title;
    meta.content = isKo ? ko["meta.description"] : en.description;
    if (flow) {
      flow.src = isKo ? flow.dataset.srcKo : flowEn.src;
      flow.alt = isKo ? flow.dataset.altKo : flowEn.alt;
    }
    switcher.textContent = isKo ? "English" : "한국어";
    switcher.lang = isKo ? "en" : "ko";
    switcher.setAttribute("aria-label", isKo ? "View in English" : "한국어로 보기");
  };

  const fromUrl = new URLSearchParams(location.search).get("lang");
  let saved = null;
  try { saved = localStorage.getItem("itda-lang"); } catch {}
  const initial = ["en", "ko"].includes(fromUrl) ? fromUrl
    : saved ?? (navigator.language?.toLowerCase().startsWith("ko") ? "ko" : "en");
  if (initial === "ko") apply("ko");

  switcher.addEventListener("click", () => {
    const next = document.documentElement.lang === "ko" ? "en" : "ko";
    apply(next);
    try { localStorage.setItem("itda-lang", next); } catch {}
    const url = new URL(location.href);
    url.searchParams.set("lang", next);
    history.replaceState(null, "", url);
  });
}
