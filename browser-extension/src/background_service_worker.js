chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "download-with-rget",
    title: "Download with rget",
    contexts: ["link"]
  });
});

chrome.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId === "download-with-rget" && info.linkUrl) {
    chrome.runtime.sendNativeMessage("rget", { op: "add_task", url: info.linkUrl }, (reply) => {
      if (chrome.runtime.lastError) console.error("rget:", chrome.runtime.lastError.message);
      else if (reply && reply.event === "task_failed") console.error("rget:", reply.error);
    });
  }
});
