// The only part of the settings page that differs between the app's Settings window and the extension's options
// page. Both reach the same handler in the app.
const inApp = "__TAURI__" in window;

// Sends one request to the app and resolves with its reply.
function call(msg) {
  if (inApp) return window.__TAURI__.core.invoke("settings", { msg });
  // Through background.js, which adds the protocol version and reports a missing app as `missing`.
  return new Promise((resolve) => chrome.runtime.sendMessage(msg, (res) => {
    resolve(res ?? { ok: false, error: chrome.runtime.lastError?.message ?? "No response from the extension" });
  }));
}

// Calls `reload` when the settings may have changed somewhere else: the app says so, a browser tab can't hear that
// and looks again whenever it is shown.
function onChange(reload) {
  if (inApp) window.__TAURI__.event.listen("settings-changed", reload);
  else document.addEventListener("visibilitychange", () => document.visibilityState === "visible" && reload());
}
