import test from "node:test";
import assert from "node:assert/strict";
import { relayTarget } from "../site/relay.js";

const N = "0123456789abcdef0123456789abcdef";
const q = (host, port, extra = "&code=c") => `?state=${N}_${host}_${port}${extra}`;

test("private and loopback targets are forwarded with the query intact", () => {
  for (const h of ["127.0.0.1", "localhost", "10.0.0.5", "192.168.1.20", "172.16.0.1", "172.31.255.1"]) {
    assert.equal(relayTarget(q(h, 8123)), `http://${h}:8123/callback${q(h, 8123)}`);
  }
  assert.ok(relayTarget(q("127.0.0.1", 1, "&error=access_denied")));
});

test("anything else is refused", () => {
  for (const [h, p] of [["8.8.8.8", 80], ["172.32.0.1", 80], ["256.1.1.1", 80], ["127.0.0.1", 0], ["127.0.0.1", 70000], ["example.com", 80]]) {
    assert.equal(relayTarget(q(h, p)), null, `${h}:${p}`);
  }
  assert.equal(relayTarget(`?state=zz${N.slice(2)}_127.0.0.1_80&code=c`), null);
  assert.equal(relayTarget(q("127.0.0.1", 80, "")), null, "no code");
  assert.equal(relayTarget(`?state=${N}_127.0.0.1_80_x&code=c`), null);
  assert.equal(relayTarget(""), null);
});
