chrome.downloads.onCreated.addListener((item) => {
  chrome.runtime.sendNativeMessage("rget", { url: item.url, filename: item.filename });
});
