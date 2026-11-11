const NATIVE_HOST = "rget";

// Only hand off explicitly requested links. Cancelling a browser download before
// the daemon has actually completed it risks losing the user's download.
browser.contextMenus.create({
  id: "rget-download",
  title: "Download with rget",
  contexts: ["link"]
});

browser.contextMenus.onClicked.addListener(async (info) => {
  if (info.menuItemId !== "rget-download" || !info.linkUrl) return;
  try {
    const reply = await browser.runtime.sendNativeMessage(NATIVE_HOST, { op: "add_task", url: info.linkUrl });
    if (reply.event === "task_failed") console.error("rget rejected download:", reply.error);
  } catch (err) {
    console.error("Failed to hand off download to rget:", err);
  }
});
