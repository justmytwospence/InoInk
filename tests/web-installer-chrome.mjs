// Drives the project page's installer in headless Chrome, with a fake Kobo in the browser's
// origin-private file system standing in for the USB drive (a page cannot be handed a real drive
// without the native folder picker). Checks the real browser APIs, the GitHub release lookup and
// download (CORS), checksum verification, the files written and removal. Not part of CI: it needs
// Chrome and the network.
//   node tests/web-installer-chrome.mjs <published inoink-<version>-kobo.tar.gz> [screenshot-dir]
// CHROME=/path/to/chrome overrides the browser.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PKG = path.resolve(process.argv[2] || "");
const SHOTS = process.argv[3] ? path.resolve(process.argv[3]) : null;
if (!fs.existsSync(PKG)) {
  console.error("usage: node tests/web-installer-chrome.mjs <inoink-<version>-kobo.tar.gz> [screenshot-dir]");
  process.exit(2);
}
const CHROME = process.env.CHROME || ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].find((p) => fs.existsSync(p));

let fails = 0;
const check = (name, ok, extra = "") => {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${extra ? ` (${extra})` : ""}`);
  if (!ok) fails++;
};

// Serve site/ like GitHub Pages.
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".webp": "image/webp" };
const server = http.createServer((req, res) => {
  const p = path.join(ROOT, "site", decodeURIComponent(new URL(req.url, "http://x").pathname).replace(/\/$/, "/index.html"));
  if (!p.startsWith(path.join(ROOT, "site")) || !fs.existsSync(p)) return res.writeHead(404).end();
  res.writeHead(200, { "Content-Type": TYPES[path.extname(p)] || "application/octet-stream" }).end(fs.readFileSync(p));
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const SITE = `http://localhost:${server.address().port}/`;

const profile = fs.mkdtempSync(path.join(os.tmpdir(), "inoink-chrome-"));
const chrome = spawn(CHROME, ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "--window-size=1200,1600", "about:blank"], { stdio: "ignore" });
let port;
for (let i = 0; i < 100 && !port; i++) {
  await new Promise((r) => setTimeout(r, 100));
  try {
    port = fs.readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
  } catch {}
}
const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const ws = new WebSocket(targets.find((t) => t.type === "page").webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let nextId = 0;
const pending = new Map();
const console_ = [];
ws.addEventListener("message", (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  } else if (msg.method === "Runtime.consoleAPICalled") {
    console_.push(`${msg.params.type}: ${msg.params.args.map((a) => a.value ?? a.description).join(" ")}`);
  } else if (msg.method === "Log.entryAdded") {
    console_.push(`log: ${msg.params.entry.text.replace(/\?[^ ']*/g, "?...")}`);
  } else if (msg.method === "Runtime.exceptionThrown") {
    console_.push(`exception: ${msg.params.exceptionDetails.exception?.description}`);
  }
});
const cdp = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, (msg) => (msg.error ? reject(new Error(`${method}: ${msg.error.message}`)) : resolve(msg.result)));
    ws.send(JSON.stringify({ id, method, params }));
  });
const js = async (expression) => {
  const r = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
  return r.result.value;
};
const visibleStep = () => js(`[...document.querySelectorAll("#installer [data-step]")].find((e) => !e.hidden)?.dataset.step`);
const waitStep = async (want, ms = 120000) => {
  const end = Date.now() + ms;
  let s;
  while (Date.now() < end) {
    s = await visibleStep();
    if ([].concat(want).includes(s)) return s;
    await new Promise((r) => setTimeout(r, 200));
  }
  return s;
};
const shot = async (name) => {
  if (!SHOTS) return;
  fs.mkdirSync(SHOTS, { recursive: true });
  await js(`document.getElementById("installer").scrollIntoView({ behavior: "instant" })`);
  const { data } = await cdp("Page.captureScreenshot", { format: "png" });
  fs.writeFileSync(path.join(SHOTS, `${name}.png`), Buffer.from(data, "base64"));
};
const click = (selector) => js(`document.querySelector(${JSON.stringify(selector)}).click()`);
const text = (f) => js(`document.querySelector('#installer [data-field="${f}"]').textContent`);

// The fake Kobo, in the origin-private file system.
const OPFS = `
  async function opfsPath(parts, create) {
    let d = await navigator.storage.getDirectory();
    for (const p of parts) d = await d.getDirectoryHandle(p, { create });
    return d;
  }
  async function opfsTree(d, pre = "") {
    d ??= await navigator.storage.getDirectory();
    const out = {};
    for await (const [name, h] of d.entries()) {
      if (h.kind === "directory") Object.assign(out, { [pre + name + "/"]: "" }, await opfsTree(h, pre + name + "/"));
      else out[pre + name] = (await h.getFile()).size;
    }
    return out;
  }
`;

try {
  await cdp("Runtime.enable");
  await cdp("Log.enable");
  await cdp("Page.enable");
  await cdp("Page.navigate", { url: SITE });
  await new Promise((r) => setTimeout(r, 1500));
  await js(OPFS.replace(/async function (\w+)/g, "window.$1 = async function $1"));

  check("page loads with the installer ready", (await visibleStep()) === "intro");
  check("Chrome counts as supported", await js(`!document.querySelector('[data-action="pick"]').disabled`));
  const shown = await js(`[...document.querySelectorAll(".device-groups li")].filter((l) => l.offsetParent).length`);
  const all = await js(`document.querySelectorAll(".device-groups li").length`);
  await click('[data-action="all-devices"]');
  const after = await js(`[...document.querySelectorAll(".device-groups li")].filter((l) => l.offsetParent).length`);
  check("device list shows some, then all", shown > 0 && shown < all && after === all, `${shown} -> ${after} of ${all}`);

  // A folder that is not a Kobo.
  await js(`(async () => { const r = await navigator.storage.getDirectory(); for await (const k of r.keys()) await r.removeEntry(k, { recursive: true }); })()`);
  await js(`(async () => { const m = await import("./page.js"); await m.connect(await opfsPath(["notakobo"], true)); })()`);
  check("a folder that is not a Kobo is refused", (await js(`document.querySelector('[data-show="pick-error"]').textContent`)).includes("not the root of a Kobo"));

  await js(`(async () => {
    const r = await navigator.storage.getDirectory();
    await r.removeEntry("notakobo", { recursive: true });
    const w = await (await (await opfsPath([".kobo"], true)).getFileHandle("version", { create: true })).createWritable();
    await w.write("N428000000000,4.9.77,4.46.23852,4.9.77,4.9.77,00000000-0000-0000-0000-000000000390\\n");
    await w.close();
    await opfsPath([".adds", "nm"], true);
    const m = await import("./page.js");
    await m.connect(r);
  })()`);
  check("the Kobo is recognised", (await visibleStep()) === "device" && (await text("model")) === "Kobo Libra Colour", await text("model"));
  check("firmware shown", (await text("firmware")) === "4.46.23852");
  const t0 = Date.now();
  while (Date.now() - t0 < 15000 && (await text("latest")).startsWith("Checking")) await new Promise((r) => setTimeout(r, 200));
  const latest = await text("latest");
  check("latest release read from GitHub", /^InoInk \d+\.\d+\.\d+/.test(latest), latest);
  check("install button names the release", (await js(`document.querySelector('[data-action="install"]').textContent`)).startsWith("Install InoInk "));
  await shot("1-device");

  await click('[data-action="install"]');
  const s = await waitStep(["package", "done", "error"]);
  check("GitHub's asset download is blocked by CORS, so the page asks for the file", s === "package", s);
  check("Chrome reports the CORS block", console_.some((l) => l.includes("blocked by CORS policy")));
  await shot("2-package");

  if (s === "package") {
    const { root } = await cdp("DOM.getDocument");
    const { nodeId } = await cdp("DOM.querySelector", { nodeId: root.nodeId, selector: "#package-file" });
    await cdp("DOM.setFileInputFiles", { nodeId, files: [PKG] });
    const t = Date.now();
    const done = await waitStep(["done", "error"]);
    check("installs from the chosen file", done === "done", done === "error" ? await text("error-message") : `${Date.now() - t} ms`);
    await shot("3-done");
    const tree = await js(`opfsTree()`);
    const size = fs.statSync(PKG).size;
    check("INSTALL.TGZ is the package", tree[".adds/inoink/INSTALL.TGZ"] === size);
    check("launcher written", tree[".adds/inoink/current/inoink.sh"] > 0);
    check("NickelMenu entry written", tree[".adds/nm/inoink"] > 0);
    check("data folder created", ".adds/inoink/data/" in tree);
    check("no swap files left", !Object.keys(tree).some((k) => k.endsWith(".crswap")));
    const sum = await js(`(async () => (await (await (await opfsPath([".adds", "inoink"])).getFileHandle("INSTALL.SUM")).getFile()).text())()`);
    check("INSTALL.SUM is what cksum prints", sum === execFileSync("cksum", { input: fs.readFileSync(PKG) }).toString(), sum.trim());
    const sha = await js(`(async () => { const b = await (await (await (await opfsPath([".adds", "inoink"])).getFileHandle("INSTALL.TGZ")).getFile()).arrayBuffer(); return [...new Uint8Array(await crypto.subtle.digest("SHA-256", b))].map((x) => x.toString(16).padStart(2, "0")).join(""); })()`);
    check("the copy is byte-identical", sha === execFileSync("shasum", ["-a", "256", PKG]).toString().split(" ")[0]);
    console.log("     files:", JSON.stringify(tree));

    // A file that is not the release.
    const bad = path.join(profile, path.basename(PKG));
    const bytes = fs.readFileSync(PKG);
    bytes[bytes.length - 100] ^= 1;
    fs.writeFileSync(bad, bytes);
    await click('#installer [data-step="done"] [data-action="restart"]');
    await js(`(async () => { const m = await import("./page.js"); await m.connect(await navigator.storage.getDirectory()); })()`);
    check("an installed but not yet started package is reported", (await text("installed")).includes("copied earlier"), await text("installed"));
    await click('[data-action="install"]');
    await waitStep("package");
    const { root: r2 } = await cdp("DOM.getDocument");
    const { nodeId: n2 } = await cdp("DOM.querySelector", { nodeId: r2.nodeId, selector: "#package-file" });
    await cdp("DOM.setFileInputFiles", { nodeId: n2, files: [bad] });
    const s2 = await waitStep(["done", "error"]);
    check("a modified package is refused", s2 === "error", await text("error-message"));

    // Removal, keeping data, then everything.
    await click('#installer [data-step="error"] [data-action="restart"]');
    await js(`(async () => {
      const w = await (await (await opfsPath([".adds", "inoink", "data"])).getFileHandle("inoink.db", { create: true })).createWritable();
      await w.write("db"); await w.close();
      const m = await import("./page.js"); await m.connect(await navigator.storage.getDirectory());
    })()`);
    await click('[data-action="remove"]');
    await shot("4-remove");
    await click('[data-action="remove-confirm"]');
    await waitStep(["done", "error"]);
    let t2 = await js(`opfsTree()`);
    check("remove (keep data) leaves only the data", !t2[".adds/inoink/INSTALL.TGZ"] && !t2[".adds/nm/inoink"] && !(".adds/inoink/current/" in t2) && t2[".adds/inoink/data/inoink.db"] === 2, JSON.stringify(t2));
    await click('#installer [data-step="done"] [data-action="restart"]');
    await js(`(async () => { const m = await import("./page.js"); await m.connect(await navigator.storage.getDirectory()); })()`);
    await click('[data-action="remove"]');
    await click('input[name="remove-level"][value="all"]');
    await click('[data-action="remove-confirm"]');
    await waitStep(["done", "error"]);
    t2 = await js(`opfsTree()`);
    check("remove (everything) deletes .adds/inoink", !(".adds/inoink/" in t2) && ".adds/nm/" in t2, JSON.stringify(t2));
  }
  console.log("     console:", console_.join(" | ") || "(empty)");
} catch (e) {
  check("browser run", false, e.message);
} finally {
  ws.close();
  await new Promise((r) => {
    chrome.once("exit", r);
    chrome.kill();
  });
  server.close();
  try {
    fs.rmSync(profile, { recursive: true, force: true });
  } catch {}
}
console.log(fails ? `${fails} browser check(s) FAILED` : "browser checks passed");
process.exit(fails ? 1 : 0);
