chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "download-with-rget",
    title: "Download with rget",
    contexts: ["link"]
  });
});

chrome.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId === "download-with-rget" && info.linkUrl) {
    chrome.runtime.sendNativeMessage("rget", { url: info.linkUrl });
  }
});
