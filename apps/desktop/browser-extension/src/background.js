const HOST_NAME = "com.shaikzeeshan.mnema.browser_integration";
let port = null;
let browserFamily = "chromium";
let pairingToken = "";
let requestSequence = 0;
const pendingRequests = new Map();

chrome.storage.local.get(["pairingToken", "browserFamily"]).then((stored) => {
  pairingToken = stored.pairingToken || "";
  browserFamily = stored.browserFamily || browserFamily;
});

function connect() {
  if (port) return port;
  port = chrome.runtime.connectNative(HOST_NAME);
  port.onMessage.addListener((message) => {
    const requestId = message?.requestId;
    if (!requestId || !pendingRequests.has(requestId)) return;
    pendingRequests.get(requestId)(message);
    pendingRequests.delete(requestId);
  });
  port.onDisconnect.addListener(() => {
    for (const resolve of pendingRequests.values()) {
      resolve({ ok: false, error: chrome.runtime.lastError?.message || "native_host_disconnected" });
    }
    pendingRequests.clear();
    port = null;
  });
  return port;
}

function postNativeMessage(message) {
  return new Promise((resolve) => {
    const requestId = `req-${Date.now()}-${requestSequence += 1}`;
    const nativePort = connect();
    pendingRequests.set(requestId, resolve);
    nativePort.postMessage({ ...message, requestId });
    setTimeout(() => {
      if (!pendingRequests.has(requestId)) return;
      pendingRequests.delete(requestId);
      resolve({ ok: false, error: "native_host_timeout" });
    }, 3000);
  });
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.channel === "pair" && typeof message.pairingToken === "string") {
    pairingToken = message.pairingToken;
    if (message.browserFamily === "safari" || message.browserFamily === "chromium") {
      browserFamily = message.browserFamily;
    }
    chrome.storage.local.set({ pairingToken, browserFamily });
    if (pairingToken) {
      postNativeMessage({
        channel: "pair",
        pairingToken,
        signal: {
          version: 1,
          kind: "browser_secure_entry_signal",
          browserFamily,
          state: "clear",
          reason: "no_focused_credential_control",
          observedAtUnixMs: Date.now(),
          sequence: Date.now()
        }
      }).then(sendResponse);
      return true;
    } else {
      sendResponse({ ok: true });
      return false;
    }
  }
  if (!message || message.channel !== "secureEntry" || !message.signal) {
    return false;
  }
  const signal = {
    ...message.signal,
    browserFamily
  };
  postNativeMessage({ channel: "secureEntry", pairingToken, signal });
  return false;
});

chrome.tabs?.onActivated?.addListener(async ({ tabId }) => {
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab?.url) return;
  postNativeMessage({
    channel: "metadata",
    pairingToken,
    signal: {
      version: 1,
      kind: "browser_metadata_signal",
      browserFamily,
      state: "available",
      reason: "active_tab",
      observedAtUnixMs: Date.now(),
      sequence: Date.now(),
      url: tab.url
    }
  });
});
