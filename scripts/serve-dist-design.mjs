#!/usr/bin/env node
// Minimal static file server for dist-design/, used by scripts/shots.ps1.
// Not part of the production app; only ever run as a helper for `pnpm shots`.
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const DIST_DESIGN = path.join(ROOT, "dist-design");
const PORT = Number(process.env.SHOTS_PORT ?? 4173);

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".json": "application/json",
  ".woff2": "font/woff2",
};

const server = createServer(async (req, res) => {
  try {
    let urlPath = decodeURIComponent(req.url.split("?")[0]);
    if (urlPath === "/") urlPath = "/design.html";
    const filePath = path.join(DIST_DESIGN, urlPath);
    if (!filePath.startsWith(DIST_DESIGN)) {
      res.writeHead(403);
      res.end();
      return;
    }
    const body = await readFile(filePath);
    const ext = path.extname(filePath);
    res.writeHead(200, { "Content-Type": MIME[ext] ?? "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404);
    res.end("not found");
  }
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`serving dist-design on http://127.0.0.1:${PORT}`);
});
