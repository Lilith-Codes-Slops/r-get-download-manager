chrome.contextMenus.onClicked.addListener((info) => {
  if (info.linkUrl) {
    chrome.runtime.sendNativeMessage("rget", { action: "add", url: info.linkUrl });
  }
});
