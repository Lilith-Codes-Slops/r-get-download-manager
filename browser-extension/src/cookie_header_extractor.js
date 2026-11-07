async function extract(url) {
  const cookies = await chrome.cookies.getAll({ url });
  return cookies.map(c => `${c.name}=${c.value}`).join("; ");
}
