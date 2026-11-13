const input = document.getElementById("dir");
const browse = document.getElementById("browse");
// Setting (its <select> id) -> the list of choices the native host detects.
const PICKS = { ide: "ides", iosIde: "iosIdes", emulator: "avds", simulator: "simulators" };
const status = document.getElementById("status");

function show(text, isError = false) {
  status.textContent = text;
  status.classList.toggle("error", isError);
}

function save() {
  const worktreeDir = input.value.trim();
  chrome.storage.local.set({ worktreeDir }, () => show(worktreeDir ? "Saved" : "Saved: not set"));
}

chrome.storage.local.get("worktreeDir", (stored) => { input.value = stored?.worktreeDir || ""; });
input.addEventListener("change", save);

chrome.runtime.sendNativeMessage("github_pr_quick_filters", { action: "detect" }, (res) => {
  if (chrome.runtime.lastError) return show(chrome.runtime.lastError.message, true);
  if (!res?.ok) return show(res?.error ?? "No response from the native host", true);
  chrome.storage.local.get(Object.keys(PICKS), (stored) => {
    for (const [key, list] of Object.entries(PICKS)) {
      document.getElementById(key).append(...res[list].map((name) => new Option(name, name, false, name === stored?.[key])));
    }
  });
});
for (const key of Object.keys(PICKS)) {
  const select = document.getElementById(key);
  select.addEventListener("change", () => chrome.storage.local.set({ [key]: select.value }, () => show(select.value ? "Saved" : "Saved: not chosen")));
}

// The browser's own pickers never reveal an absolute path, so the native host shows the macOS folder dialog.
browse.addEventListener("click", () => {
  browse.disabled = true;
  chrome.runtime.sendNativeMessage("github_pr_quick_filters", { action: "choose_folder", current: input.value.trim() }, (res) => {
    browse.disabled = false;
    if (chrome.runtime.lastError) return show(chrome.runtime.lastError.message, true);
    if (res?.cancelled) return;
    if (!res?.ok) return show(res?.error ?? "No response from the native host", true);
    input.value = res.path;
    save();
  });
});
