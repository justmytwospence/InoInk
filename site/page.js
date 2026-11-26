// The project page: the device list and the "Install or remove on Kobo" panel. The work itself is
// in kobo.js.
import * as kobo from "./kobo.js";

const STATUS = {
  tested: { label: "Tested", blurb: "Checked on the hardware." },
  expected: { label: "Expected to work", blurb: "Same code paths as the tested model; nobody has tried yet." },
  experimental: {
    label: "Experimental",
    blurb: "Their Linux is older than the minimum the compiler targets, so InoInk may not start.",
  },
};

// --- device list -----------------------------------------------------------------------------------

function renderDevices() {
  const list = document.querySelector('[data-field="device-list"]');
  const button = document.querySelector('[data-action="all-devices"]');
  if (!list) return;
  const SHOWN = 9;
  let n = 0;
  for (const [status, info] of Object.entries(STATUS)) {
    const models = kobo.MODELS.filter((m) => m.status === status);
    const group = document.createElement("div");
    group.className = `device-group ${status}`;
    const h = document.createElement("h3");
    h.textContent = info.label;
    const p = document.createElement("p");
    p.textContent = info.blurb;
    const ul = document.createElement("ul");
    for (const m of models) {
      const li = document.createElement("li");
      li.textContent = m.name;
      if (n++ >= SHOWN) li.className = "extra";
      ul.append(li);
    }
    group.append(h, p, ul);
    if (models.every((_, i) => n - models.length + i >= SHOWN)) group.classList.add("extra");
    list.append(group);
  }
  if (n > SHOWN && button) {
    button.hidden = false;
    button.textContent = `Show all ${n} devices`;
    button.addEventListener("click", () => {
      list.classList.add("all");
      button.hidden = true;
    });
  }
}

// --- installer -------------------------------------------------------------------------------------

const box = document.getElementById("installer");
const field = (name) => box.querySelector(`[data-field="${name}"]`);
const show = (name, on) => box.querySelectorAll(`[data-show="${name}"]`).forEach((e) => (e.hidden = !on));
const action = (name) => box.querySelectorAll(`[data-action="${name}"]`);
const mb = (bytes) => `${Math.round(bytes / 1e6)} MB`;

const state = { root: null, device: null, status: null, release: null, releasePromise: null, busy: false, blocked: false };

function step(name) {
  for (const el of box.querySelectorAll("[data-step]")) el.hidden = el.dataset.step !== name;
}

export function supported(w = window) {
  return typeof w.showDirectoryPicker === "function" && !/Android|iPhone|iPad|Mobile/i.test(w.navigator.userAgent);
}

function release() {
  state.releasePromise ??= kobo.siteRelease().then((r) => r || kobo.latestRelease()).then(
    (r) => (state.release = r),
    (e) => {
      state.releasePromise = null; // try again next time
      throw e;
    },
  );
  return state.releasePromise;
}

async function pick() {
  show("pick-error", false);
  let root;
  try {
    root = await window.showDirectoryPicker({ id: "kobo", mode: "readwrite" });
  } catch (e) {
    if (e?.name !== "AbortError") pickError(e);
    return;
  }
  await connect(root);
}

function pickError(e) {
  const el = box.querySelector('[data-show="pick-error"]');
  el.textContent = e?.message || String(e);
  el.hidden = false;
  step("intro");
}

/** Shows what is on the Kobo whose root directory handle is given. */
export async function connect(root) {
  try {
    state.device = await kobo.readDevice(root);
    state.status = await kobo.inspect(root);
    state.root = root;
  } catch (e) {
    pickError(e);
    return;
  }
  release().then(renderDevice, renderDevice);
  renderDevice();
  step("device");
}

function renderDevice() {
  const { device: d, status: s, release: r } = state;
  if (!d) return;
  field("model").textContent = d.model || `Unknown model (${d.hardwareId})`;
  const ms = field("model-status");
  ms.textContent = d.status ? STATUS[d.status].label : "Not in the list";
  ms.dataset.status = d.status || "unknown";
  field("firmware").textContent = d.firmware;
  const fs = field("firmware-status");
  fs.textContent = { supported: "", experimental: "Experimental", "too-old": "Too old", unknown: "Unknown" }[d.firmwareStatus];
  fs.hidden = !fs.textContent;
  fs.dataset.status = d.firmwareStatus;

  const notes = [];
  if (d.firmwareStatus === "too-old") notes.push(`InoInk needs firmware ${kobo.MIN_FIRMWARE} or newer. Update the Kobo first.`);
  if (d.firmwareStatus === "experimental") {
    notes.push("Firmware 5 and later is experimental for InoInk, and official NickelMenu does not run on it.");
  }
  if (d.status === "experimental") notes.push("InoInk may not start on this model: its Linux is older than the minimum the compiler targets.");
  if (d.status === "expected") notes.push("Nobody has tried InoInk on this model yet.");
  if (!d.status) notes.push("This model is not in InoInk's list. It may work if it runs Kobo firmware, but nobody has tried.");
  if (d.status !== "tested") notes.push("A device report helps everyone with the same model.");
  const note = field("device-note");
  note.textContent = notes.join(" ");
  note.hidden = d.status === "tested" && d.firmwareStatus === "supported";

  let installed = "Not installed";
  if (s.installed) installed = `InoInk ${s.installed} is installed`;
  else if (s.present && s.hasData) installed = "Not installed; data from an earlier installation is on the Kobo";
  if (s.pending) installed += s.installed ? ". A package copied earlier installs when InoInk next starts" : ". A package copied earlier installs when InoInk first starts";
  field("installed").textContent = `${installed}.`;

  field("latest").textContent = r ? `InoInk ${r.version}` : state.releasePromise ? "Checking GitHub..." : "GitHub could not be reached; you can still choose a downloaded package.";
  field("nickelmenu").textContent = s.nickelMenu ? (s.menuEntry ? "Installed, with InoInk's entry" : "Installed") : "Not installed";
  show("no-nickelmenu", !s.nickelMenu);
  show("menu-option", s.nickelMenu);

  const install = action("install")[0];
  install.disabled = d.firmwareStatus === "too-old" || state.busy;
  let label = "Install InoInk";
  if (r) {
    const c = s.installed ? kobo.compareDotted(s.installed.split("-")[0], r.version) : null;
    if (c === null) label = `Install InoInk ${r.version}`;
    else if (c < 0) label = `Update to ${r.version}`;
    else if (s.installed === r.version) label = `Reinstall ${r.version}`;
    else label = `Install ${r.version}`;
  }
  install.textContent = label;
  action("remove")[0].hidden = !(s.present || s.menuEntry);
}

function working(title, detail = "Keep the Kobo connected.", fraction = null) {
  field("working-title").textContent = title;
  field("working-detail").textContent = detail;
  const p = field("progress");
  if (fraction === null) p.removeAttribute("value");
  else p.value = fraction;
  step("working");
}

function fail(e) {
  console.error(e);
  let msg = e?.message || String(e);
  if (e?.name === "NotAllowedError") msg = "The page was not allowed to change the Kobo. Choose it again and allow editing.";
  if (e?.name === "NotFoundError") msg = "The Kobo is no longer connected. Connect it, tap Connect, and choose it again.";
  field("error-message").textContent = msg;
  step("error");
}

async function busy(fn) {
  if (state.busy) return;
  state.busy = true;
  try {
    await fn();
  } catch (e) {
    fail(e);
  } finally {
    state.busy = false;
  }
}

function startInstall() {
  return busy(async () => {
    let r = null;
    try {
      r = await release();
    } catch {
      // Offline or rate-limited: a downloaded file can still be checked against its release later.
    }
    if (r && !state.blocked) {
      const from = r.asset.siteUrl ? "this site" : "GitHub";
      working(`Downloading InoInk ${r.version}`, `${mb(r.asset.size)} from ${from}.`);
      try {
        const bytes = await kobo.downloadPackage(r.asset, {
          onProgress: (f) => working(`Downloading InoInk ${r.version}`, `${mb(r.asset.size)} from ${from}.`, f),
        });
        await kobo.verifyAsset(bytes, r.asset);
        return copy(bytes);
      } catch (e) {
        if (!(e instanceof kobo.DownloadBlocked)) throw e;
        state.blocked = true; // GitHub sends no CORS headers for release files; do not ask again
      }
    }
    if (r) {
      const a = field("download");
      a.href = r.asset.downloadUrl;
      a.textContent = `Download ${r.asset.name} (${mb(r.asset.size)})`;
    }
    field("file").value = "";
    step("package");
  });
}

function fromFile(file) {
  if (!file) return;
  return busy(async () => {
    working("Checking the package", file.name);
    const bytes = new Uint8Array(await file.arrayBuffer());
    let r = state.release;
    if (!r || bytes.length !== r.asset.size || (await kobo.sha256Hex(bytes)) !== r.asset.sha256) {
      const pkg = await kobo.readPackage(bytes);
      try {
        r = await kobo.releaseFor(pkg.version);
      } catch {
        throw new Error(`InoInk ${pkg.version} is not a published release, or GitHub could not be reached, so this file cannot be checked. Download a release package, or install with install-inoink.sh.`);
      }
      await kobo.verifyAsset(bytes, r.asset);
    }
    await copy(bytes);
  });
}

async function copy(bytes) {
  working("Checking the package");
  const pkg = await kobo.readPackage(bytes);
  const wasSignedIn = state.status.signedIn;
  const menu = state.status.nickelMenu && field("menu").checked;
  const r = await kobo.install(state.root, bytes, pkg, {
    menu,
    onProgress: (phase, f) =>
      phase === "copy" ? working(`Copying InoInk ${pkg.version}`, "Keep the Kobo connected.", f) : working("Checking the copy"),
  });
  state.status = await kobo.inspect(state.root);
  field("done-title").textContent = `InoInk ${r.version} is on your Kobo`;
  const next = [
    "Eject the Kobo now (in Finder or File Explorer), unplug it, and open InoInk: it finishes installing as it starts.",
  ];
  if (r.menu === "no-nickelmenu" || (!menu && !state.status.menuEntry)) {
    next.push("There is no NickelMenu entry, so start .adds/inoink/current/inoink.sh with your launcher.");
  }
  next.push(
    wasSignedIn
      ? "Your sign-in and data are kept."
      : "Then sign in: on the Kobo, open Main menu > Settings > Accounts > Set up from a phone; or connect it again and run in a Mac or Linux terminal: curl -fsSL https://justmytwospence.github.io/InoInk/install.sh | sh -s -- --accounts-only",
  );
  field("done-message").textContent = next.join(" ");
  step("done");
}

function confirmRemove() {
  return busy(async () => {
    const level = box.querySelector('input[name="remove-level"]:checked').value;
    working("Removing InoInk");
    const removed = await kobo.removeInoInk(state.root, level);
    state.status = await kobo.inspect(state.root);
    field("done-title").textContent = removed ? "InoInk is removed" : "InoInk was not installed";
    const kept = {
      keep: "Your data is kept in .adds/inoink/data.",
      settings: "Its settings were reset; your sign-in, articles and highlights are kept in .adds/inoink/data.",
      all: "Its data was deleted too.",
    }[level];
    field("done-message").textContent = `${removed ? `${kept} ` : ""}Eject the Kobo before unplugging it.`;
    step("done");
  });
}

function restart() {
  state.root = state.device = state.status = null;
  show("pick-error", false);
  step("intro");
}

function setup() {
  renderDevices();
  if (!box) return;
  const ok = supported();
  show("unsupported", !ok);
  action("pick")[0].disabled = !ok;
  const on = (name, fn) => action(name).forEach((b) => b.addEventListener("click", fn));
  on("pick", pick);
  on("install", startInstall);
  on("remove", () => step("remove"));
  on("remove-confirm", confirmRemove);
  on("back", () => (state.root ? (renderDevice(), step("device")) : step("intro")));
  on("restart", restart);
  on("karakeep-save", () =>
    busy(async () => {
      await kobo.saveKarakeep(state.root, field("karakeep-url").value, field("karakeep-key").value);
      field("karakeep-key").value = "";
      field("karakeep-status").textContent = "Saved. InoInk picks it up the next time it starts.";
    }),
  );
  field("file").addEventListener("change", (e) => fromFile(e.target.files[0]));
  window.addEventListener("beforeunload", (e) => {
    if (state.busy) e.preventDefault();
  });
}

setup();
