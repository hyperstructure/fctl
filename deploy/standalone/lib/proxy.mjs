// The server's front door. Browsers, the CLI and hosts (or the reverse proxy
// in front of them) connect here, and each request goes on to Miniflare on a
// loopback port. On Cloudflare the edge sets CF-Connecting-IP, which the
// Worker's rate limits and audit log go by, and answers /cdn-cgi/ itself;
// here this does both. Miniflare would keep a CF-Connecting-IP the client
// sent, and it takes headers that steer it (MF-*) from anyone.

import http from "node:http";
import net from "node:net";

/** Request headers a client must not choose. `x-forwarded-proto` makes Miniflare rewrite the URL. */
const DROPPED = /^(cf-|mf-|x-forwarded-proto$|x-forwarded-host$)/i;
/** Hop-by-hop headers (RFC 9110, 7.6.1), for requests that don't upgrade, and `expect`, which this server answered. */
const HOP_BY_HOP = new Set([
  "connection",
  "expect",
  "keep-alive",
  "proxy-connection",
  "proxy-authenticate",
  "proxy-authorization",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
]);

/** `::ffff:192.0.2.1`, `192.0.2.1:443` → `192.0.2.1`; `[2001:db8::1]:443` → `2001:db8::1`. */
function plain(ip) {
  const bare = /^\[([^\]]+)\](?::\d+)?$/.exec(ip ?? "")?.[1] ?? /^([\d.]+):\d+$/.exec(ip ?? "")?.[1] ?? ip;
  return bare?.startsWith("::ffff:") && net.isIPv4(bare.slice(7)) ? bare.slice(7) : bare;
}

/**
 * The client's address: the connection's, or with `trustedHops` proxies in
 * front, the address the outermost of them saw, from X-Forwarded-For (each
 * proxy appends the address it heard from).
 */
export function clientIp(remoteAddress, forwardedFor, trustedHops) {
  const chain = [];
  if (trustedHops > 0 && forwardedFor) {
    for (const part of String(forwardedFor).split(",")) {
      const ip = plain(part.trim());
      if (ip) chain.push(ip);
    }
  }
  chain.push(plain(remoteAddress) ?? "");
  const ip = chain[Math.max(0, chain.length - 1 - trustedHops)];
  return net.isIP(ip) ? ip : plain(remoteAddress);
}

/**
 * Where a request goes on: its path and query (origin-form, whatever form
 * the client sent), or `undefined` for one this server answers itself with
 * a 404: what isn't a URL, and /cdn-cgi/, which the edge answers on
 * Cloudflare and where Miniflare serves its own internals.
 */
export function target(url) {
  let parsed;
  try {
    parsed = new URL(url, "http://x");
  } catch {
    return undefined;
  }
  const { pathname, search } = parsed;
  if (pathname === "/cdn-cgi" || pathname.startsWith("/cdn-cgi/")) return undefined;
  return pathname + search;
}

/**
 * The raw header list (`[name, value, …]`) to send on: without what clients
 * must not set, with the client's address and Miniflare's plain error
 * responses. Upgrades keep Connection and Upgrade.
 */
export function upstreamHeaders(rawHeaders, ip, { upgrade = false } = {}) {
  const out = [];
  for (let i = 0; i + 1 < rawHeaders.length; i += 2) {
    const name = rawHeaders[i];
    const lower = name.toLowerCase();
    if (DROPPED.test(lower)) continue;
    if (!upgrade && HOP_BY_HOP.has(lower)) continue;
    out.push(name, rawHeaders[i + 1]);
  }
  out.push("CF-Connecting-IP", ip);
  // A Worker that throws gets a plain 500, not a page with its stack trace.
  out.push("MF-Disable-Pretty-Error", "true");
  return out;
}

/** Response headers to pass back: Miniflare's own (an error's stack, MF-*) stay here. */
export function downstreamHeaders(rawHeaders) {
  const out = [];
  for (let i = 0; i + 1 < rawHeaders.length; i += 2) {
    const lower = rawHeaders[i].toLowerCase();
    if (lower.startsWith("mf-") || HOP_BY_HOP.has(lower)) continue;
    out.push(rawHeaders[i], rawHeaders[i + 1]);
  }
  return out;
}

function headerBlock(raw) {
  let s = "";
  for (let i = 0; i + 1 < raw.length; i += 2) s += `${raw[i]}: ${raw[i + 1]}\r\n`;
  return s;
}

/**
 * An HTTP server that hands every request to `upstream` (`{ host, port }`),
 * WebSocket upgrades included. `trustProxy`: how many reverse proxies are in
 * front (`clientIp`).
 */
export function createProxy({ upstream, trustProxy, onError = () => {} }) {
  const sockets = new Set();
  const server = http.createServer({
    // Attachments are up to 20 MB, which takes a while on a slow uplink.
    requestTimeout: 15 * 60_000,
    // Longer than the idle timeouts of the load balancers in front.
    keepAliveTimeout: 65_000,
  });
  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
  });

  server.on("request", (req, res) => {
    const path = target(req.url);
    if (path === undefined) {
      res.writeHead(404, { "content-type": "text/plain" }).end("Not found\n");
      return;
    }
    const ip = clientIp(req.socket.remoteAddress, req.headers["x-forwarded-for"], trustProxy);
    const up = http.request({
      host: upstream.host,
      port: upstream.port,
      method: req.method,
      path,
      headers: upstreamHeaders(req.rawHeaders, ip),
    });
    up.on("response", (r) => {
      res.writeHead(r.statusCode ?? 502, r.statusMessage, downstreamHeaders(r.rawHeaders));
      r.pipe(res);
      r.on("error", () => res.destroy());
    });
    up.on("error", (e) => {
      if (!res.headersSent) {
        res.writeHead(502, { "content-type": "text/plain" }).end("The control plane isn't answering\n");
        onError(e);
      } else {
        res.destroy();
      }
    });
    res.on("close", () => {
      if (!res.writableFinished) up.destroy();
    });
    req.pipe(up);
  });

  server.on("upgrade", (req, socket, head) => {
    socket.on("error", () => socket.destroy());
    const path = target(req.url);
    if (path === undefined) {
      socket.end("HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
      return;
    }
    const ip = clientIp(req.socket.remoteAddress, req.headers["x-forwarded-for"], trustProxy);
    const up = net.connect(upstream.port, upstream.host);
    up.on("connect", () => {
      const headers = upstreamHeaders(req.rawHeaders, ip, { upgrade: true });
      up.write(`${req.method} ${path} HTTP/1.1\r\n${headerBlock(headers)}\r\n`);
      if (head.length > 0) up.write(head);
      socket.setNoDelay(true);
      up.setNoDelay(true);
      socket.pipe(up).pipe(socket);
    });
    up.on("error", (e) => {
      if (!socket.destroyed && up.bytesRead === 0) {
        socket.end("HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
        onError(e);
      } else {
        socket.destroy();
      }
    });
    up.on("close", () => socket.destroy());
    socket.on("close", () => up.destroy());
  });

  /** Stops listening and drops every open connection (hosts and viewers reconnect). */
  server.closeAll = () => {
    server.close();
    for (const s of sockets) s.destroy();
  };
  return server;
}
