const input = document.getElementById("dir");
const browse = document.getElementById("browse");
const status = document.getElementById("status");

function show(text, isError = false) {
  status.textContent = text;
  status.classList.toggle("error", isError);
}

function save() {
  const worktreeDir = input.value.trim();
  chrome.storage.local.set({ worktreeDir }, () => show(worktreeDir ? "Saved" : "Saved: next to the main clone"));
}

chrome.storage.local.get("worktreeDir", (stored) => (input.value = stored?.worktreeDir || ""));
input.addEventListener("change", save);

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
