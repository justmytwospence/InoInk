// Where the sign-in redirect page may send an Inoreader answer: only back to the computer or the
// Kobo that started the sign-in, named in the OAuth state as <32 hex>_<host>_<port>.

const STATE = /^[0-9a-f]{32}_(localhost|\d{1,3}(?:\.\d{1,3}){3})_(\d{1,5})$/;

function private_(host) {
  if (host === "localhost") return true;
  const o = host.split(".").map(Number);
  if (o.length !== 4 || o.some((n) => !(n >= 0 && n <= 255))) return false;
  return o[0] === 127 || o[0] === 10 || (o[0] === 192 && o[1] === 168) || (o[0] === 172 && o[1] >= 16 && o[1] <= 31);
}

/** The local address to hand the answer to, or null when it must not be forwarded. */
export function relayTarget(search) {
  const q = new URLSearchParams(search);
  const m = STATE.exec(q.get("state") || "");
  if (!m || !(q.has("code") || q.has("error"))) return null;
  const [, host, p] = m;
  const port = Number(p);
  if (!private_(host) || port < 1 || port > 65535) return null;
  return `http://${host}:${port}/callback${search}`;
}
