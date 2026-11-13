// Content scripts can't use native messaging, so they relay through here. The settings ride along with every request.
const SETTINGS = ["worktreeDir", "ide", "iosIde", "emulator", "simulator"];
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  chrome.storage.local.get(SETTINGS, (stored) => {
    const request = { ...msg, ...Object.fromEntries(SETTINGS.map((key) => [key, stored?.[key] || ""])) };
    chrome.runtime.sendNativeMessage("github_pr_quick_filters", request, (res) => {
      sendResponse(chrome.runtime.lastError ? { ok: false, error: chrome.runtime.lastError.message } : res);
    });
  });
  return true; // keep sendResponse alive for the async reply
});

// Without a popup, clicking the toolbar / extensions-menu entry fires this.
chrome.action.onClicked.addListener(() => chrome.runtime.openOptionsPage());
