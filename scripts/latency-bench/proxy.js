#!/usr/bin/env node

const net = require("net");
const fs = require("fs");

const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const i = args.indexOf(name);
  if (i < 0) return fallback;
  const value = args[i + 1];
  if (value === undefined) {
    console.error(`missing value for ${name}`);
    process.exit(2);
  }
  return value;
};

const listen = flag("--listen", "127.0.0.1:8792");
const upstream = flag("--upstream", "127.0.0.1:8791");
const rtt = Number(flag("--rtt-ms", "150"));
const statsPath = flag("--stats", "");
if (!Number.isFinite(rtt) || rtt < 0) {
  console.error(`bad --rtt-ms ${rtt}`);
  process.exit(2);
}
const [listenHost, listenPort] = split(listen);
const [upHost, upPort] = split(upstream);
const delay = rtt / 2;

let transferred = 0;
let connections = 0;
const clients = new Set();

function split(addr) {
  const at = addr.lastIndexOf(":");
  return [addr.slice(0, at), Number(addr.slice(at + 1))];
}

function later(sock, chunk) {
  setTimeout(() => {
    if (sock.destroyed) return;
    if (chunk === null) sock.end();
    else sock.write(chunk);
  }, delay);
}

function arm(from, to) {
  const queued = [];
  let open = false;
  from.on("data", (chunk) => {
    transferred += chunk.length;
    if (!open) queued.push(chunk);
    else later(to, chunk);
  });
  from.on("end", () => {
    if (!open) queued.push(null);
    else later(to, null);
  });
  from.on("error", () => to.destroy());
  return {
    ready() {
      open = true;
      for (const chunk of queued) later(to, chunk);
      queued.length = 0;
    },
  };
}

function writeStats() {
  if (!statsPath) return;
  const body = JSON.stringify({
    transferred,
    connections,
    rtt_ms: rtt,
    delay_ms: delay,
  });
  fs.writeFileSync(statsPath, `${body}\n`);
}

const server = net.createServer((client) => {
  connections += 1;
  clients.add(client);
  client.on("close", () => clients.delete(client));
  client.setNoDelay(true);
  const remote = net.connect({ host: upHost, port: upPort });
  const toRemote = arm(client, remote);
  const toClient = arm(remote, client);
  remote.on("connect", () => {
    remote.setNoDelay(true);
    toRemote.ready();
    toClient.ready();
  });
  remote.on("error", () => client.destroy());
  client.on("error", () => remote.destroy());
});

server.on("error", (err) => {
  console.error(err.message);
  process.exit(1);
});
server.listen(listenPort, listenHost, () => {
  console.error(`proxy ${listen} -> ${upstream} rtt ${rtt}ms`);
});
setInterval(writeStats, 500);
process.on("SIGUSR1", () => {
  console.error(`closing ${clients.size} client sockets`);
  for (const sock of clients) sock.destroy();
});
process.on("SIGTERM", () => {
  writeStats();
  process.exit(0);
});
process.on("SIGINT", () => {
  writeStats();
  process.exit(0);
});
