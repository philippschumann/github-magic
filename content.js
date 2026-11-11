// Quick filters on the PR list are hidden for now; flip to true to show them again.
const SHOW_FILTERS = false;

// Edit this list to change the buttons. `q` is a normal GitHub search query.
const FILTERS = [
  { label: "Open", q: "is:pr is:open" },
  { label: "Mine", q: "is:pr is:open author:@me" },
  { label: "Review requested", q: "is:pr is:open review-requested:@me" },
  { label: "Reviewed by me", q: "is:pr is:open reviewed-by:@me" },
  { label: "Assigned", q: "is:pr is:open assignee:@me" },
  { label: "Drafts", q: "is:pr is:open draft:true" },
  { label: "Approved", q: "is:pr is:open review:approved" },
  { label: "Changes requested", q: "is:pr is:open review:changes_requested" },
  { label: "Merged", q: "is:pr is:merged" },
];

const BAR_ID = "pr-quick-filters";
// Search box container on /owner/repo/pulls; the page is a React SPA, so we re-check on DOM changes.
const ANCHOR = "#repo-pulls-dashboard-filter";

const norm = (q) => (q || "").trim().replace(/\s+/g, " ");

function render() {
  const anchor = document.querySelector(ANCHOR);
  const current = norm(new URLSearchParams(location.search).get("q"));
  const existing = document.getElementById(BAR_ID);
  if (existing && existing.dataset.q === current && anchor && existing.previousElementSibling === anchor) return;
  existing?.remove();
  if (!anchor) return;

  const bar = document.createElement("nav");
  bar.id = BAR_ID;
  bar.dataset.q = current;
  bar.setAttribute("aria-label", "Quick filters");
  bar.style.cssText = "display:flex;flex-wrap:wrap;gap:6px;margin:8px 0;";

  for (const f of FILTERS) {
    const a = document.createElement("a");
    a.textContent = f.label;
    a.href = `${location.pathname}?q=${encodeURIComponent(f.q)}`;
    a.className = "btn btn-sm";
    if (norm(f.q) === current) {
      a.setAttribute("aria-current", "page");
      a.classList.add("btn-primary");
    }
    bar.append(a);
  }
  anchor.after(bar);
}

// Dropdown on PR pages; the native host (native-host/) checks the PR out in a worktree and acts on it.
const MENU_ID = "pr-open-locally";
const PR_PATH = /^\/([^/]+\/[^/]+)\/pull\/(\d+)/;
const LABEL = "Android Studio ▾";
const ACTIONS = [
  { action: "open", label: "Open in Android Studio", busy: "Opening…", done: "Opened ✓" },
  { action: "phone", label: "Run on phone", busy: "Preparing…", done: "Building for phone…" },
  { action: "emulator", label: "Run on emulator", busy: "Preparing…", done: "Building for emulator…" },
  { action: "claude", label: "Open Claude session", busy: "Opening…", done: "Claude opened ✓" },
];

function addMenuStyle() {
  if (document.getElementById(`${MENU_ID}-style`)) return;
  const style = document.createElement("style");
  style.id = `${MENU_ID}-style`;
  style.textContent = `
    #${MENU_ID} { position: relative; display: inline-block; margin-top: 4px; }
    #${MENU_ID} > summary { list-style: none; }
    #${MENU_ID} > summary::-webkit-details-marker { display: none; }
    #${MENU_ID} [role=menu] { position: absolute; z-index: 100; top: calc(100% + 4px); left: 0; min-width: 210px; padding: 4px;
      background: var(--overlay-bgColor, var(--bgColor-default, #fff)); border: 1px solid var(--borderColor-default, #d0d7de);
      border-radius: 12px; box-shadow: var(--shadow-floating-small, 0 8px 24px rgba(0,0,0,.15)); }
    #${MENU_ID} [role=menuitem] { display: block; width: 100%; text-align: left; padding: 6px 8px; border: 0; border-radius: 6px;
      background: none; color: inherit; font: inherit; cursor: pointer; }
    #${MENU_ID} [role=menuitem]:hover, #${MENU_ID} [role=menuitem]:focus-visible { background: var(--control-transparent-bgColor-hover, rgba(0,0,0,.06)); }`;
  document.head.append(style);
}

function renderPrMenu() {
  const m = location.pathname.match(PR_PATH);
  const key = m && `${m[1]}#${m[2]}`;
  const anchor = document.querySelector('[data-component="PageHeader.Description"]');
  const existing = document.getElementById(MENU_ID);
  if (existing && existing.dataset.pr === key && anchor?.contains(existing)) return;
  existing?.remove();
  if (!m || !anchor) return;
  addMenuStyle();

  const menu = document.createElement("details");
  menu.id = MENU_ID;
  menu.dataset.pr = key;
  const summary = document.createElement("summary");
  summary.className = "btn btn-sm";
  summary.textContent = LABEL;
  summary.setAttribute("aria-haspopup", "menu");
  const list = document.createElement("div");
  list.setAttribute("role", "menu");

  for (const a of ACTIONS) {
    const item = document.createElement("button");
    item.type = "button";
    item.setAttribute("role", "menuitem");
    item.textContent = a.label;
    item.addEventListener("click", () => {
      menu.open = false;
      summary.textContent = a.busy;
      chrome.runtime.sendMessage({ repo: m[1], pr: Number(m[2]), action: a.action }, (res) => {
        const err = chrome.runtime.lastError?.message ?? (res?.ok ? null : res?.error ?? "No response from extension");
        summary.textContent = err ? LABEL : a.done;
        summary.title = err ?? [res.path, res.log && `Log: ${res.log}`].filter(Boolean).join("\n");
        if (err) alert(`${a.label} failed:\n${err}`);
        else setTimeout(() => (summary.textContent = LABEL), 4000);
      });
    });
    list.append(item);
  }
  menu.append(summary, list);
  anchor.append(menu);
}

// Close the menu on outside click or Escape.
document.addEventListener("click", (e) => {
  const menu = document.getElementById(MENU_ID);
  if (menu?.open && !menu.contains(e.target)) menu.open = false;
});
document.addEventListener("keydown", (e) => {
  const menu = document.getElementById(MENU_ID);
  if (e.key === "Escape" && menu?.open) {
    menu.open = false;
    menu.querySelector("summary").focus();
  }
});

function tick() {
  if (SHOW_FILTERS) render();
  renderPrMenu();
}

tick();
new MutationObserver(tick).observe(document.body, { childList: true, subtree: true });
