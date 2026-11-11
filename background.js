// Content scripts can't use native messaging, so they relay through here. The settings (worktree folder, Claude
// model and effort) ride along with every request.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  chrome.storage.local.get(["worktreeDir", "claudeModel", "claudeEffort"], (stored) => {
    const request = {
      ...msg,
      worktreeDir: stored?.worktreeDir || "",
      claudeModel: stored?.claudeModel || "default",
      claudeEffort: stored?.claudeEffort || "default",
    };
    chrome.runtime.sendNativeMessage("github_pr_quick_filters", request, (res) => {
      sendResponse(chrome.runtime.lastError ? { ok: false, error: chrome.runtime.lastError.message } : res);
    });
  });
  return true; // keep sendResponse alive for the async reply
});
