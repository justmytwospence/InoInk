// InoInk's browser installer: everything that touches the Kobo or the package, without the page.
//
// Install writes exactly what scripts/install-inoink.sh writes, in the same order: the package as
// .adds/inoink/INSTALL.TGZ (checked by reading it back), the launcher current/inoink.sh, then the
// checksum INSTALL.SUM last, because the Kobo ignores a package without one. Every name is a plain
// 8.3 name; the Kobo unpacks the release itself when InoInk next starts
// (contrib/inoink/inoink-dispatch.sh). Remove mirrors scripts/remove-inoink.sh.
//
// The device table and the idea of a browser installer come from Kobalt
// (https://github.com/nicoverbruggen/kobalt, MIT, Copyright Nico Verbruggen).

export const REPO = "justmytwospence/InoInk";

// --- devices -------------------------------------------------------------------------------------

// The last three digits of the hardware id in .kobo/version. Names and groups match the README's
// Status section (tests/web-installer.test.mjs checks that they agree).
export const MODELS = [
  { ids: ["390"], name: "Kobo Libra Colour", status: "tested" },
  { ids: ["393"], name: "Kobo Clara Colour", status: "expected" },
  { ids: ["391", "395"], name: "Kobo Clara BW", status: "expected" },
  { ids: ["389"], name: "Kobo Elipsa 2E", status: "expected" },
  { ids: ["386"], name: "Kobo Clara 2E", status: "expected" },
  { ids: ["388"], name: "Kobo Libra 2", status: "expected" },
  { ids: ["383"], name: "Kobo Sage", status: "expected" },
  { ids: ["387"], name: "Kobo Elipsa", status: "expected" },
  { ids: ["382"], name: "Kobo Nia", status: "expected" },
  { ids: ["384"], name: "Kobo Libra H₂O", status: "expected" },
  { ids: ["377", "380"], name: "Kobo Forma", status: "expected" },
  { ids: ["376"], name: "Kobo Clara HD", status: "expected" },
  { ids: ["378"], name: "Kobo Aura H₂O Edition 2 (revision 2)", status: "expected" },
  { ids: ["379"], name: "Kobo Aura Edition 2 (revision 2)", status: "expected" },
  { ids: ["690"], name: "Tolino Vision Color", status: "expected" },
  { ids: ["693"], name: "Tolino Shine Color", status: "expected" },
  { ids: ["691", "695"], name: "Tolino Shine (5th generation)", status: "expected" },
  { ids: ["373", "381"], name: "Kobo Aura ONE", status: "experimental" },
  { ids: ["371"], name: "Kobo Glo HD", status: "experimental" },
  { ids: ["372"], name: "Kobo Touch 2.0", status: "experimental" },
  { ids: ["375"], name: "Kobo Aura Edition 2 (revision 1)", status: "experimental" },
  { ids: ["374"], name: "Kobo Aura H₂O Edition 2 (revision 1)", status: "experimental" },
  { ids: ["370"], name: "Kobo Aura H₂O", status: "experimental" },
  { ids: ["360"], name: "Kobo Aura", status: "experimental" },
  { ids: ["350"], name: "Kobo Aura HD", status: "experimental" },
  { ids: ["330"], name: "Kobo Glo", status: "experimental" },
  { ids: ["340"], name: "Kobo Mini", status: "experimental" },
  { ids: ["310", "320"], name: "Kobo Touch A/B/C", status: "experimental" },
];

export const MIN_FIRMWARE = "4.6";

/** Compares dotted numbers: -1, 0 or 1. Missing parts count as 0. */
export function compareDotted(a, b) {
  const x = String(a).split(".").map((n) => parseInt(n, 10) || 0);
  const y = String(b).split(".").map((n) => parseInt(n, 10) || 0);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] || 0) - (y[i] || 0);
    if (d) return d < 0 ? -1 : 1;
  }
  return 0;
}

/** "supported", "experimental" (5.x and later: no official NickelMenu), "too-old" or "unknown". */
export function firmwareSupport(firmware) {
  if (!/^\d+(\.\d+)+$/.test(firmware || "")) return "unknown";
  if (compareDotted(firmware, MIN_FIRMWARE) < 0) return "too-old";
  if (compareDotted(firmware, "5") >= 0) return "experimental";
  return "supported";
}

/**
 * Reads .kobo/version: serial, kernel, firmware, ..., hardware id. Throws when it is not one.
 * Returns { serial, firmware, hardwareId, model, status, firmwareStatus }; model and status are
 * null for a device that is not in MODELS.
 */
export function parseVersionFile(text) {
  const f = String(text).trim().split(",").map((s) => s.trim());
  if (f.length < 6 || !f[2] || !f[5]) {
    throw new Error("The Kobo's .kobo/version file does not look like one.");
  }
  const hardwareId = f[5];
  const id = hardwareId.slice(-3);
  const m = MODELS.find((model) => model.ids.includes(id)) || null;
  return {
    serial: f[0],
    firmware: f[2],
    hardwareId,
    model: m ? m.name : null,
    status: m ? m.status : null,
    firmwareStatus: firmwareSupport(f[2]),
  };
}

// --- checksums -------------------------------------------------------------------------------------

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i << 24;
    for (let k = 0; k < 8; k++) c = c & 0x80000000 ? (c << 1) ^ 0x04c11db7 : c << 1;
    t[i] = c >>> 0;
  }
  return t;
})();

/** What `cksum < file` prints (POSIX CRC and size), without the newline. */
export function cksum(bytes) {
  let crc = 0;
  for (let i = 0; i < bytes.length; i++) {
    crc = ((crc << 8) ^ CRC_TABLE[((crc >>> 24) ^ bytes[i]) & 0xff]) >>> 0;
  }
  for (let n = bytes.length; n > 0; n = Math.floor(n / 256)) {
    crc = ((crc << 8) ^ CRC_TABLE[((crc >>> 24) ^ n) & 0xff]) >>> 0;
  }
  return `${~crc >>> 0} ${bytes.length}`;
}

export async function sha256Hex(bytes) {
  const d = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return Array.from(d, (b) => b.toString(16).padStart(2, "0")).join("");
}

// --- the package -----------------------------------------------------------------------------------

const UNSAFE_PATH = /(^\/|(^|\/)\.\.(\/|$))/;
const VALID_NAME = /^[A-Za-z0-9_-][A-Za-z0-9._-]*$/;
const WANTED = ["inoink/VERSION", "inoink/dispatch/inoink.sh", "inoink/dispatch/inoink.nm"];
const REQUIRED = ["inoink/release/inoink", "inoink/release/inoink-run.sh"];

export class PackageError extends Error {}

class Bytes {
  constructor(stream) {
    this.reader = stream.getReader();
    this.chunks = [];
    this.have = 0;
    this.done = false;
  }
  async fill(n) {
    while (this.have < n && !this.done) {
      const { value, done } = await this.reader.read();
      if (done) this.done = true;
      else if (value.length) {
        this.chunks.push(value);
        this.have += value.length;
      }
    }
    return this.have >= n;
  }
  // n bytes, or null at a clean end of the stream.
  async read(n) {
    if (!(await this.fill(n))) {
      if (this.have === 0) return null;
      throw new PackageError("The package is cut short.");
    }
    const out = new Uint8Array(n);
    let at = 0;
    while (at < n) {
      const c = this.chunks[0];
      const take = Math.min(c.length, n - at);
      out.set(c.subarray(0, take), at);
      at += take;
      if (take === c.length) this.chunks.shift();
      else this.chunks[0] = c.subarray(take);
    }
    this.have -= n;
    return out;
  }
  async skip(n) {
    while (n > 0) {
      if (!(await this.fill(1))) throw new PackageError("The package is cut short.");
      const c = this.chunks[0];
      const take = Math.min(c.length, n);
      if (take === c.length) this.chunks.shift();
      else this.chunks[0] = c.subarray(take);
      this.have -= take;
      n -= take;
    }
  }
}

const text = (b) => new TextDecoder().decode(b);
const cstr = (b) => {
  const z = b.indexOf(0);
  return text(z < 0 ? b : b.subarray(0, z));
};

function octal(b) {
  if (b[0] & 0x80) {
    // GNU base-256 for large numbers.
    let n = b[0] & 0x7f;
    for (let i = 1; i < b.length; i++) n = n * 256 + b[i];
    return n;
  }
  const s = cstr(b).trim();
  if (s && !/^[0-7]+$/.test(s)) throw new PackageError("The package is not a valid archive.");
  return s ? parseInt(s, 8) : 0;
}

function headerOk(h) {
  let sum = 0;
  for (let i = 0; i < 512; i++) sum += i >= 148 && i < 156 ? 32 : h[i];
  return octal(h.subarray(148, 156)) === sum;
}

/**
 * Reads a Kobo package (.tar.gz) the way install-inoink.sh does: every entry must stay inside the
 * archive, VERSION must be a plain name, and the launcher files must be present.
 * Returns { version, files: { "dispatch/inoink.sh": Uint8Array, "dispatch/inoink.nm": ... } }.
 */
export async function readPackage(bytes) {
  let stream;
  try {
    stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip"));
  } catch (e) {
    throw new PackageError(`The package could not be opened: ${e.message}`);
  }
  const r = new Bytes(stream);
  const found = {};
  const seen = new Set();
  let longName = null;
  let paxPath = null;
  let entries = 0;
  try {
    for (;;) {
      const h = await r.read(512);
      if (h === null || h.every((b) => b === 0)) break;
      if (!headerOk(h)) throw new PackageError("The package is not a valid archive.");
      const size = octal(h.subarray(124, 136));
      const type = String.fromCharCode(h[156] || 48);
      const padded = Math.ceil(size / 512) * 512;
      if (type === "L" || type === "x") {
        const data = await r.read(padded);
        if (data === null) throw new PackageError("The package is cut short.");
        if (type === "L") longName = cstr(data.subarray(0, size));
        else paxPath = paxField(data.subarray(0, size), "path") ?? paxPath;
        continue;
      }
      let name = cstr(h.subarray(0, 100));
      if (cstr(h.subarray(257, 263)) === "ustar" && h[262] === 0) {
        const prefix = cstr(h.subarray(345, 500));
        if (prefix) name = `${prefix}/${name}`;
      }
      name = paxPath ?? longName ?? name;
      paxPath = longName = null;
      if (type === "g" || type === "K") {
        await r.skip(padded);
        continue;
      }
      entries++;
      if (UNSAFE_PATH.test(name)) throw new PackageError("The package contains unsafe paths.");
      const clean = name.replace(/^(\.\/)+/, "").replace(/\/+$/, "");
      seen.add(clean);
      if (WANTED.includes(clean) && (type === "0" || type === "\0")) {
        const data = await r.read(padded);
        if (data === null) throw new PackageError("The package is cut short.");
        found[clean] = data.slice(0, size);
      } else {
        await r.skip(padded);
      }
    }
  } catch (e) {
    if (e instanceof PackageError) throw e;
    throw new PackageError(`The file is damaged or not a .tar.gz package (${e.message}). Download it again.`);
  }
  if (!entries) throw new PackageError("The package is empty.");
  if (!found["inoink/VERSION"]) throw new PackageError("This is not an InoInk package (no VERSION).");
  const version = text(found["inoink/VERSION"]).replace(/\n+$/, "");
  if (!VALID_NAME.test(version)) throw new PackageError("Bad version in the package.");
  for (const f of ["inoink/dispatch/inoink.sh", ...REQUIRED]) {
    if (!seen.has(f)) throw new PackageError(`The package is incomplete (${f} is missing).`);
  }
  const files = {};
  for (const [k, v] of Object.entries(found)) files[k.replace(/^inoink\//, "")] = v;
  return { version, files };
}

function paxField(data, key) {
  const s = text(data);
  let at = 0;
  let value = null;
  while (at < s.length) {
    const sp = s.indexOf(" ", at);
    if (sp < 0) break;
    const len = parseInt(s.slice(at, sp), 10);
    if (!(len > 0)) break;
    const rec = s.slice(sp + 1, at + len - 1);
    const eq = rec.indexOf("=");
    if (rec.slice(0, eq) === key) value = rec.slice(eq + 1);
    at += len;
  }
  return value;
}

// --- releases on GitHub ----------------------------------------------------------------------------

export const packageName = (version) => `inoink-${version}-kobo.tar.gz`;

function releaseInfo(json) {
  const version = String(json.tag_name || "").replace(/^v/, "");
  const asset = (json.assets || []).find((a) => a.name === packageName(version));
  if (!asset) throw new Error(`Release ${json.tag_name} has no Kobo package.`);
  return {
    version,
    url: json.html_url,
    asset: {
      name: asset.name,
      size: asset.size,
      sha256: /^sha256:([0-9a-f]{64})$/.exec(asset.digest || "")?.[1] || null,
      apiUrl: asset.url,
      downloadUrl: asset.browser_download_url,
    },
  };
}

async function getJson(url, fetchFn) {
  const res = await fetchFn(url, { headers: { Accept: "application/vnd.github+json" } });
  if (!res.ok) throw new Error(`GitHub answered ${res.status} for ${url}.`);
  return res.json();
}

/** The latest release and its Kobo package (name, size, SHA-256, URLs). */
export async function latestRelease(fetchFn = fetch) {
  return releaseInfo(await getJson(`https://api.github.com/repos/${REPO}/releases/latest`, fetchFn));
}

/** The release named by a package's VERSION. */
export async function releaseFor(version, fetchFn = fetch) {
  const tag = encodeURIComponent(`v${version}`);
  return releaseInfo(await getJson(`https://api.github.com/repos/${REPO}/releases/tags/${tag}`, fetchFn));
}

/**
 * The package published with this site (release.json, written by scripts/pages-site.sh), which
 * the browser can download because it comes from the same site. Null when there is none.
 */
export async function siteRelease(fetchFn = fetch, base = new URL(".", import.meta.url)) {
  try {
    const res = await fetchFn(new URL("release.json", base), { cache: "no-store" });
    if (!res.ok) return null;
    const j = await res.json();
    if (
      !VALID_NAME.test(String(j.version)) ||
      j.name !== packageName(j.version) ||
      !/^[0-9a-f]{64}$/.test(String(j.sha256)) ||
      !Number.isInteger(j.size) ||
      !String(j.file).startsWith("downloads/") ||
      String(j.file).includes("..")
    ) {
      return null;
    }
    return {
      version: j.version,
      url: `https://github.com/${REPO}/releases/tag/v${j.version}`,
      asset: {
        name: j.name,
        size: j.size,
        sha256: j.sha256,
        siteUrl: new URL(j.file, base).href,
        apiUrl: null,
        downloadUrl: `https://github.com/${REPO}/releases/download/v${j.version}/${j.name}`,
      },
    };
  } catch {
    return null;
  }
}

export class DownloadBlocked extends Error {}

/**
 * Downloads a release package in the browser: from this site when it hosts the package
 * (asset.siteUrl), else from GitHub, whose asset storage sends no CORS headers today, so that
 * usually throws DownloadBlocked and the page asks for the downloaded file instead.
 */
export async function downloadPackage(asset, { onProgress, fetchFn = fetch } = {}) {
  let res;
  try {
    res = asset.siteUrl
      ? await fetchFn(asset.siteUrl)
      : await fetchFn(asset.apiUrl, { headers: { Accept: "application/octet-stream" } });
  } catch (e) {
    throw new DownloadBlocked(e.message);
  }
  if (!res.ok || !res.body) throw new DownloadBlocked(`GitHub answered ${res.status}.`);
  const reader = res.body.getReader();
  const chunks = [];
  let got = 0;
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    chunks.push(value);
    got += value.length;
    onProgress?.(asset.size ? got / asset.size : null);
  }
  const out = new Uint8Array(got);
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.length;
  }
  return out;
}

/** Throws unless the bytes are the release asset (SHA-256 from GitHub). */
export async function verifyAsset(bytes, asset) {
  if (!asset.sha256) {
    throw new Error("GitHub publishes no checksum for this package, so it cannot be checked here. Install it with install-inoink.sh instead.");
  }
  if (bytes.length !== asset.size || (await sha256Hex(bytes)) !== asset.sha256) {
    throw new Error(`This file is not the published ${asset.name}: its checksum does not match. Download it again.`);
  }
}

// --- the Kobo --------------------------------------------------------------------------------------

const notFound = (e) => e?.name === "NotFoundError" || e?.name === "TypeMismatchError";

async function dir(root, path, create = false) {
  let d = root;
  for (const p of path) d = await d.getDirectoryHandle(p, { create });
  return d;
}

async function tryDir(root, path) {
  try {
    return await dir(root, path);
  } catch (e) {
    if (notFound(e)) return null;
    throw e;
  }
}

async function readFile(d, name) {
  try {
    const f = await (await d.getFileHandle(name)).getFile();
    return new Uint8Array(await f.arrayBuffer());
  } catch (e) {
    if (notFound(e)) return null;
    throw e;
  }
}

async function writeFile(d, name, bytes, onProgress) {
  const w = await (await d.getFileHandle(name, { create: true })).createWritable({ keepExistingData: false });
  try {
    const step = 4 << 20;
    for (let at = 0; at < bytes.length; at += step) {
      await w.write(bytes.subarray(at, at + step));
      onProgress?.(Math.min(1, (at + step) / bytes.length));
    }
    if (!bytes.length) await w.write(bytes);
  } catch (e) {
    await w.abort?.().catch(() => {});
    throw e;
  }
  await w.close();
}

async function remove(d, name) {
  if (!d) return false;
  try {
    await d.removeEntry(name, { recursive: true });
    return true;
  } catch (e) {
    if (notFound(e)) return false;
    throw e;
  }
}

async function exists(d, name) {
  if (!d) return false;
  for (const kind of ["getFileHandle", "getDirectoryHandle"]) {
    try {
      await d[kind](name);
      return true;
    } catch (e) {
      if (!notFound(e)) throw e;
    }
  }
  return false;
}

/** Reads .kobo/version from the drive's root; throws a readable error for any other folder. */
export async function readDevice(root) {
  const kobo = await tryDir(root, [".kobo"]);
  const v = kobo && (await readFile(kobo, "version"));
  if (!v) {
    throw new Error("This is not the root of a Kobo (no .kobo/version). Choose the drive named KOBOeReader itself, not a folder on it.");
  }
  return parseVersionFile(text(v));
}

const firstLine = (b) => (b ? text(b).replace(/[\r\n ]/g, "") : "");

/** What InoInk has on the Kobo now. */
export async function inspect(root) {
  const adds = await tryDir(root, [".adds"]);
  const nm = adds && (await tryDir(adds, ["nm"]));
  const base = adds && (await tryDir(adds, ["inoink"]));
  const data = base && (await tryDir(base, ["data"]));
  const active = base ? firstLine(await readFile(base, "active-release")) : "";
  const previous = base ? firstLine(await readFile(base, "previous-release")) : "";
  return {
    present: !!base,
    installed: VALID_NAME.test(active) ? active : null,
    previous: VALID_NAME.test(previous) ? previous : null,
    launcher: !!base && (await exists(await tryDir(base, ["current"]), "inoink.sh")),
    pending: !!base && (await exists(base, "INSTALL.TGZ")),
    signedIn: await exists(data, "credentials.json"),
    accountsPending: await exists(data && (await tryDir(data, ["provision"])), "accounts.json"),
    hasData: await exists(data, "inoink.db"),
    nickelMenu: !!nm,
    menuEntry: await exists(nm, "inoink"),
  };
}

/**
 * Installs or updates: the same files, in the same order, as install-inoink.sh. `pkg` is the
 * result of readPackage(bytes). Data in .adds/inoink/data is never touched.
 * Returns { version, menu: "written" | "no-nickelmenu" | "skipped" }.
 */
export async function install(root, bytes, pkg, { menu = true, onProgress } = {}) {
  const sum = cksum(bytes);
  const adds = await dir(root, [".adds"], true);
  const base = await dir(adds, ["inoink"], true);
  const current = await dir(base, ["current"], true);
  await dir(base, ["data"], true);
  await remove(base, "INSTALL.SUM");
  await writeFile(base, "INSTALL.TGZ", bytes, (f) => onProgress?.("copy", f));
  onProgress?.("check", null);
  const copy = await readFile(base, "INSTALL.TGZ");
  if (!copy || cksum(copy) !== sum) {
    await remove(base, "INSTALL.TGZ");
    throw new Error("The copy to the Kobo is damaged; nothing was changed. Try again, or try another USB port or cable.");
  }
  await writeFile(current, "inoink.sh", pkg.files["dispatch/inoink.sh"]);
  // The checksum goes last: the Kobo ignores a package without one.
  await writeFile(base, "INSTALL.SUM", new TextEncoder().encode(`${sum}\n`));
  let entry = "skipped";
  // An entry from an earlier install is replaced too, so it gets the current start-up checks.
  const nm = await tryDir(adds, ["nm"]);
  if (menu || (await exists(nm, "inoink"))) {
    if (nm && pkg.files["dispatch/inoink.nm"]) {
      await writeFile(nm, "inoink", pkg.files["dispatch/inoink.nm"]);
      entry = "written";
    } else entry = "no-nickelmenu";
  }
  return { version: pkg.version, menu: entry };
}

/**
 * Saves the Karakeep address and key for the Kobo to import when InoInk starts
 * (.adds/inoink/data/provision/accounts.json), keeping any other waiting entry.
 */
export async function saveKarakeep(root, url, apiKey) {
  url = String(url || "").trim().replace(/\/+$/, "");
  if (url && !url.includes("://")) url = `https://${url}`;
  if (!/^https?:\/\/[^\s/]+\S*$/.test(url)) throw new Error("The Karakeep address must be a web address, such as https://karakeep.example.com.");
  apiKey = String(apiKey || "").trim();
  if (!apiKey) throw new Error("Enter the Karakeep API key.");
  const data = await dir(root, [".adds", "inoink", "data"], true);
  const provision = await dir(data, ["provision"], true);
  let obj = {};
  try {
    const old = await readFile(provision, "accounts.json");
    if (old) obj = JSON.parse(text(old)) || {};
  } catch {
    obj = {};
  }
  obj.karakeep = { url, api_key: apiKey };
  await writeFile(provision, "accounts.json", new TextEncoder().encode(`${JSON.stringify(obj, null, 2)}\n`));
}

export const REMOVE_LEVELS = ["keep", "settings", "all"];

/**
 * Removes InoInk like remove-inoink.sh. keep: the app goes, .adds/inoink/data stays (sign-in,
 * articles, highlights, settings). settings: also Settings.toml and config.sh. all: the whole
 * .adds/inoink folder. Returns false when InoInk was not installed.
 */
export async function removeInoInk(root, level = "keep") {
  if (!REMOVE_LEVELS.includes(level)) throw new Error(`Unknown removal level ${level}.`);
  const adds = await tryDir(root, [".adds"]);
  const base = adds && (await tryDir(adds, ["inoink"]));
  await remove(adds && (await tryDir(adds, ["nm"])), "inoink");
  if (!base) return false;
  for (const name of ["releases", "current", "active-release", "previous-release", "INSTALL.TGZ", "INSTALL.SUM"]) {
    await remove(base, name);
  }
  if (level === "settings") {
    const data = await tryDir(base, ["data"]);
    await remove(data, "Settings.toml");
    await remove(data, "config.sh");
  }
  if (level === "all") await remove(adds, "inoink");
  return true;
}
