const port = chrome.runtime.connectNative("rget");
port.onMessage.addListener((msg) => console.log(msg));
port.onDisconnect.addListener(() => console.log("disconnected"));
export function send(msg) { port.postMessage(msg); }
