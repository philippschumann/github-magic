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

// PR page actions: an "Android Studio" dropdown and a "Claude" button. The native host (native-host/) checks the
// PR out in a worktree and acts on it.
const MENU_ID = "pr-open-locally";
const PR_PATH = /^\/([^/]+\/[^/]+)\/pull\/(\d+)/;
const STUDIO_LABEL = "Android Studio";
const ACTIONS = [
  { action: "open", label: "Open in Android Studio", busy: "Opening…", done: "Opened ✓" },
  // Phone/emulator runs continue in the background; the button polls until the app is installed and started.
  { action: "phone", label: "Run on phone", busy: "Installing…", done: "Installed ✓", poll: true },
  { action: "emulator", label: "Run on emulator", busy: "Installing…", done: "Installed ✓", poll: true },
];
const CLAUDE_LABEL = "Claude";
const CLAUDE = { action: "claude", label: "Open Claude session", busy: "Opening…", done: "Opened ✓" };
const POLL_MS = 3000;
const TRIANGLE_DOWN = '<svg aria-hidden="true" focusable="false" viewBox="0 0 16 16" width="16" height="16" fill="currentColor"><path d="m4.427 7.427 3.396 3.396a.25.25 0 0 0 .354 0l3.396-3.396A.25.25 0 0 0 11.396 7H4.604a.25.25 0 0 0-.177.427Z"></path></svg>';

// GitHub's React buttons use hashed class names that change between deploys; borrow the current ones from the page.
function primerClass(prefix) {
  const el = document.querySelector(`[class*="${prefix}"]`);
  return el ? [...el.classList].find((c) => c.startsWith(prefix)) : null;
}

function makeButton(text, caret = false) {
  const btn = document.createElement("button");
  btn.type = "button";
  const label = document.createElement("span");
  label.textContent = text;
  const base = primerClass("prc-Button-ButtonBase-");
  if (base) {
    btn.className = base;
    Object.assign(btn.dataset, { component: "Button", loading: "false", size: "small", variant: "default" });
    const content = document.createElement("span");
    content.className = primerClass("prc-Button-ButtonContent-") ?? "";
    Object.assign(content.dataset, { component: "buttonContent", align: "center" });
    label.className = primerClass("prc-Button-Label-") ?? "";
    label.dataset.component = "text";
    content.append(label);
    btn.append(content);
  } else {
    btn.className = "btn btn-sm"; // pages without GitHub's React buttons: classic Primer CSS
    btn.append(label);
  }
  if (caret) {
    const icon = document.createElement("span");
    icon.className = primerClass("prc-Button-TrailingAction-") ?? "";
    icon.dataset.component = "trailingAction";
    icon.innerHTML = TRIANGLE_DOWN;
    btn.append(icon);
  }
  btn.label = label;
  return btn;
}

function addMenuStyle() {
  if (document.getElementById(`${MENU_ID}-style`)) return;
  const style = document.createElement("style");
  style.id = `${MENU_ID}-style`;
  // Mirrors GitHub's ActionMenu overlay with its own design tokens, so it follows light and dark themes.
  style.textContent = `
    #${MENU_ID} { position: relative; display: inline-flex; gap: 8px; align-items: center; margin-top: 8px; }
    #${MENU_ID} > button { display: inline-flex; align-items: center; gap: 4px; }
    #${MENU_ID} > button [data-component] { display: inline-flex; align-items: center; }
    #${MENU_ID} > button svg { display: block; }
    #${MENU_ID} [role=menu] { position: absolute; z-index: 100; top: calc(100% + 4px); left: 0; min-width: 192px; padding: 8px 0;
      background: var(--overlay-bgColor, #fff); border-radius: var(--borderRadius-large, 12px);
      box-shadow: var(--shadow-floating-small, 0 0 0 1px #d1d9e080, 0 6px 12px -3px #25292e0a, 0 6px 18px 0 #25292e1f); }
    #${MENU_ID} [role=menu][hidden] { display: none; }
    #${MENU_ID} [role=menuitem] { display: flex; align-items: center; box-sizing: border-box; min-height: 32px;
      width: calc(100% - 16px); margin: 0 8px; padding: 6px 8px; border: 0;
      border-radius: var(--borderRadius-medium, 6px); background: transparent; color: var(--fgColor-default, inherit);
      font: inherit; font-size: var(--text-body-size-medium, 14px); line-height: 20px; text-align: left; cursor: pointer; }
    #${MENU_ID} [role=menuitem]:hover, #${MENU_ID} [role=menuitem]:focus-visible {
      background: var(--control-transparent-bgColor-hover, #818b981a); outline: none; }`;
  document.head.append(style);
}

function send(msg, cb) {
  chrome.runtime.sendMessage(msg, (res) => {
    cb(res, chrome.runtime.lastError?.message ?? (res?.ok ? null : res?.error ?? "No response from extension"));
  });
}

// Runs one action; `btn` shows its progress and returns to `idle` afterwards. A newer click on the same button wins.
function runAction(m, a, btn, idle) {
  const token = (btn.run = {});
  const show = (text, title = "") => {
    if (btn.run !== token) return;
    btn.label.textContent = text;
    btn.title = title;
  };
  const fail = (err) => {
    if (btn.run !== token) return;
    show(idle, err);
    alert(`${a.label} failed:\n${err}`);
  };
  const finish = (title) => {
    show(a.done, title);
    setTimeout(() => show(idle, title), 4000);
  };
  show(a.busy);
  send({ repo: m[1], pr: Number(m[2]), action: a.action }, (res, err) => {
    if (err) return fail(err);
    if (!a.poll) return finish(res.path);
    const poll = () => send({ action: "status", log: res.log }, (st, err2) => {
      if (btn.run !== token) return;
      if (err2) return fail(err2);
      if (st.state === "running") return setTimeout(poll, POLL_MS);
      if (st.state === "started") return finish(st.message);
      if (st.state === "cancelled") return show(idle, "Cancelled: a newer run took over");
      fail(`${st.message}\nLog: ${res.log}`);
    });
    setTimeout(poll, POLL_MS);
  });
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

  const root = document.createElement("div");
  root.id = MENU_ID;
  root.dataset.pr = key;

  const studio = makeButton(STUDIO_LABEL, true);
  studio.setAttribute("aria-haspopup", "menu");
  studio.setAttribute("aria-expanded", "false");
  const list = document.createElement("div");
  list.setAttribute("role", "menu");
  list.setAttribute("aria-label", STUDIO_LABEL);
  list.hidden = true;
  studio.addEventListener("click", () => setMenuOpen(list.hidden));
  for (const a of ACTIONS) {
    const item = document.createElement("button");
    item.type = "button";
    item.setAttribute("role", "menuitem");
    item.textContent = a.label;
    item.addEventListener("click", () => {
      setMenuOpen(false);
      runAction(m, a, studio, STUDIO_LABEL);
    });
    list.append(item);
  }

  const claude = makeButton(CLAUDE_LABEL);
  claude.setAttribute("aria-label", CLAUDE.label);
  claude.addEventListener("click", () => runAction(m, CLAUDE, claude, CLAUDE_LABEL));

  root.append(studio, claude, list);
  anchor.append(root);
}

function setMenuOpen(open) {
  const button = document.querySelector(`#${MENU_ID} [aria-haspopup=menu]`);
  const list = document.querySelector(`#${MENU_ID} [role=menu]`);
  if (!button || !list) return;
  list.hidden = !open;
  button.setAttribute("aria-expanded", String(open));
}

// Close the menu on outside click or Escape. Capture phase: GitHub's own handlers stop some clicks from bubbling.
document.addEventListener("click", (e) => {
  const button = document.querySelector(`#${MENU_ID} [aria-haspopup=menu]`);
  const list = document.querySelector(`#${MENU_ID} [role=menu]`);
  if (list && !list.hidden && !list.contains(e.target) && !button.contains(e.target)) setMenuOpen(false);
}, true);
document.addEventListener("keydown", (e) => {
  const list = document.querySelector(`#${MENU_ID} [role=menu]`);
  if (e.key === "Escape" && list && !list.hidden) {
    setMenuOpen(false);
    document.querySelector(`#${MENU_ID} [aria-haspopup=menu]`).focus();
  }
});

function tick() {
  if (SHOW_FILTERS) render();
  renderPrMenu();
}

tick();
new MutationObserver(tick).observe(document.body, { childList: true, subtree: true });
