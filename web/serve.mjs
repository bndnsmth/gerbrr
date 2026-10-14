import { createServer } from "node:http";
import { createReadStream } from "node:fs";
import { stat, realpath } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { extname, resolve, relative, isAbsolute } from "node:path";

const root = await realpath(fileURLToPath(new URL("./", import.meta.url)));
const port = Number(process.env.PORT || 4173);
if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error("Invalid PORT");
const mime = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".wasm": "application/wasm", ".svg": "image/svg+xml" };
function outside(path) { const rel = relative(root, path); return rel.startsWith("..") || isAbsolute(rel); }
const server = createServer(async (req, res) => {
  if (!["GET", "HEAD"].includes(req.method)) { res.writeHead(405, { Allow: "GET, HEAD" }); res.end("Static server: no upload endpoints"); return; }
  try {
    const pathname = decodeURIComponent(new URL(req.url, "http://localhost").pathname);
    const file = resolve(root, `.${pathname === "/" ? "/index.html" : pathname}`);
    if (outside(file)) throw new Error("Forbidden path");
    const canonical = await realpath(file);
    if (outside(canonical)) throw new Error("Forbidden path");
    const info = await stat(canonical);
    if (!info.isFile()) throw new Error("Not a file");
    res.writeHead(200, { "Content-Type": mime[extname(file)] || "application/octet-stream", "Content-Length": info.size,
      "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff", "Referrer-Policy": "no-referrer" });
    if (req.method === "HEAD") res.end();
    else createReadStream(canonical).on("error", () => res.destroy()).pipe(res);
  } catch { res.writeHead(404, { "Content-Type": "text/plain" }); res.end("Not found. Run mise run web-build first if generated assets are missing."); }
});
server.listen(port, "127.0.0.1", () => console.log(`Gerbrr — Gerber viewing at ridiculous speed.\nhttp://localhost:${port}\nLocal-only static server. Gerber files stay in the browser.`));
