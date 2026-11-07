const status = document.getElementById("status");
try {
  const port = chrome.runtime.connectNative("rget");
  port.onMessage.addListener((msg) => {
    status.textContent = `Active: ${msg.count || 0}`;
  });
  port.postMessage({ op: "status" });
} catch (e) {
  status.textContent = "Not connected";
}
