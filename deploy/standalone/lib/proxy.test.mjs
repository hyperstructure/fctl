import assert from "node:assert/strict";
import http from "node:http";
import net from "node:net";
import { after, before, describe, test } from "node:test";
import { clientIp, createProxy, downstreamHeaders, target, upstreamHeaders } from "./proxy.mjs";

describe("clientIp", () => {
  test("is the connection's address without proxies to trust", () => {
    assert.equal(clientIp("203.0.113.9", "198.51.100.1", 0), "203.0.113.9");
    assert.equal(clientIp("::ffff:203.0.113.9", undefined, 0), "203.0.113.9");
  });

  test("is what the outermost trusted proxy saw", () => {
    // The client sent its own X-Forwarded-For; the one proxy appended what it saw.
    assert.equal(clientIp("10.0.0.2", "6.6.6.6, 198.51.100.7", 1), "198.51.100.7");
    assert.equal(clientIp("10.0.0.3", "6.6.6.6, 198.51.100.7, 10.0.0.2", 2), "198.51.100.7");
  });

  test("stops at the chain's start when fewer proxies were there", () => {
    assert.equal(clientIp("10.0.0.2", undefined, 1), "10.0.0.2");
    assert.equal(clientIp("10.0.0.2", "198.51.100.7", 3), "198.51.100.7");
  });

  test("ignores what isn't an address", () => {
    assert.equal(clientIp("10.0.0.2", "not-an-ip", 1), "10.0.0.2");
  });

  test("takes addresses with ports", () => {
    assert.equal(clientIp("10.0.0.2", "198.51.100.7:51234", 1), "198.51.100.7");
    assert.equal(clientIp("10.0.0.2", "[2001:db8::7]:51234", 1), "2001:db8::7");
  });
});

test("requests go on in origin-form, except to the edge's paths", () => {
  assert.equal(target("/api/v1/sessions?after=3"), "/api/v1/sessions?after=3");
  assert.equal(target("http://elsewhere.example/api/v1/health"), "/api/v1/health");
  assert.equal(target("/cdn-cgi-docs"), "/cdn-cgi-docs");
  assert.equal(target("/cdn-cgi/handler/scheduled?cron=1"), undefined);
  assert.equal(target("/cdn-cgi"), undefined);
  assert.equal(target("/api/../cdn-cgi/handler/scheduled"), undefined);
  assert.equal(target("/%63dn-cgi/handler/scheduled"), undefined);
  assert.equal(target("/CDN-CGI/trace"), undefined);
  assert.equal(target("/%E0%A4%A"), undefined);
  assert.equal(target("http://[bad"), undefined);
});

test("upstream headers drop what clients mustn't choose", () => {
  const out = upstreamHeaders(
    [
      "Host",
      "fctl.example.com",
      "CF-Connecting-IP",
      "6.6.6.6",
      "MF-Original-URL",
      "http://evil/",
      "X-Forwarded-Proto",
      "https",
      "Connection",
      "keep-alive",
      "Cookie",
      "a=1",
    ],
    "198.51.100.7",
  );
  assert.deepEqual(out, [
    "Host",
    "fctl.example.com",
    "Cookie",
    "a=1",
    "CF-Connecting-IP",
    "198.51.100.7",
    "MF-Disable-Pretty-Error",
    "true",
  ]);
  const upgrade = upstreamHeaders(["Connection", "Upgrade", "Upgrade", "websocket"], "::1", {
    upgrade: true,
  });
  assert.deepEqual(upgrade.slice(0, 4), ["Connection", "Upgrade", "Upgrade", "websocket"]);
});

test("downstream headers keep every Set-Cookie and drop Miniflare's", () => {
  const out = downstreamHeaders([
    "Set-Cookie",
    "a=1",
    "Set-Cookie",
    "b=2",
    "MF-Experimental-Error-Stack",
    "…",
    "Transfer-Encoding",
    "chunked",
  ]);
  assert.deepEqual(out, ["Set-Cookie", "a=1", "Set-Cookie", "b=2"]);
});

describe("the proxy", () => {
  let upstream;
  let proxy;
  let base;
  const seen = [];

  before(async () => {
    upstream = http.createServer((req, res) => {
      seen.push(req.headers);
      res.setHeader("Set-Cookie", ["a=1", "b=2"]);
      res.setHeader("MF-Experimental-Error-Stack", "stack");
      res.end(`${req.method} ${req.url}`);
    });
    upstream.on("upgrade", (req, socket) => {
      seen.push(req.headers);
      socket.write("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n");
      socket.on("data", (d) => socket.write(d));
    });
    await new Promise((r) => upstream.listen(0, "127.0.0.1", r));
    proxy = createProxy({ upstream: { host: "127.0.0.1", port: upstream.address().port }, trustProxy: 0 });
    await new Promise((r) => proxy.listen(0, "127.0.0.1", r));
    base = `http://127.0.0.1:${proxy.address().port}`;
  });

  after(() => {
    proxy.closeAll();
    upstream.close();
    upstream.closeAllConnections();
  });

  test("passes requests on with the client's address", async () => {
    const res = await fetch(`${base}/api/v1/health?x=1`, { headers: { "CF-Connecting-IP": "6.6.6.6" } });
    assert.equal(await res.text(), "GET /api/v1/health?x=1");
    assert.deepEqual(res.headers.getSetCookie(), ["a=1", "b=2"]);
    assert.equal(res.headers.get("mf-experimental-error-stack"), null);
    const headers = seen.at(-1);
    assert.equal(headers["cf-connecting-ip"], "127.0.0.1");
    assert.equal(headers["mf-disable-pretty-error"], "true");
  });

  test("answers /cdn-cgi/ itself", async () => {
    const before = seen.length;
    const res = await fetch(`${base}/cdn-cgi/handler/scheduled`);
    assert.equal(res.status, 404);
    assert.equal(seen.length, before);
  });

  test("streams request bodies", async () => {
    const body = "x".repeat(1 << 20);
    const res = await fetch(`${base}/api/v1/upload`, { method: "POST", body });
    assert.equal(await res.text(), "POST /api/v1/upload");
  });

  test("carries WebSocket upgrades both ways", async () => {
    const socket = net.connect(proxy.address().port, "127.0.0.1");
    await new Promise((r) => socket.once("connect", r));
    socket.write(
      "GET /api/v1/feed/ws HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nCF-Connecting-IP: 6.6.6.6\r\n\r\n",
    );
    let text = "";
    socket.on("data", (d) => {
      text += d;
    });
    await new Promise((r) => setTimeout(r, 100));
    socket.write("ping");
    await new Promise((r) => setTimeout(r, 100));
    socket.destroy();
    assert.match(text, /^HTTP\/1\.1 101 /);
    assert.ok(text.endsWith("ping"));
    assert.equal(seen.at(-1)["cf-connecting-ip"], "127.0.0.1");
    assert.equal(seen.at(-1).upgrade, "websocket");
  });
});

describe("a handshake the control plane doesn't take", () => {
  let upstream;
  let proxy;
  // Request lines in the order the control plane read them, from every connection.
  const requests = [];

  before(async () => {
    // Reads requests one after another on each connection and answers each
    // as a request, keeping the connection open, as a server may after it
    // refuses an upgrade.
    upstream = net.createServer((socket) => {
      let buffered = "";
      socket.on("data", (d) => {
        buffered += d;
        let end = buffered.indexOf("\r\n\r\n");
        while (end !== -1) {
          requests.push(buffered.slice(0, end).split("\r\n"));
          buffered = buffered.slice(end + 4);
          socket.write("HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nno");
          end = buffered.indexOf("\r\n\r\n");
        }
      });
      socket.on("error", () => {});
    });
    await new Promise((r) => upstream.listen(0, "127.0.0.1", r));
    proxy = createProxy({ upstream: { host: "127.0.0.1", port: upstream.address().port }, trustProxy: 0 });
    await new Promise((r) => proxy.listen(0, "127.0.0.1", r));
  });

  after(() => {
    proxy.closeAll();
    upstream.close();
  });

  /** Sends `raw` to the proxy and returns what came back by the time the connection closed (or 500 ms passed). */
  async function exchange(raw) {
    const socket = net.connect(proxy.address().port, "127.0.0.1");
    await new Promise((r) => socket.once("connect", r));
    let text = "";
    socket.on("data", (d) => {
      text += d;
    });
    const closed = new Promise((r) => socket.once("close", () => r(true)));
    socket.write(raw);
    const ended = await Promise.race([closed, new Promise((r) => setTimeout(() => r(false), 500))]);
    socket.destroy();
    return { text, ended };
  }

  test("answers it and ends the connection, passing on nothing that follows", async () => {
    requests.length = 0;
    const { text, ended } = await exchange(
      "GET /api/v1/sessions/ses_1/ws HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n" +
        "GET /cdn-cgi/handler/scheduled HTTP/1.1\r\nHost: localhost\r\nCF-Connecting-IP: 6.6.6.6\r\nMF-Route-Override: x\r\n\r\n",
    );
    assert.match(text, /^HTTP\/1\.1 401 Unauthorized\r\n/);
    assert.match(text, /\r\nConnection: close\r\n/i);
    assert.ok(text.endsWith("\r\n\r\nno"));
    assert.ok(ended, "the connection ends with the answer");
    assert.equal(requests.length, 1);
    assert.equal(requests[0][0], "GET /api/v1/sessions/ses_1/ws HTTP/1.1");
    assert.ok(requests[0].includes("CF-Connecting-IP: 127.0.0.1"));
    assert.ok(!requests.flat().some((line) => /6\.6\.6\.6|MF-Route-Override|cdn-cgi/i.test(line)));
  });

  test("takes only a GET without a body", async () => {
    requests.length = 0;
    const { text, ended } = await exchange(
      "POST /api/v1/health HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nContent-Length: 0\r\n\r\n",
    );
    assert.match(text, /^HTTP\/1\.1 400 /);
    assert.ok(ended);
    assert.equal(requests.length, 0);
  });
});
