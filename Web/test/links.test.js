// Runs the real phone page script against fake WebSocket / Web Bluetooth and checks the multi-link behaviour.
// node Web/test/links.test.js
const fs = require('fs');
const assert = require('assert');
const path = require('path');
const html = fs.readFileSync(path.join(__dirname, '..', 'index.html'), 'utf8');
let code = html.match(/<script>([\s\S]*)<\/script>/)[1];
code = code.replace('let BEAT_MS = 1000, DEAD_MS = 3500;', 'let BEAT_MS = 40, DEAD_MS = 250;');   // fast watchdog for the test

const wait = ms => new Promise(r => setTimeout(r, ms));
const until = async (f, ms = 3000) => { const t = Date.now(); while (Date.now() - t < ms) { if (f()) return true; await wait(5); } return false; };
const proxy = () => { const t = function () {}; const p = new Proxy(t, { get: (o, k) => (k in o ? o[k] : k === Symbol.toPrimitive ? () => '' : p), set: (o, k, v) => { o[k] = v; return true; }, apply: () => p }); return p; };

function world({ echo = true } = {}) {
  const w = { sockets: [], ble: { writes: [], connected: false, listeners: [] }, echo };
  class FakeWS {
    constructor(url) { this.url = url; this.readyState = 0; this.sent = []; this.bufferedAmount = 0; w.sockets.push(this); setTimeout(() => { this.readyState = 1; this.onopen && this.onopen(); }, 1); }
    send(d) { this.sent.push(d); if (typeof d === 'string' && w.echo && this.readyState === 1) setTimeout(() => this.onmessage && this.onmessage({ data: 'k' }), 1); }
    close() { this.readyState = 3; }
    drop() { this.readyState = 3; this.onclose && this.onclose(); }
    status(state, paused, name) { const b = new Uint8Array([state, paused, ...Buffer.from(name)]).buffer; this.onmessage && this.onmessage({ data: b }); }
  }
  const inp = { writeValueWithoutResponse: async b => { w.ble.writes.push(new Uint8Array(b)); } };
  const sta = { startNotifications: async () => {}, readValue: async () => new DataView(new Uint8Array([1, 0, 80, 67]).buffer), value: new DataView(new Uint8Array([1, 0, 80, 67]).buffer) };
  const svc = { getCharacteristic: async u => (u.endsWith('0002-3c2b-4f7d-9b61-2a9e5d0c4f10') ? inp : sta) };
  const dev = { id: 'dev1', addEventListener: (n, f) => w.ble.listeners.push(f), gatt: { get connected() { return w.ble.connected; }, connect: async () => { w.ble.connected = true; return { getPrimaryService: async () => svc }; }, disconnect: () => { w.ble.connected = false; w.ble.listeners.splice(0).forEach(f => f()); } } };
  w.dev = dev;
  const store = {}; w.store = store;
  const env = {
    document: { getElementById: () => proxy(), querySelectorAll: () => [], addEventListener() {}, visibilityState: 'visible' },
    window: { isSecureContext: true }, addEventListener() {}, removeEventListener() {},
    navigator: { userAgent: 'Android', bluetooth: { getDevices: async () => [dev], requestDevice: async () => dev }, vibrate() {}, wakeLock: { request: async () => ({}) }, getBattery: async () => ({ level: 0.5 }) },
    location: { protocol: 'http:', host: 'pc:47821', origin: 'http://pc:47821' },
    WebSocket: FakeWS, localStorage: { getItem: k => store[k] ?? null, setItem: (k, v) => { store[k] = String(v); }, removeItem: k => { delete store[k]; } },
    DeviceMotionEvent: undefined, DeviceOrientationEvent: undefined, crypto: require('crypto').webcrypto,
  };
  const fn = new Function(...Object.keys(env), code + '\nreturn { start, stop, links, wifi, ble, push, timed, K, pair, aimAll, primary, setSamples(a, b) { s1 = a; s2 = b; }, mvAdd(x, y) { mv[0] += x; mv[1] += y; }, getMv: () => mv, getState: () => state, sendTextOut, sendKeyOut, setTune, getTune: () => tune, updateCode };');
  w.page = fn(...Object.values(env));
  return w;
}
const kinds = arr => arr.map(b => (typeof b === 'string' ? b : new Uint8Array(b)[0]));
const nonText = s => s.sent.filter(x => typeof x !== 'string');
const kindsOf = s => nonText(s).map(b => new Uint8Array(b)[0]);

const tests = {
  async 'tune goes out once on every link after approval, text and keys on the primary only'(w) {
    await w.page.start(); await until(() => w.page.wifi.up && w.page.ble.up);
    w.sockets[0].status(0, 0, 'PC');
    assert(!kindsOf(w.sockets[0]).includes(12), 'no tune before approval');
    w.sockets[0].status(1, 0, 'PC');
    assert(await until(() => kindsOf(w.sockets[0]).includes(12) && w.ble.writes.some(b => b[0] === 12)), 'tune on both');
    w.sockets[0].status(1, 0, 'PC'); w.sockets[0].status(1, 0, 'PC');
    await new Promise(r => setTimeout(r, 30));
    assert(kindsOf(w.sockets[0]).filter(k => k === 12).length === 1, 'tune not repeated');
    const t = nonText(w.sockets[0]).find(b => new Uint8Array(b)[0] === 12);
    assert([...new Uint8Array(t)].join() === '12,50,40,50', 'default tune ' + [...new Uint8Array(t)]);
    w.page.setTune([80, 0, 30]);
    assert(await until(() => w.ble.writes.filter(b => b[0] === 12).length === 2), 'changed tune resent');
    w.page.sendTextOut('h\u200bi \u00e9');
    w.page.sendKeyOut(2, 4);
    assert(await until(() => kindsOf(w.sockets[0]).includes(13) && kindsOf(w.sockets[0]).includes(14)), 'text and key on wifi');
    const tx = nonText(w.sockets[0]).find(b => new Uint8Array(b)[0] === 13);
    assert(new TextDecoder().decode(new Uint8Array(tx).slice(1)) === 'hi \u00e9', 'zero-width stripped');
    assert(!w.ble.writes.some(b => b[0] === 13 || b[0] === 14), 'not duplicated on bluetooth');
    assert(w.store.pairedPC, 'paired PC remembered');
    assert(w.store.tune === '[80,0,30]', 'tune remembered');
  },
  async 'both links come up and both say hello'(w) {
    await w.page.start();
    assert(await until(() => w.page.wifi.up && w.page.ble.up), 'both up');
    assert(await until(() => kindsOf(w.sockets[0]).includes(1) && w.ble.writes.some(b => b[0] === 1)), 'hello on both');
    assert(w.ble.writes.some(b => b[0] === 9) && kindsOf(w.sockets[0]).includes(9), 'info on both');
  },
  async 'aim, click and pause go to every live link'(w) {
    await w.page.start(); await until(() => w.page.wifi.up && w.page.ble.up);
    w.sockets[0].status(1, 0, 'PC');
    w.page.setSamples({ ms: 1000, az: 1, el: 2 }, { ms: 1010, az: 1.1, el: 2.1 });
    w.page.aimAll();
    assert(await until(() => kindsOf(w.sockets[0]).some(k => k === 2 || k === 11) && w.ble.writes.some(b => b[0] === 2 || b[0] === 11)), 'aim on both');
    w.page.push(w.page.timed(w.page.K.press));
    assert(await until(() => kindsOf(w.sockets[0]).includes(3) && w.ble.writes.some(b => b[0] === 3)), 'press on both');
    const a = nonText(w.sockets[0]).find(b => new Uint8Array(b)[0] === 3), b = w.ble.writes.find(x => x[0] === 3);
    assert.deepStrictEqual([...new Uint8Array(a)], [...b], 'identical bytes (same timestamp) so the PC fires one click');
  },
  async 'scroll and trackpad deltas use the primary link only'(w) {
    await w.page.start(); await until(() => w.page.wifi.up && w.page.ble.up);
    w.page.mvAdd(5, 6); w.page.aimAll(); // pump pulls the delta on the primary
    await wait(30);
    assert(kindsOf(w.sockets[0]).includes(10), 'move on Wi-Fi');
    assert(!w.ble.writes.some(b => b[0] === 10), 'not on Bluetooth');
  },
  async 'Wi-Fi drops: Bluetooth keeps streaming, Wi-Fi comes back by itself'(w) {
    await w.page.start(); await until(() => w.page.wifi.up && w.page.ble.up);
    w.page.setSamples({ ms: 1000, az: 1, el: 2 }, { ms: 1010, az: 1.1, el: 2.1 });
    const first = w.sockets[0]; first.drop();
    assert(await until(() => !w.page.wifi.up, 200), 'wifi marked down at once');
    assert(w.page.ble.up && w.page.primary() === w.page.ble, 'ble is primary right away');
    const before = w.ble.writes.length; w.page.aimAll(); w.page.mvAdd(1, 1); w.page.aimAll();
    assert(await until(() => w.ble.writes.length > before), 'ble still carries aim');
    assert(w.ble.writes.slice(before).some(b => b[0] === 10), 'trackpad delta moved to Bluetooth');
    assert(await until(() => w.sockets.length === 2 && w.page.wifi.up, 4000), 'wifi reconnected on a new socket');
    assert(await until(() => kindsOf(w.sockets[1]).includes(1)), 'new socket says hello (PC replaces the dead one)');
  },
  async 'a silent Wi-Fi link is declared dead and replaced'(w) {
    w.echo = false;
    await w.page.start(); await until(() => w.page.wifi.up);
    assert(await until(() => !w.page.wifi.up, 1500), 'declared dead after DEAD_MS');
    assert(await until(() => w.sockets.length >= 2, 3000), 'reconnect attempted');
  },
  async 'Bluetooth drops and returns; Wi-Fi never notices'(w) {
    await w.page.start(); await until(() => w.page.wifi.up && w.page.ble.up);
    w.dev.gatt.disconnect();
    assert(await until(() => !w.page.ble.up));
    assert(w.page.wifi.up, 'wifi untouched');
    assert(await until(() => w.page.ble.up, 4000), 'ble reconnected');
  },
  async 'stop closes everything and nothing reconnects'(w) {
    await w.page.start(); await until(() => w.page.wifi.up && w.page.ble.up);
    w.page.stop();
    assert(!w.page.wifi.up && !w.page.ble.up);
    const n = w.sockets.length; await wait(800);
    assert.strictEqual(w.sockets.length, n, 'no new sockets after stop');
    assert(!w.ble.connected);
  },
};

(async () => {
  let fail = 0;
  for (const [name, fn] of Object.entries(tests)) {
    const w = world();
    try { await fn(w); console.log('ok   ' + name); }
    catch (e) { fail++; console.log('FAIL ' + name + '\n     ' + e.message); }
    try { w.page.stop(); } catch {}
  }
  process.exit(fail ? 1 : 0);
})();
