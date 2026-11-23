// Checks for the screen review tool: node --test scripts/review/
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import zlib from 'node:zlib';
import { ask, beforeImage, cleanEntry, createRound, createServer, listRounds, loadRound, main, notes, pngSize, reply, setBaseline, verdict } from './review.mjs';

function crc32(buf) {
	let c = ~0;
	for (const b of buf) {
		c ^= b;
		for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
	}
	return ~c >>> 0;
}
function chunk(type, data) {
	const out = Buffer.alloc(12 + data.length);
	out.writeUInt32BE(data.length, 0);
	out.write(type, 4, 'latin1');
	data.copy(out, 8);
	out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
	return out;
}
// A real grey PNG whose pixels depend on `shade`.
function png(width, height, shade) {
	const ihdr = Buffer.alloc(13);
	ihdr.writeUInt32BE(width, 0);
	ihdr.writeUInt32BE(height, 4);
	ihdr[8] = 8; // bit depth
	ihdr[9] = 0; // greyscale
	const raw = Buffer.alloc((width + 1) * height, shade);
	for (let y = 0; y < height; y++) raw[y * (width + 1)] = 0;
	return Buffer.concat([Buffer.from('89504e470d0a1a0a', 'hex'), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

function workspace(t) {
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'inoink-review-'));
	t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
	const review = path.join(dir, 'review');
	const shots = (name, screens) => {
		const d = path.join(dir, name);
		for (const [id, shade] of Object.entries(screens)) {
			const [device, file] = id.split('/');
			fs.mkdirSync(path.join(d, device), { recursive: true });
			fs.writeFileSync(path.join(d, device, `${file}.png`), png(4, 6, shade));
		}
		return d;
	};
	return { dir, review, shots };
}

test('png size and verdicts', () => {
	assert.deepEqual(pngSize(png(4, 6, 1)), { width: 4, height: 6 });
	assert.equal(pngSize(Buffer.from('not a png at all, really not')), null);
	assert.equal(verdict(undefined), 'unseen');
	assert.equal(verdict({ seen: true, note: ' ', pins: [] }), 'approved');
	assert.equal(verdict({ seen: false, note: 'x', pins: [] }), 'needs changes');
	assert.equal(verdict({ seen: true, note: '', pins: [{ x: 0, y: 0, note: '' }] }), 'needs changes');
});

test('entries are validated', () => {
	assert.deepEqual(cleanEntry({ seen: true, note: 'n', pins: [{ x: 0.5, y: 1, note: 'p', extra: 1 }], from: 3 }), { seen: true, note: 'n', pins: [{ x: 0.5, y: 1, note: 'p' }] });
	for (const bad of [null, { note: 3 }, { note: '', pins: [{ x: 2, y: 0, note: '' }] }, { note: '', pins: [{ x: 0, y: 0 }] }, { note: '', pins: 'x' }]) {
		assert.throws(() => cleanEntry(bad));
	}
});

test('rounds carry verdicts for unchanged images and show earlier notes for changed ones', (t) => {
	const { review, shots } = workspace(t);
	const one = shots('one', { 'monza/01-list': 10, 'monza/02-article': 20, 'monza/03-menu': 30 });
	const r1 = createRound(review, { sources: [one] });
	assert.equal(r1.round, 1);
	assert.deepEqual(r1.screens.map((s) => [s.id, s.status]), [['monza/01-list', 'new'], ['monza/02-article', 'new'], ['monza/03-menu', 'new']]);
	assert.equal(r1.screens[0].width, 4);
	// Images are stored once by checksum.
	assert.equal(fs.readdirSync(path.join(review, 'images')).length, 3);

	const fb = loadRound(review, 1).feedback;
	fb.screens['monza/01-list'] = { seen: true, note: '', pins: [] };
	fb.screens['monza/02-article'] = { seen: true, note: 'title too long', pins: [{ x: 0.5, y: 0.25, note: 'clipped' }] };
	fb.screens['monza/03-menu'] = { seen: true, note: 'is this right?', pins: [] };
	fs.writeFileSync(path.join(review, 'rounds/001/feedback.json'), JSON.stringify(fb));
	reply(review, 'monza/03-menu', 'Yes, by design.');
	assert.throws(() => reply(review, 'monza/99-none', 'x'));

	const two = shots('two', { 'monza/01-list': 10, 'monza/02-article': 21, 'monza/03-menu': 30, 'monza/04-new': 40 });
	const r2 = createRound(review, { sources: [two] });
	const by = Object.fromEntries(r2.screens.map((s) => [s.id, s]));
	assert.equal(by['monza/01-list'].status, 'unchanged');
	assert.equal(by['monza/02-article'].status, 'changed');
	assert.equal(by['monza/02-article'].previous.verdict, 'needs changes');
	assert.equal(by['monza/02-article'].previous.pins[0].note, 'clipped');
	assert.equal(by['monza/04-new'].status, 'new');
	assert.equal(by['monza/03-menu'].answered.reply, 'Yes, by design.');
	const fb2 = loadRound(review, 2).feedback;
	assert.deepEqual(fb2.screens['monza/01-list'], { seen: true, note: '', pins: [], from: 1 });
	assert.equal(fb2.screens['monza/02-article'], undefined, 'a changed image starts without a verdict');
	assert.equal(fb2.screens['monza/03-menu'], undefined, 'an answered screen starts unseen');
	assert.ok(!fs.existsSync(path.join(review, 'replies.json')), 'replies are delivered once');

	// The answer stays until the reviewer has seen it; a partial capture keeps the other screens.
	const three = shots('three', { 'monza/01-list': 11 });
	const r3 = createRound(review, { sources: [three] });
	const by3 = Object.fromEntries(r3.screens.map((s) => [s.id, s]));
	assert.equal(by3['monza/03-menu'].status, 'kept');
	assert.equal(by3['monza/03-menu'].answered.reply, 'Yes, by design.');
	assert.equal(by3['monza/01-list'].status, 'changed');
	assert.equal(r3.screens.length, 4);
	const r4 = createRound(review, { sources: [three], fresh: true });
	assert.deepEqual(r4.screens.map((s) => s.id), ['monza/01-list']);
	assert.deepEqual(listRounds(review), [1, 2, 3, 4]);
});

test('duplicate names, empty and missing sources are refused', (t) => {
	const { dir, review, shots } = workspace(t);
	const a = shots('a', { 'monza/01': 1 });
	assert.throws(() => createRound(review, { sources: [a, a] }), /two captures/);
	fs.mkdirSync(path.join(dir, 'empty'));
	assert.throws(() => createRound(review, { sources: [path.join(dir, 'empty')] }), /no PNG/);
	assert.throws(() => createRound(review, { sources: [path.join(dir, 'missing')] }), /not a folder/);
	// A folder of PNGs without device folders takes --device, or its own name.
	const flat = path.join(dir, 'flat');
	fs.mkdirSync(flat);
	fs.writeFileSync(path.join(flat, 'shot.png'), png(2, 2, 5));
	assert.equal(createRound(review, { sources: [flat], device: 'spaBW' }).screens[0].id, 'spaBW/shot');
	fs.writeFileSync(path.join(flat, 'bad name.png'), png(2, 2, 5));
	assert.throws(() => createRound(review, { sources: [flat] }), /unsupported screen name/);
});

test('baseline matches screens by id or by a mapped name', (t) => {
	const { review, shots } = workspace(t);
	const base = shots('base', { 'monza/01-list': 1, 'monza/menu': 2 });
	const b = setBaseline(review, { source: base, commit: 'abc123', match: ['02-main-menu=menu'] });
	assert.equal(beforeImage(b, { id: 'monza/01-list', device: 'monza', name: '01-list' }), b.screens['monza/01-list']);
	assert.equal(beforeImage(b, { id: 'monza/02-main-menu', device: 'monza', name: '02-main-menu' }), b.screens['monza/menu']);
	assert.equal(beforeImage(b, { id: 'monza/03-other', device: 'monza', name: '03-other' }), null);
	assert.throws(() => setBaseline(review, { source: base, match: ['nonsense'] }));
});

test('ask queues screens and notes lists the feedback', (t) => {
	const { review, shots } = workspace(t);
	createRound(review, { sources: [shots('s', { 'monza/01-list': 1, 'spaBW/01-list': 2, 'monza/02-x': 3 })] });
	assert.deepEqual(ask(review, '01-list', 'Are the rows even?').sort(), ['monza/01-list', 'spaBW/01-list']);
	assert.throws(() => ask(review, 'nope'));
	const fb = loadRound(review, 1).feedback;
	fb.screens['monza/02-x'] = { seen: true, note: 'Move this', pins: [{ x: 0.5, y: 0.5, note: 'here' }] };
	fs.writeFileSync(path.join(review, 'rounds/001/feedback.json'), JSON.stringify(fb));
	const text = notes(review);
	assert.match(text, /^Round 1: 3 screens, 1 seen, 1 need changes, 2 unseen\./);
	assert.match(text, /Not finished yet\./);
	assert.match(text, /monza\/01-list: unseen, queued \(not seen yet\)\n {2}image: .*\.png \(4x6\)\n {2}question: Are the rows even\?/);
	assert.match(text, /monza\/02-x: needs changes\n.*\n {2}note: Move this\n {2}pin 1 at \(2, 3\): here/);
	assert.deepEqual(ask(review, '--clear'), []);
	assert.doesNotMatch(notes(review), /queued/);
});

test('command line', async (t) => {
	const { review, shots } = workspace(t);
	const env = { INOINK_REVIEW_DIR: review };
	const s = shots('s', { 'monza/01': 1 });
	const log = t.mock.method(console, 'log', () => {});
	assert.equal(await main(['round', '--source', s], env), 0);
	assert.match(log.mock.calls.at(-1).arguments[0], /^Round 1: 1 screens \(1 new/);
	await assert.rejects(main(['round', '--bogus'], env), /unknown option/);
	await assert.rejects(main(['reply', 'monza/01'], env), /reply takes/);
	await assert.rejects(main(['frobnicate'], env), /unknown command/);
	assert.equal(await main(['notes'], env), 0);
});

async function serve(t, review) {
	const server = createServer(review, 0);
	await new Promise((r) => server.listen(0, '127.0.0.1', r));
	t.after(() => server.close());
	const port = server.address().port;
	const call = (p, opts = {}) =>
		fetch(`http://127.0.0.1:${port}${p}`, opts).then(async (r) => ({ status: r.status, type: r.headers.get('content-type'), body: r.headers.get('content-type')?.startsWith('application/json') ? await r.json() : await r.arrayBuffer() }));
	return { port, call };
}

test('server saves feedback for the latest round only', async (t) => {
	const { review, shots } = workspace(t);
	createRound(review, { sources: [shots('s', { 'monza/01': 1, 'monza/02': 2 })] });
	const { call } = await serve(t, review);
	const json = { 'Content-Type': 'application/json' };

	const page = await call('/');
	assert.equal(page.status, 200);
	assert.match(page.type, /text\/html/);
	const state = await call('/api/state');
	assert.deepEqual(state.body, { latest: 1, rounds: [1] });
	const round = await call('/api/rounds/1');
	const sha = round.body.round.screens[0].image;
	const img = await call(`/images/${sha}.png`);
	assert.equal(img.status, 200);
	assert.equal(img.type, 'image/png');
	assert.equal((await call('/images/..%2F..%2Fetc.png')).status, 404);

	const put = (n, id, body, headers = json) => call(`/api/rounds/${n}/screens/${encodeURIComponent(id)}`, { method: 'PUT', headers, body: JSON.stringify(body) });
	assert.equal((await put(1, 'monza/01', { seen: true, note: 'hello', pins: [{ x: 0.1, y: 0.2, note: 'p' }] })).status, 200);
	assert.equal(loadRound(review, 1).feedback.screens['monza/01'].note, 'hello');
	assert.equal((await put(1, 'monza/99', { seen: true, note: '', pins: [] })).status, 404);
	assert.equal((await put(1, 'monza/01', { seen: true, note: '', pins: [{ x: 5, y: 0, note: '' }] })).status, 400);
	// Writes need JSON from this page's own origin.
	assert.equal((await put(1, 'monza/01', { seen: true, note: '', pins: [] }, { 'Content-Type': 'text/plain' })).status, 403);
	assert.equal((await put(1, 'monza/01', { seen: true, note: '', pins: [] }, { ...json, Origin: 'http://evil.example' })).status, 403);

	const fin = await call('/api/rounds/1/finish', { method: 'POST', headers: json, body: '{}' });
	assert.ok(fin.body.feedback.finished);
	assert.equal((await put(1, 'monza/02', { seen: true, note: '', pins: [] })).status, 200);
	assert.equal(loadRound(review, 1).feedback.finished, null, 'a change after Finish reopens the round');

	createRound(review, { sources: [shots('t', { 'monza/01': 1 })] });
	assert.equal((await put(1, 'monza/01', { seen: false, note: '', pins: [] })).status, 409);
	assert.equal((await call('/api/rounds/2')).body.latest, 2);
	assert.equal((await call('/api/rounds/7')).status, 404);
});

test('server refuses other host names', async (t) => {
	const { review, shots } = workspace(t);
	createRound(review, { sources: [shots('s', { 'monza/01': 1 })] });
	const { port } = await serve(t, review);
	const http = await import('node:http');
	const status = await new Promise((resolve, reject) => {
		const req = http.request({ host: '127.0.0.1', port, path: '/api/state', headers: { Host: `attacker.example:${port}` } }, (res) => {
			res.resume();
			resolve(res.statusCode);
		});
		req.on('error', reject);
		req.end();
	});
	assert.equal(status, 403);
});
