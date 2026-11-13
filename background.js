// Content scripts can't use native messaging, so they relay through here. The worktree folder, IDE and emulator
// settings ride along with every request.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  chrome.storage.local.get(["worktreeDir", "ide", "emulator"], (stored) => {
    const request = { ...msg, worktreeDir: stored?.worktreeDir || "", ide: stored?.ide || "", emulator: stored?.emulator || "" };
    chrome.runtime.sendNativeMessage("github_pr_quick_filters", request, (res) => {
      sendResponse(chrome.runtime.lastError ? { ok: false, error: chrome.runtime.lastError.message } : res);
    });
  });
  return true; // keep sendResponse alive for the async reply
});

// Without a popup, clicking the toolbar / extensions-menu entry fires this.
chrome.action.onClicked.addListener(() => chrome.runtime.openOptionsPage());
