// Serves the page the browser panel loads and collects what probe.js reports.
// Prints one JSON line per report and exits when the probe says it is done.
const http = require("http");

const seen = new Set();
const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://127.0.0.1");
  res.setHeader("Access-Control-Allow-Origin", "*");
  if (url.pathname === "/frame") {
    res.setHeader("Content-Type", "text/html");
    return res.end(`<!doctype html><title>probe</title><h1>pinned probe</h1><script>fetch("/hit?what=frame-ran")</script>`);
  }
  if (url.pathname === "/opened") {
    seen.add("opened");
    res.setHeader("Content-Type", "text/html");
    return res.end(`<!doctype html><title>pinned probe</title><p>pinned's native probe opened this page. You can close this tab.</p>`);
  }
  if (url.pathname === "/hit") {
    seen.add(url.searchParams.get("what"));
    return res.end("ok");
  }
  if (url.pathname === "/seen") return res.end(JSON.stringify(seen.has(url.searchParams.get("what"))));
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    res.end("ok");
    if (url.pathname === "/report") console.log(body);
    if (url.pathname === "/done") server.close(() => process.exit(0));
  });
});
server.listen(Number(process.argv[2]), "127.0.0.1");
