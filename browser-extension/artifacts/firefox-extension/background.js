const NATIVE_HOST = "rget";

browser.downloads.onCreated.addListener(async (item) => {
  if (!item.url || item.url.startsWith("blob:") || item.url.startsWith("data:")) {
    return;
  }

  try {
    const port = browser.runtime.connectNative(NATIVE_HOST);
    port.onDisconnect.addListener(() => {
      if (browser.runtime.lastError) {
        console.error("rget native host disconnected:", browser.runtime.lastError.message);
      }
    });

    port.postMessage({
      op: "add_task",
      url: item.url,
      referer: item.referrer || "",
      filename: item.filename || ""
    });

    await browser.downloads.cancel(item.id);
    await browser.downloads.erase({ id: item.id });
  } catch (err) {
    console.error("Failed to hand off download to rget:", err);
  }
});

browser.contextMenus.create({
  id: "rget-download",
  title: "Download with rget",
  contexts: ["link"]
});

browser.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId === "rget-download" && info.linkUrl) {
    const port = browser.runtime.connectNative(NATIVE_HOST);
    port.postMessage({
      op: "add_task",
      url: info.linkUrl,
      referer: info.pageUrl || "",
      filename: ""
    });
  }
});