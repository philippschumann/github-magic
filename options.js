const input = document.getElementById("dir");
const browse = document.getElementById("browse");
const status = document.getElementById("status");

function show(text, isError = false) {
  status.textContent = text;
  status.classList.toggle("error", isError);
}

function save() {
  const worktreeDir = input.value.trim();
  chrome.storage.local.set({ worktreeDir }, () => show(worktreeDir ? "Saved" : "Saved: not set"));
}

const model = document.getElementById("model");
const effort = document.getElementById("effort");

chrome.storage.local.get(["worktreeDir", "claudeModel", "claudeEffort"], (stored) => {
  input.value = stored?.worktreeDir || "";
  fillSelect(model, CLAUDE_MODELS, stored?.claudeModel || "default");
  fillSelect(effort, CLAUDE_EFFORTS, stored?.claudeEffort || "default");
});
input.addEventListener("change", save);
// Saving both marks the Claude choice as made, so the PR page stops asking on first click.
for (const select of [model, effort]) {
  select.addEventListener("change", () =>
    chrome.storage.local.set({ claudeModel: model.value, claudeEffort: effort.value }, () => show("Saved")));
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
