#!/usr/bin/env node
// Screen review: rounds of emulator and preview screenshots, a local page that shows them one at a
// time with notes pinned to places on a screen, and the notes back on the command line.
// Node.js only, no packages. See doc/INOINK-SCREEN-REVIEW.md.
// Adapted from the idea of Kobalt's screen review (https://github.com/nicoverbruggen/kobalt, MIT).
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const ROOT = path.resolve(HERE, '../..');
const PAGE = path.join(HERE, 'page.html');
const DEFAULT_SOURCES = ['target/macos-preview/captures/sample', 'target/inoink-emulator-shots'];
const SHA = /^[0-9a-f]{64}$/;
const SCREEN_ID = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const MAX_BODY = 1 << 20;

export class UsageError extends Error {}

// ---- storage ---------------------------------------------------------------------------------

export function reviewDir(env = process.env) {
	return path.resolve(ROOT, env.INOINK_REVIEW_DIR || 'target/screen-review');
}

function readJson(file, fallback) {
	try {
		return JSON.parse(fs.readFileSync(file, 'utf8'));
	} catch (e) {
		if (e.code === 'ENOENT') return fallback;
		throw new Error(`cannot read ${file}: ${e.message}`);
	}
}

// Write to a temporary file and rename: a reader never sees half a file.
function writeJson(file, value) {
	fs.mkdirSync(path.dirname(file), { recursive: true });
	const tmp = `${file}.${process.pid}.tmp`;
	fs.writeFileSync(tmp, `${JSON.stringify(value, null, '\t')}\n`);
	fs.renameSync(tmp, file);
}

const pad = (n) => String(n).padStart(3, '0');
const roundDir = (dir, n) => path.join(dir, 'rounds', pad(n));

export function listRounds(dir) {
	let names = [];
	try {
		names = fs.readdirSync(path.join(dir, 'rounds'));
	} catch (e) {
		if (e.code !== 'ENOENT') throw e;
	}
	return names
		.filter((n) => /^\d{3,}$/.test(n) && fs.existsSync(path.join(dir, 'rounds', n, 'round.json')))
		.map(Number)
		.sort((a, b) => a - b);
}

export function latestRound(dir) {
	const rounds = listRounds(dir);
	return rounds.length ? rounds[rounds.length - 1] : 0;
}

export function loadRound(dir, n) {
	const d = roundDir(dir, n);
	const round = readJson(path.join(d, 'round.json'), null);
	if (!round) throw new UsageError(`round ${n} does not exist`);
	const feedback = readJson(path.join(d, 'feedback.json'), { finished: null, screens: {} });
	const queue = readJson(path.join(d, 'queue.json'), {});
	return { round, feedback, queue };
}

function requireLatest(dir) {
	const n = latestRound(dir);
	if (!n) throw new UsageError('no round yet; run "node scripts/review/review.mjs round" first');
	return n;
}

// ---- images ----------------------------------------------------------------------------------

export function pngSize(buffer) {
	const signature = '89504e470d0a1a0a';
	if (buffer.length < 24 || buffer.subarray(0, 8).toString('hex') !== signature || buffer.toString('latin1', 12, 16) !== 'IHDR') {
		return null;
	}
	return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
}

// Each image is stored once, named by its SHA-256: rounds share unchanged images.
function storeImage(dir, file) {
	const data = fs.readFileSync(file);
	const size = pngSize(data);
	if (!size) throw new UsageError(`${file} is not a PNG image`);
	const sha = createHash('sha256').update(data).digest('hex');
	const target = path.join(dir, 'images', `${sha}.png`);
	if (!fs.existsSync(target)) {
		fs.mkdirSync(path.dirname(target), { recursive: true });
		fs.writeFileSync(target, data);
	}
	return { image: sha, ...size };
}

// A capture folder holds PNGs directly (one device, named by --device) or one folder of PNGs per
// device, named by its codename, as the emulator tour and the preview capture write them.
export function collectCaptures(source, device) {
	const abs = path.resolve(ROOT, source);
	if (!fs.existsSync(abs) || !fs.statSync(abs).isDirectory()) throw new UsageError(`${source} is not a folder`);
	const found = [];
	const pngs = (d) =>
		fs
			.readdirSync(d)
			.filter((f) => f.toLowerCase().endsWith('.png') && fs.statSync(path.join(d, f)).isFile())
			.sort();
	const direct = pngs(abs);
	if (direct.length) {
		const dev = device || path.basename(abs);
		for (const f of direct) found.push({ device: dev, name: f.replace(/\.png$/i, ''), file: path.join(abs, f) });
	}
	for (const sub of fs.readdirSync(abs).sort()) {
		const d = path.join(abs, sub);
		if (sub.startsWith('.') || !fs.statSync(d).isDirectory()) continue;
		for (const f of pngs(d)) found.push({ device: sub, name: f.replace(/\.png$/i, ''), file: path.join(d, f) });
	}
	for (const c of found) {
		c.id = `${c.device}/${c.name}`;
		if (!SCREEN_ID.test(c.id)) throw new UsageError(`unsupported screen name ${c.id}: use letters, digits, dot, dash and underscore`);
	}
	return found;
}

// ---- feedback --------------------------------------------------------------------------------

export function hasNotes(entry) {
	return !!entry && ((entry.note || '').trim() !== '' || (entry.pins || []).length > 0);
}

export function verdict(entry) {
	if (hasNotes(entry)) return 'needs changes';
	return entry && entry.seen ? 'approved' : 'unseen';
}

function cleanPins(pins) {
	if (!Array.isArray(pins) || pins.length > 200) throw new UsageError('pins must be a list of at most 200 pins');
	return pins.map((p) => {
		const x = Number(p && p.x);
		const y = Number(p && p.y);
		if (!Number.isFinite(x) || !Number.isFinite(y) || x < 0 || x > 1 || y < 0 || y > 1) throw new UsageError('pin positions are fractions from 0 to 1');
		if (typeof p.note !== 'string' || p.note.length > 20000) throw new UsageError('a pin note must be text');
		return { x, y, note: p.note };
	});
}

export function cleanEntry(body) {
	if (!body || typeof body !== 'object') throw new UsageError('expected an object');
	if (typeof body.note !== 'string' || body.note.length > 50000) throw new UsageError('note must be text');
	return { seen: body.seen === true, note: body.note, pins: cleanPins(body.pins || []) };
}

// ---- commands --------------------------------------------------------------------------------

export function createRound(dir, { sources, device, fresh = false, now = new Date() } = {}) {
	// Without sources: the first default capture folder that exists.
	const list = sources && sources.length ? sources : DEFAULT_SOURCES.filter((s) => fs.existsSync(path.resolve(ROOT, s))).slice(0, 1);
	if (!list.length) {
		throw new UsageError('no captures found; run "scripts/preview-macos.sh --capture" or pass --source DIR');
	}
	const captures = list.flatMap((s) => collectCaptures(s, device));
	if (!captures.length) throw new UsageError(`no PNG images in ${list.join(', ')}`);
	const seen = new Set();
	for (const c of captures) {
		if (seen.has(c.id)) throw new UsageError(`two captures are both named ${c.id}`);
		seen.add(c.id);
	}

	const prevN = latestRound(dir);
	const prev = prevN ? loadRound(dir, prevN) : null;
	const prevScreens = new Map((prev ? prev.round.screens : []).map((s) => [s.id, s]));
	const replies = readJson(path.join(dir, 'replies.json'), {});
	const n = prevN + 1;
	const delivered = new Set();
	const screens = [];
	const feedback = { finished: null, screens: {} };

	const carry = (screen, old, entry) => {
		// An answer the reviewer has not seen yet stays with the screen.
		if (old.answered && !(entry && entry.seen)) screen.answered = old.answered;
	};

	for (const c of captures) {
		const stored = storeImage(dir, c.file);
		const screen = { id: c.id, device: c.device, name: c.name, ...stored };
		const old = prevScreens.get(c.id);
		const entry = old && prev.feedback.screens[c.id];
		if (!old) {
			screen.status = 'new';
		} else {
			screen.status = old.image === stored.image ? 'unchanged' : 'changed';
			screen.previous = { round: prevN, image: old.image, width: old.width, height: old.height };
			if (entry) {
				screen.previous.verdict = verdict(entry);
				screen.previous.note = entry.note || '';
				screen.previous.pins = entry.pins || [];
			}
			if (replies[c.id]) {
				delivered.add(c.id);
				screen.answered = { round: prevN, note: entry ? entry.note || '' : '', pins: entry ? entry.pins || [] : [], reply: replies[c.id].text };
			} else if (screen.status === 'unchanged' && entry) {
				feedback.screens[c.id] = { seen: !!entry.seen, note: entry.note || '', pins: entry.pins || [], from: entry.from || prevN };
				carry(screen, old, entry);
			} else {
				carry(screen, old, entry);
			}
		}
		screens.push(screen);
	}
	// A capture of only some screens keeps the others, with their feedback, from the last round.
	if (prev && !fresh) {
		for (const old of prev.round.screens) {
			if (seen.has(old.id)) continue;
			const screen = { ...old, status: 'kept' };
			delete screen.previous;
			screens.push(screen);
			const entry = prev.feedback.screens[old.id];
			if (entry) feedback.screens[old.id] = { ...entry, from: entry.from || prevN };
		}
	}
	screens.sort((a, b) => (a.device === b.device ? a.name.localeCompare(b.name) : a.device.localeCompare(b.device)));

	const round = { round: n, created: now.toISOString(), sources: list, previousRound: prevN || null, screens };
	writeJson(path.join(roundDir(dir, n), 'round.json'), round);
	writeJson(path.join(roundDir(dir, n), 'feedback.json'), feedback);
	// Replies are delivered with this round; one for a screen that was not captured waits.
	if (delivered.size) {
		for (const id of delivered) delete replies[id];
		if (Object.keys(replies).length) writeJson(path.join(dir, 'replies.json'), replies);
		else fs.rmSync(path.join(dir, 'replies.json'), { force: true });
	}
	return round;
}

export function setBaseline(dir, { source, device, commit = null, ref = null, match = [], now = new Date() }) {
	if (!source) throw new UsageError('baseline needs --source DIR');
	const captures = collectCaptures(source, device);
	if (!captures.length) throw new UsageError(`no PNG images in ${source}`);
	const screens = {};
	for (const c of captures) screens[c.id] = storeImage(dir, c.file).image;
	const pairs = {};
	for (const m of match) {
		const [now_, before] = m.split('=');
		if (!now_ || !before) throw new UsageError(`--match takes NAME=BEFORE, got ${m}`);
		pairs[now_] = before;
	}
	const baseline = { created: now.toISOString(), source, commit, ref, screens, match: pairs };
	writeJson(path.join(dir, 'baseline.json'), baseline);
	return baseline;
}

// The Before image of a screen: the same id, or the base screen it replaces (--match NAME=BEFORE,
// by screen name or full id).
export function beforeImage(baseline, screen) {
	if (!baseline) return null;
	if (baseline.screens[screen.id]) return baseline.screens[screen.id];
	const m = baseline.match[screen.id] || baseline.match[screen.name];
	if (!m) return null;
	return baseline.screens[m.includes('/') ? m : `${screen.device}/${m}`] || null;
}

export function reply(dir, id, text) {
	const n = requireLatest(dir);
	const { round } = loadRound(dir, n);
	if (!round.screens.some((s) => s.id === id)) throw new UsageError(`round ${n} has no screen ${id}`);
	if (!text || !text.trim()) throw new UsageError('reply needs a text');
	const replies = readJson(path.join(dir, 'replies.json'), {});
	replies[id] = { text, round: n };
	writeJson(path.join(dir, 'replies.json'), replies);
}

export function ask(dir, target, question) {
	const n = requireLatest(dir);
	const { round, queue } = loadRound(dir, n);
	const file = path.join(roundDir(dir, n), 'queue.json');
	if (target === '--clear') {
		writeJson(file, {});
		return [];
	}
	const ids = round.screens.filter((s) => s.id === target || s.name === target).map((s) => s.id);
	if (!ids.length) throw new UsageError(`round ${n} has no screen ${target}`);
	for (const id of ids) queue[id] = question || '';
	writeJson(file, queue);
	return ids;
}

function rel(file) {
	const r = path.relative(process.cwd(), file);
	return !r || r.startsWith('..') ? file : r;
}

export function notes(dir, n = 0) {
	const latest = requireLatest(dir);
	n = n || latest;
	const { round, feedback, queue } = loadRound(dir, n);
	const baseline = readJson(path.join(dir, 'baseline.json'), null);
	const replies = readJson(path.join(dir, 'replies.json'), {});
	const img = (sha) => rel(path.join(dir, 'images', `${sha}.png`));
	const counts = { seen: 0, needs: 0, unseen: 0 };
	for (const s of round.screens) {
		const e = feedback.screens[s.id];
		if (e && e.seen) counts.seen++;
		else counts.unseen++;
		if (hasNotes(e)) counts.needs++;
	}
	const out = [];
	out.push(`Round ${n}${n === latest ? '' : ` (latest is ${latest})`}: ${round.screens.length} screens, ${counts.seen} seen, ${counts.needs} need changes, ${counts.unseen} unseen.`);
	out.push(feedback.finished ? `Finished by the reviewer at ${feedback.finished}.` : 'Not finished yet.');
	for (const s of round.screens) {
		const e = feedback.screens[s.id];
		const queued = Object.prototype.hasOwnProperty.call(queue, s.id);
		if (!hasNotes(e) && !queued && !replies[s.id]) continue;
		out.push('');
		out.push(`${s.id}: ${verdict(e)}${queued ? (e && e.seen ? ', queued (seen)' : ', queued (not seen yet)') : ''}${e && e.from ? ` (from round ${e.from})` : ''}`);
		out.push(`  image: ${img(s.image)} (${s.width}x${s.height})`);
		const before = beforeImage(baseline, s);
		if (before) out.push(`  before: ${img(before)}${baseline.commit ? ` (${baseline.commit.slice(0, 12)})` : ''}`);
		if (queued && queue[s.id]) out.push(`  question: ${queue[s.id]}`);
		if (e && e.note && e.note.trim()) out.push(`  note: ${e.note.trim().replace(/\n/g, '\n        ')}`);
		(e ? e.pins : []).forEach((p, i) => {
			out.push(`  pin ${i + 1} at (${Math.round(p.x * s.width)}, ${Math.round(p.y * s.height)}): ${p.note.trim().replace(/\n/g, '\n    ') || '(no text)'}`);
		});
		if (replies[s.id]) out.push(`  reply for the next round: ${replies[s.id].text}`);
	}
	return out.join('\n');
}

// ---- server ----------------------------------------------------------------------------------

function send(res, status, body, type = 'application/json; charset=utf-8') {
	const data = typeof body === 'string' || Buffer.isBuffer(body) ? body : JSON.stringify(body);
	res.writeHead(status, { 'Content-Type': type, 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' });
	res.end(data);
}

function readBody(req) {
	return new Promise((resolve, reject) => {
		let size = 0;
		const chunks = [];
		req.on('data', (c) => {
			size += c.length;
			if (size > MAX_BODY) {
				reject(new UsageError('request too large'));
				req.destroy();
			} else chunks.push(c);
		});
		req.on('end', () => {
			try {
				resolve(JSON.parse(Buffer.concat(chunks).toString('utf8')));
			} catch {
				reject(new UsageError('invalid JSON'));
			}
		});
		req.on('error', reject);
	});
}

// Only this computer, by name: a page on another site cannot reach the server through DNS
// rebinding, and a write needs a JSON request from this page's own origin.
function allowed(req, port) {
	const hosts = [`localhost:${port}`, `127.0.0.1:${port}`];
	if (!hosts.includes(req.headers.host)) return false;
	if (req.method !== 'GET') {
		const origin = req.headers.origin;
		if (origin && !hosts.some((h) => origin === `http://${h}`)) return false;
		if (!(req.headers['content-type'] || '').startsWith('application/json')) return false;
	}
	return true;
}

export function roundPayload(dir, n) {
	const { round, feedback, queue } = loadRound(dir, n);
	const baseline = readJson(path.join(dir, 'baseline.json'), null);
	const before = {};
	if (baseline) for (const s of round.screens) before[s.id] = beforeImage(baseline, s);
	return {
		round,
		feedback,
		queue,
		latest: latestRound(dir),
		baseline: baseline ? { commit: baseline.commit, ref: baseline.ref, created: baseline.created, images: before } : null,
	};
}

export function createServer(dir, port) {
	const server = http.createServer(async (req, res) => {
		try {
			const actualPort = server.address().port;
			if (!allowed(req, port || actualPort)) return send(res, 403, { error: 'forbidden' });
			const url = new URL(req.url, 'http://localhost');
			const parts = url.pathname.split('/').filter(Boolean).map(decodeURIComponent);
			if (req.method === 'GET' && url.pathname === '/') {
				return send(res, 200, fs.readFileSync(PAGE), 'text/html; charset=utf-8');
			}
			if (req.method === 'GET' && parts[0] === 'images' && parts.length === 2) {
				const sha = parts[1].replace(/\.png$/, '');
				if (!SHA.test(sha)) return send(res, 404, { error: 'not found' });
				const file = path.join(dir, 'images', `${sha}.png`);
				if (!fs.existsSync(file)) return send(res, 404, { error: 'not found' });
				res.writeHead(200, { 'Content-Type': 'image/png', 'Cache-Control': 'max-age=31536000, immutable' });
				return fs.createReadStream(file).pipe(res);
			}
			if (parts[0] !== 'api') return send(res, 404, { error: 'not found' });
			if (req.method === 'GET' && parts[1] === 'state' && parts.length === 2) {
				return send(res, 200, { latest: latestRound(dir), rounds: listRounds(dir) });
			}
			if (parts[1] !== 'rounds' || !/^\d+$/.test(parts[2] || '')) return send(res, 404, { error: 'not found' });
			const n = Number(parts[2]);
			if (!listRounds(dir).includes(n)) return send(res, 404, { error: `round ${n} does not exist` });
			if (req.method === 'GET' && parts.length === 3) return send(res, 200, roundPayload(dir, n));
			if (req.method === 'GET') return send(res, 404, { error: 'not found' });
			// Only the latest round takes feedback; earlier rounds are read only.
			if (n !== latestRound(dir)) return send(res, 409, { error: 'a newer round exists; this one is read only' });
			const file = path.join(roundDir(dir, n), 'feedback.json');
			const { round, feedback } = loadRound(dir, n);
			if (req.method === 'PUT' && parts[3] === 'screens' && parts.length === 5) {
				const id = parts[4];
				if (!round.screens.some((s) => s.id === id)) return send(res, 404, { error: 'no such screen' });
				const entry = cleanEntry(await readBody(req));
				feedback.screens[id] = entry;
				// A change after Finish review reopens the round.
				feedback.finished = null;
				writeJson(file, feedback);
				return send(res, 200, { ok: true, feedback });
			}
			if (req.method === 'POST' && parts[3] === 'finish' && parts.length === 4) {
				await readBody(req);
				feedback.finished = new Date().toISOString();
				writeJson(file, feedback);
				return send(res, 200, { ok: true, feedback });
			}
			return send(res, 404, { error: 'not found' });
		} catch (e) {
			if (e instanceof UsageError) return send(res, 400, { error: e.message });
			console.error(e);
			return send(res, 500, { error: 'internal error' });
		}
	});
	return server;
}

// ---- command line ----------------------------------------------------------------------------

const USAGE = `Usage: node scripts/review/review.mjs COMMAND
  round [--source DIR]... [--device NAME] [--fresh]   create the next round from captures
  baseline --source DIR [--device NAME] [--commit SHA] [--ref REF] [--match NAME=BEFORE]...
                                                      record the Before screens
  serve [--port N]                                    serve the review page on this computer
  notes [--round N]                                   print the reviewer's notes
  reply SCREEN TEXT                                   answer a note in the next round
  ask SCREEN [QUESTION] | ask --clear                 queue screens for the reviewer
Data lives in target/screen-review (INOINK_REVIEW_DIR overrides).`;

function options(args, spec) {
	const out = { _: [] };
	for (let i = 0; i < args.length; i++) {
		const a = args[i];
		if (!a.startsWith('--') || a === '--clear') {
			out._.push(a);
			continue;
		}
		const key = a.slice(2);
		if (!(key in spec)) throw new UsageError(`unknown option ${a}`);
		if (spec[key] === 'flag') {
			out[key] = true;
			continue;
		}
		const value = args[++i];
		if (value === undefined) throw new UsageError(`${a} needs a value`);
		if (spec[key] === 'list') (out[key] ||= []).push(value);
		else out[key] = value;
	}
	return out;
}

export async function main(argv, env = process.env) {
	const [command, ...rest] = argv;
	const dir = reviewDir(env);
	switch (command) {
		case 'round': {
			const o = options(rest, { source: 'list', device: 'value', fresh: 'flag' });
			const r = createRound(dir, { sources: o.source, device: o.device, fresh: o.fresh });
			const by = (st) => r.screens.filter((s) => s.status === st).length;
			console.log(`Round ${r.round}: ${r.screens.length} screens (${by('new')} new, ${by('changed')} changed, ${by('unchanged')} unchanged, ${by('kept')} kept).`);
			return 0;
		}
		case 'baseline': {
			const o = options(rest, { source: 'value', device: 'value', commit: 'value', ref: 'value', match: 'list' });
			const b = setBaseline(dir, { source: o.source, device: o.device, commit: o.commit || null, ref: o.ref || null, match: o.match || [] });
			console.log(`Before: ${Object.keys(b.screens).length} screens from ${b.source}.`);
			return 0;
		}
		case 'serve': {
			const o = options(rest, { port: 'value' });
			const port = o.port === undefined ? 8765 : Number(o.port);
			if (!Number.isInteger(port) || port < 0 || port > 65535) throw new UsageError('invalid port');
			requireLatest(dir);
			const server = createServer(dir, port);
			await new Promise((resolve, reject) => {
				server.once('error', reject);
				server.listen(port, '127.0.0.1', resolve);
			});
			console.log(`Screen review at http://localhost:${server.address().port}/ (this computer only). Ctrl-C stops it.`);
			return new Promise(() => {});
		}
		case 'notes': {
			const o = options(rest, { round: 'value' });
			console.log(notes(dir, o.round ? Number(o.round) : 0));
			return 0;
		}
		case 'reply': {
			if (rest.length !== 2) throw new UsageError('reply takes SCREEN and TEXT');
			reply(dir, rest[0], rest[1]);
			console.log(`Reply saved for ${rest[0]}; it shows in the next round.`);
			return 0;
		}
		case 'ask': {
			if (!rest.length || rest.length > 2) throw new UsageError('ask takes SCREEN and an optional QUESTION, or --clear');
			const ids = ask(dir, rest[0], rest[1]);
			console.log(rest[0] === '--clear' ? 'Queue cleared.' : `Queued ${ids.join(', ')}.`);
			return 0;
		}
		case undefined:
		case 'help':
		case '--help':
			console.log(USAGE);
			return command ? 0 : 2;
		default:
			throw new UsageError(`unknown command ${command}`);
	}
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
	main(process.argv.slice(2)).then(
		(code) => process.exit(code),
		(e) => {
			console.error(`error: ${e.message}`);
			if (e instanceof UsageError) console.error(USAGE);
			process.exit(e instanceof UsageError ? 2 : 1);
		},
	);
}
