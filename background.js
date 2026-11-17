// Content scripts can't use native messaging, so they relay through here, and so does the settings page. Every
// request carries the protocol version: it changes only when the messages change incompatibly.
const PROTOCOL = 1;
// What the browsers say when the app, and with it the native messaging host, isn't installed.
const MISSING = /native messaging host not found|failed to start native messaging host|no such native application/i;
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  chrome.runtime.sendNativeMessage("github_magic", { ...msg, protocol: PROTOCOL }, (res) => {
    const error = chrome.runtime.lastError?.message;
    sendResponse(error ? { ok: false, error, missing: MISSING.test(error) } : res);
  });
  return true; // keep sendResponse alive for the async reply
});

// Without a popup, clicking the toolbar / extensions-menu entry fires this.
chrome.action.onClicked.addListener(() => chrome.runtime.openOptionsPage());
