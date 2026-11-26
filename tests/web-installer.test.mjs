// Tests for the project page's browser installer (site/kobo.js), under Node with a stand-in for
// the browser's File System Access API on a real directory. The installer must leave the Kobo
// exactly as scripts/install-inoink.sh and scripts/remove-inoink.sh do, and the Kobo's own
// launcher (contrib/inoink/inoink-dispatch.sh) must accept what it wrote.
//   node --test tests/web-installer.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";
import * as kobo from "../site/kobo.js";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), "inoink-web-"));

// --- a File System Access stand-in ---------------------------------------------------------------
// Like Chrome: getFileHandle({create}) creates an empty file, createWritable() writes to
// <name>.crswap and moves it into place on close().

const domError = (name, msg) => new DOMException(msg, name);

class FileHandle {
  kind = "file";
  constructor(p, opts) {
    this.path = p;
    this.name = path.basename(p);
    this.opts = opts;
  }
  async getFile() {
    return new Blob([fs.readFileSync(this.path)]);
  }
  async createWritable() {
    const swap = `${this.path}.crswap`;
    fs.writeFileSync(swap, "");
    const opts = this.opts;
    const target = this.path;
    return {
      async write(chunk) {
        const b = Buffer.from(chunk);
        if (opts.corrupt === path.basename(target) && b.length) b[0] ^= 0xff;
        fs.appendFileSync(swap, b);
      },
      async close() {
        fs.renameSync(swap, target);
      },
      async abort() {
        fs.rmSync(swap, { force: true });
      },
    };
  }
}

class DirHandle {
  kind = "directory";
  constructor(p, opts = {}) {
    this.path = p;
    this.name = path.basename(p);
    this.opts = opts;
  }
  child(name) {
    if (!name || name === "." || name === ".." || /[/\\]/.test(name)) throw new TypeError(`bad name ${name}`);
    return path.join(this.path, name);
  }
  async getDirectoryHandle(name, { create = false } = {}) {
    const p = this.child(name);
    if (!fs.existsSync(p)) {
      if (!create) throw domError("NotFoundError", `${name} not found`);
      fs.mkdirSync(p);
    } else if (!fs.statSync(p).isDirectory()) throw domError("TypeMismatchError", `${name} is a file`);
    return new DirHandle(p, this.opts);
  }
  async getFileHandle(name, { create = false } = {}) {
    const p = this.child(name);
    if (!fs.existsSync(p)) {
      if (!create) throw domError("NotFoundError", `${name} not found`);
      fs.writeFileSync(p, "");
    } else if (fs.statSync(p).isDirectory()) throw domError("TypeMismatchError", `${name} is a folder`);
    return new FileHandle(p, this.opts);
  }
  async removeEntry(name, { recursive = false } = {}) {
    const p = this.child(name);
    if (!fs.existsSync(p)) throw domError("NotFoundError", `${name} not found`);
    if (fs.statSync(p).isDirectory() && fs.readdirSync(p).length && !recursive) {
      throw domError("InvalidModificationError", `${name} is not empty`);
    }
    fs.rmSync(p, { recursive: true });
  }
}

// --- fixtures ------------------------------------------------------------------------------------

const VERSION_LINE = "N428000000000,4.9.77,4.46.23852,4.9.77,4.9.77,00000000-0000-0000-0000-000000000390";

function makeKobo({ nickelMenu = true } = {}) {
  const k = tmp();
  fs.mkdirSync(path.join(k, ".kobo"));
  fs.writeFileSync(path.join(k, ".kobo/version"), `${VERSION_LINE}\n`);
  fs.mkdirSync(path.join(k, ".adds/other"), { recursive: true });
  fs.writeFileSync(path.join(k, ".adds/other/file"), "keep\n");
  if (nickelMenu) fs.mkdirSync(path.join(k, ".adds/nm"));
  return k;
}

// A package like tests/package.sh builds, made with the system tar.
function makePackage(version, { format } = {}) {
  const stage = tmp();
  const r = path.join(stage, "inoink/release");
  fs.mkdirSync(path.join(r, "scripts"), { recursive: true });
  fs.mkdirSync(path.join(stage, "inoink/dispatch"));
  fs.writeFileSync(path.join(stage, "inoink/VERSION"), `${version}\n`);
  fs.writeFileSync(path.join(r, "inoink"), "#!/bin/sh\nexit 0\n", { mode: 0o755 });
  fs.copyFileSync(path.join(ROOT, "contrib/inoink/inoink-run.sh"), path.join(r, "inoink-run.sh"));
  fs.copyFileSync(path.join(ROOT, "contrib/inoink/inoink-dispatch.sh"), path.join(stage, "inoink/dispatch/inoink.sh"));
  fs.copyFileSync(path.join(ROOT, "contrib/inoink/inoink.nm"), path.join(stage, "inoink/dispatch/inoink.nm"));
  // A name longer than 100 bytes needs the format's long-name mechanism.
  const deep = path.join(r, "scripts", "a-rather-long-directory-name-for-testing", "and-another-long-directory-name-inside");
  fs.mkdirSync(deep, { recursive: true });
  fs.writeFileSync(path.join(deep, "with-a-long-file-name-at-the-end.txt"), "long\n");
  const out = path.join(tmp(), `inoink-${version}-kobo.tar.gz`);
  const args = ["-czf", out, "-C", stage];
  if (format) args.unshift(`--format=${format}`);
  execFileSync("tar", [...args, "./inoink"]);
  return out;
}

// A hand-made archive, for entries tar refuses to create.
function rawTar(entries) {
  const blocks = [];
  for (const { name, data = "", type = "0" } of entries) {
    const body = Buffer.from(data);
    const h = Buffer.alloc(512);
    h.write(name, 0, 100);
    h.write("0000644\0", 100);
    h.write("0000000\0", 108);
    h.write("0000000\0", 116);
    h.write(`${body.length.toString(8).padStart(11, "0")}\0`, 124);
    h.write("00000000000\0", 136);
    h.write("        ", 148);
    h.write(type, 156);
    h.write("ustar\0", 257);
    h.write("00", 263);
    let sum = 0;
    for (const b of h) sum += b;
    h.write(`${sum.toString(8).padStart(6, "0")}\0 `, 148);
    blocks.push(h, body, Buffer.alloc((512 - (body.length % 512)) % 512));
  }
  blocks.push(Buffer.alloc(1024));
  return zlib.gzipSync(Buffer.concat(blocks));
}

const complete = (version) => [
  { name: "inoink/VERSION", data: `${version}\n` },
  { name: "inoink/release/inoink", data: "x" },
  { name: "inoink/release/inoink-run.sh", data: "x" },
  { name: "inoink/dispatch/inoink.sh", data: "x" },
];

function tree(dir) {
  const out = {};
  const walk = (d, rel) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const p = path.join(d, e.name);
      const r = rel ? `${rel}/${e.name}` : e.name;
      if (r === ".adds/inoink/data/inoink.log") continue; // timestamps differ
      if (e.isDirectory()) {
        out[`${r}/`] = "";
        walk(p, r);
      } else out[r] = fs.readFileSync(p).toString("base64");
    }
  };
  walk(dir, "");
  return out;
}

const read = (k, p) => fs.readFileSync(path.join(k, p), "utf8");
const has = (k, p) => fs.existsSync(path.join(k, p));

function scriptInstall(pkg, k, menu = true) {
  execFileSync("sh", [path.join(ROOT, "scripts/install-inoink.sh"), pkg, k, ...(menu ? ["--menu"] : [])], {
    env: { ...process.env, INOINK_NO_EJECT: "1" },
    stdio: "pipe",
  });
}

async function webInstall(pkgPath, k, opts = {}) {
  const bytes = new Uint8Array(fs.readFileSync(pkgPath));
  const pkg = await kobo.readPackage(bytes);
  return kobo.install(new DirHandle(k, opts), bytes, pkg, opts);
}

// Runs the Kobo's launcher in its install-only mode, as the Kobo does when InoInk starts.
function kobosideUpdate(k) {
  const r = spawnSync("sh", [path.join(k, ".adds/inoink/current/inoink.sh")], {
    env: { ...process.env, INOINK_UPDATE_ONLY: "1" },
    stdio: "pipe",
  });
  assert.equal(r.status, 0, r.stderr.toString());
}

// --- tests ---------------------------------------------------------------------------------------

test("cksum matches the system's cksum", () => {
  const samples = [Buffer.alloc(0), Buffer.from("a"), Buffer.from("InoInk\n"), Buffer.alloc(70000, 7), fs.readFileSync(makePackage("1.0.0"))];
  for (const s of samples) {
    const want = execFileSync("cksum", { input: s }).toString().trim();
    assert.equal(kobo.cksum(new Uint8Array(s)), want);
  }
});

test(".kobo/version: model, firmware and their support", () => {
  const d = kobo.parseVersionFile(VERSION_LINE);
  assert.equal(d.model, "Kobo Libra Colour");
  assert.equal(d.status, "tested");
  assert.equal(d.firmware, "4.46.23852");
  assert.equal(d.firmwareStatus, "supported");
  assert.equal(kobo.parseVersionFile("SN,x,4.46.23852,x,x,00000000390").model, "Kobo Libra Colour");
  assert.equal(kobo.parseVersionFile("SN,x,4.38.1,x,x,00000000-0000-0000-0000-000000000690").status, "expected");
  assert.equal(kobo.parseVersionFile("SN,x,4.38.1,x,x,00000000-0000-0000-0000-000000000310").status, "experimental");
  const unknown = kobo.parseVersionFile("SN,x,4.38.1,x,x,00000000-0000-0000-0000-000000000999");
  assert.equal(unknown.model, null);
  assert.equal(kobo.firmwareSupport("4.6"), "supported");
  assert.equal(kobo.firmwareSupport("4.5.9999"), "too-old");
  assert.equal(kobo.firmwareSupport("3.19.5761"), "too-old");
  assert.equal(kobo.firmwareSupport("5.2.1"), "experimental");
  assert.equal(kobo.firmwareSupport("6.0.2"), "experimental");
  assert.equal(kobo.firmwareSupport("abc"), "unknown");
  assert.throws(() => kobo.parseVersionFile("not a version file"));
  const ids = kobo.MODELS.flatMap((m) => m.ids);
  assert.equal(new Set(ids).size, ids.length, "hardware ids are unique");
});

test("the device list agrees with the README's Status section", () => {
  const readme = fs.readFileSync(path.join(ROOT, "README.md"), "utf8");
  const status = readme.slice(readme.indexOf("## Status"), readme.indexOf("## What you need"));
  const line = (word) => {
    const at = status.indexOf(`**${word}`);
    return status.slice(at, status.indexOf("\n- **", at + 1)).replace(/\s+/g, " ");
  };
  const lines = { tested: line("Tested"), expected: line("Expected"), experimental: line("Experimental") };
  for (const m of kobo.MODELS) {
    const short = m.name.replace(/^(Kobo|Tolino) /, "");
    assert.ok(lines[m.status].includes(short), `README lists ${m.name} as ${m.status}`);
  }
});

test("reads packages in every tar format", async () => {
  for (const format of [undefined, "ustar", "pax", "gnutar", "gnu"]) {
    let pkg;
    try {
      pkg = makePackage("1.2.3", { format });
    } catch {
      continue; // this tar does not know the format
    }
    const p = await kobo.readPackage(new Uint8Array(fs.readFileSync(pkg)));
    assert.equal(p.version, "1.2.3", `format ${format}`);
    assert.equal(Buffer.from(p.files["dispatch/inoink.sh"]).toString(), read(ROOT, "contrib/inoink/inoink-dispatch.sh"));
    assert.equal(Buffer.from(p.files["dispatch/inoink.nm"]).toString(), read(ROOT, "contrib/inoink/inoink.nm"));
  }
});

test("refuses unsafe, incomplete and foreign packages", async () => {
  const bad = async (entries, re) => assert.rejects(kobo.readPackage(new Uint8Array(rawTar(entries))), re);
  assert.equal((await kobo.readPackage(new Uint8Array(rawTar(complete("2.0.0"))))).version, "2.0.0");
  await bad([...complete("2.0.0"), { name: "../escape", data: "x" }], /unsafe/);
  await bad([...complete("2.0.0"), { name: "/etc/passwd", data: "x" }], /unsafe/);
  await bad([...complete("2.0.0"), { name: "inoink/../../x", data: "x" }], /unsafe/);
  await bad(complete("../../escape"), /Bad version/);
  await bad(complete(".hidden"), /Bad version/);
  await bad(complete(""), /Bad version/);
  await bad(complete("2.0.0").slice(0, 3), /incomplete/);
  await bad([{ name: "README", data: "x" }], /not an InoInk package/);
  await assert.rejects(kobo.readPackage(new Uint8Array(zlib.gzipSync(Buffer.from("hello")))), kobo.PackageError);
  await assert.rejects(kobo.readPackage(new TextEncoder().encode("not gzip")), kobo.PackageError);
  const cut = rawTar(complete("2.0.0"));
  await assert.rejects(kobo.readPackage(new Uint8Array(zlib.gzipSync(zlib.gunzipSync(cut).subarray(0, 700)))), kobo.PackageError);
});

test("installs exactly what install-inoink.sh installs, and the Kobo accepts it", async () => {
  const pkg = makePackage("1.0.0");
  const viaScript = makeKobo();
  const viaWeb = makeKobo();
  scriptInstall(pkg, viaScript);
  const r = await webInstall(pkg, viaWeb, { menu: true });
  assert.deepEqual(r, { version: "1.0.0", menu: "written" });
  assert.deepEqual(tree(viaWeb), tree(viaScript));
  assert.equal(read(viaWeb, ".adds/inoink/INSTALL.SUM"), `${execFileSync("cksum", { input: fs.readFileSync(pkg) }).toString().trim()}\n`);
  assert.ok(!fs.readdirSync(path.join(viaWeb, ".adds/inoink")).some((n) => n.endsWith(".crswap")));

  kobosideUpdate(viaWeb);
  assert.equal(read(viaWeb, ".adds/inoink/active-release").trim(), "1.0.0");
  assert.ok(!has(viaWeb, ".adds/inoink/INSTALL.TGZ") && !has(viaWeb, ".adds/inoink/INSTALL.SUM"));
  assert.equal(read(viaWeb, ".adds/other/file"), "keep\n");

  // An update keeps the data and the previous release.
  fs.writeFileSync(path.join(viaWeb, ".adds/inoink/data/inoink.db"), "my data");
  const u = await webInstall(makePackage("1.1.0"), viaWeb, { menu: false });
  assert.equal(u.menu, "written", "an existing menu entry is refreshed, as install-inoink.sh does");
  kobosideUpdate(viaWeb);
  assert.equal(read(viaWeb, ".adds/inoink/active-release").trim(), "1.1.0");
  assert.equal(read(viaWeb, ".adds/inoink/previous-release").trim(), "1.0.0");
  assert.equal(read(viaWeb, ".adds/inoink/data/inoink.db"), "my data");
});

test("without NickelMenu no launcher entry is written", async () => {
  const k = makeKobo({ nickelMenu: false });
  const r = await webInstall(makePackage("1.0.0"), k, { menu: true });
  assert.equal(r.menu, "no-nickelmenu");
  assert.ok(!has(k, ".adds/nm"));
});

test("a damaged copy is removed and nothing else changes", async () => {
  const k = makeKobo();
  await webInstall(makePackage("1.0.0"), k);
  kobosideUpdate(k);
  const before = read(k, ".adds/inoink/current/inoink.sh");
  await assert.rejects(webInstall(makePackage("1.1.0"), k, { corrupt: "INSTALL.TGZ" }), /damaged/);
  assert.ok(!has(k, ".adds/inoink/INSTALL.TGZ") && !has(k, ".adds/inoink/INSTALL.SUM"));
  assert.equal(read(k, ".adds/inoink/current/inoink.sh"), before);
  assert.equal(read(k, ".adds/inoink/active-release").trim(), "1.0.0");
});

test("inspect reports what is installed", async () => {
  const k = makeKobo();
  const root = new DirHandle(k);
  assert.equal((await kobo.readDevice(root)).model, "Kobo Libra Colour");
  await assert.rejects(kobo.readDevice(new DirHandle(path.join(k, ".adds"))), /not the root of a Kobo/);
  let s = await kobo.inspect(root);
  assert.equal(s.present, false);
  assert.equal(s.installed, null);
  assert.equal(s.nickelMenu, true);
  await webInstall(makePackage("1.0.0"), k);
  s = await kobo.inspect(root);
  assert.equal(s.pending, true);
  assert.equal(s.launcher, true);
  assert.equal(s.menuEntry, true);
  kobosideUpdate(k);
  fs.writeFileSync(path.join(k, ".adds/inoink/data/credentials.json"), "{}");
  s = await kobo.inspect(root);
  assert.equal(s.installed, "1.0.0");
  assert.equal(s.pending, false);
  assert.equal(s.signedIn, true);
});

test("removal matches remove-inoink.sh at every level", async () => {
  for (const [level, flag] of [["keep", []], ["settings", ["--reset-settings"]], ["all", ["--purge-data"]]]) {
    const pkg = makePackage("1.0.0");
    const kobos = [makeKobo(), makeKobo()];
    for (const k of kobos) {
      scriptInstall(pkg, k);
      kobosideUpdate(k);
      scriptInstall(pkg, k); // a package waiting to be installed
      for (const f of ["inoink.db", "Settings.toml", "config.sh", "credentials.json"]) {
        fs.writeFileSync(path.join(k, ".adds/inoink/data", f), f);
      }
    }
    execFileSync("sh", [path.join(ROOT, "scripts/remove-inoink.sh"), kobos[0], ...flag], { stdio: "pipe" });
    assert.equal(await kobo.removeInoInk(new DirHandle(kobos[1]), level), true);
    assert.deepEqual(tree(kobos[1]), tree(kobos[0]), level);
    const t = tree(kobos[1]);
    assert.ok(!t[".adds/nm/inoink"], level);
    assert.equal(t[".adds/other/file"], Buffer.from("keep\n").toString("base64"));
    assert.equal(!!t[".adds/inoink/data/inoink.db"], level !== "all", level);
    assert.equal(!!t[".adds/inoink/data/Settings.toml"], level === "keep", level);
    assert.equal(!!t[".adds/inoink/data/credentials.json"], level !== "all", level);
    assert.ok(!t[".adds/inoink/INSTALL.TGZ"] && !t[".adds/inoink/current/"], level);
  }
  assert.equal(await kobo.removeInoInk(new DirHandle(makeKobo()), "all"), false);
  await assert.rejects(kobo.removeInoInk(new DirHandle(makeKobo()), "everything"));
});

test("release lookup and package verification", async () => {
  const bytes = new Uint8Array(fs.readFileSync(makePackage("1.0.0")));
  const sha = await kobo.sha256Hex(bytes);
  assert.equal(sha, execFileSync("shasum", ["-a", "256"], { input: bytes }).toString().split(" ")[0]);
  const release = {
    tag_name: "v1.0.0",
    html_url: "https://github.com/justmytwospence/InoInk/releases/tag/v1.0.0",
    assets: [
      { name: "inoink-1.0.0-source.tar.gz", size: 1, digest: "sha256:00", url: "a", browser_download_url: "b" },
      { name: "inoink-1.0.0-kobo.tar.gz", size: bytes.length, digest: `sha256:${sha}`, url: "https://api.github.com/x/1", browser_download_url: "https://github.com/x" },
    ],
  };
  const urls = [];
  const fetchFn = async (url) => {
    urls.push(url);
    return { ok: true, json: async () => release };
  };
  const latest = await kobo.latestRelease(fetchFn);
  assert.equal(latest.version, "1.0.0");
  assert.equal(latest.asset.sha256, sha);
  assert.equal(latest.asset.apiUrl, "https://api.github.com/x/1");
  assert.equal((await kobo.releaseFor("1.0.0", fetchFn)).version, "1.0.0");
  assert.deepEqual(urls, [
    "https://api.github.com/repos/justmytwospence/InoInk/releases/latest",
    "https://api.github.com/repos/justmytwospence/InoInk/releases/tags/v1.0.0",
  ]);
  await kobo.verifyAsset(bytes, latest.asset);
  const other = bytes.slice();
  other[100] ^= 1;
  await assert.rejects(kobo.verifyAsset(other, latest.asset), /does not match/);
  await assert.rejects(kobo.verifyAsset(bytes, { ...latest.asset, sha256: null }), /no checksum/);
  await assert.rejects(kobo.latestRelease(async () => ({ ok: true, json: async () => ({ ...release, assets: [] }) })), /no Kobo package/);
  await assert.rejects(kobo.latestRelease(async () => ({ ok: false, status: 403 })), /403/);

  // A blocked download (CORS) is reported as such, so the page can ask for the file.
  await assert.rejects(kobo.downloadPackage(latest.asset, { fetchFn: async () => { throw new TypeError("Failed to fetch"); } }), kobo.DownloadBlocked);
  const body = new Blob([bytes]).stream();
  const got = await kobo.downloadPackage(latest.asset, { fetchFn: async () => ({ ok: true, body }) });
  assert.deepEqual(got, bytes);
});

test("the package published with the site is downloaded from the site", async () => {
  const sha = "a".repeat(64);
  const good = { version: "1.2.3", name: "inoink-1.2.3-kobo.tar.gz", file: `downloads/inoink-1.2.3-kobo-${sha}.tar.gz`, size: 5, sha256: sha };
  const base = new URL("https://example.test/InoInk/");
  const fetchWith = (j, ok = true) => async () => ({ ok, json: async () => j });
  const r = await kobo.siteRelease(fetchWith(good), base);
  assert.equal(r.version, "1.2.3");
  assert.equal(r.asset.siteUrl, `https://example.test/InoInk/${good.file}`);
  assert.equal(r.asset.sha256, sha);
  for (const bad of [
    { ...good, sha256: "zz" },
    { ...good, name: "inoink-9.9.9-kobo.tar.gz" },
    { ...good, file: "elsewhere/x.tar.gz" },
    { ...good, file: "downloads/../x.tar.gz" },
    { ...good, size: "5" },
  ]) {
    assert.equal(await kobo.siteRelease(fetchWith(bad), base), null);
  }
  assert.equal(await kobo.siteRelease(fetchWith(good, false), base), null);
  assert.equal(await kobo.siteRelease(async () => { throw new TypeError("offline"); }, base), null);

  const urls = [];
  const bytes = new Uint8Array([1, 2, 3, 4, 5]);
  const got = await kobo.downloadPackage(r.asset, {
    fetchFn: async (...args) => {
      urls.push(args);
      return { ok: true, body: new Blob([bytes]).stream() };
    },
  });
  assert.deepEqual(got, bytes);
  assert.deepEqual(urls, [[r.asset.siteUrl]], "no GitHub headers for the site's own file");
});

test("Karakeep settings are saved for the Kobo to import", async () => {
  const k = makeKobo();
  const root = new DirHandle(k);
  const fixture = JSON.parse(fs.readFileSync(path.join(ROOT, "tests/fixtures/accounts.json"), "utf8"));
  const file = path.join(k, ".adds/inoink/data/provision/accounts.json");
  assert.equal((await kobo.inspect(root)).accountsPending, false);
  await kobo.saveKarakeep(root, ` ${fixture.karakeep.url}/ `, ` ${fixture.karakeep.api_key} `);
  assert.deepEqual(JSON.parse(fs.readFileSync(file, "utf8")).karakeep, fixture.karakeep);
  assert.equal((await kobo.inspect(root)).accountsPending, true);
  fs.writeFileSync(file, JSON.stringify({ inoreader: fixture.inoreader }));
  await kobo.saveKarakeep(root, "http://k.example", "key");
  const saved = JSON.parse(fs.readFileSync(file, "utf8"));
  assert.deepEqual(saved.inoreader, fixture.inoreader);
  assert.deepEqual(saved.karakeep, { url: "http://k.example", api_key: "key" });
  await assert.rejects(kobo.saveKarakeep(root, "ftp://x", "key"), /must start with https/);
  await assert.rejects(kobo.saveKarakeep(root, "https://k.example", "  "), /Enter the Karakeep API key/);
});
