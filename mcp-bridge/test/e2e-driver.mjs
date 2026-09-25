#!/usr/bin/env node
// Stands in for Claude Code in the Rust-side end-to-end test
// (src-tauri/src/control/windows_impl/e2e_test.rs): spawns the real
// mcp-bridge (index.mjs, unmodified) over stdio using the official MCP
// client SDK - the same Client/StdioClientTransport pairing Claude Code
// itself uses to talk to a stdio MCP server - drives the tool calls given
// as a JSON array on argv[2], and prints the results as JSON on stdout.
//
// Not part of the published bridge: only used from that Rust test, never
// by Claude Code or by Daniel.

import path from "node:path";
import { fileURLToPath } from "node:url";

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const bridgePath = path.join(here, "..", "index.mjs");

async function main() {
  const calls = JSON.parse(process.argv[2] ?? "[]");

  const transport = new StdioClientTransport({
    command: process.execPath,
    args: [bridgePath],
    env: process.env,
    stderr: "inherit",
  });
  const client = new Client({ name: "blueflame-e2e-test-driver", version: "0.0.0" });
  await client.connect(transport);

  const tools = (await client.listTools()).tools.map((t) => t.name);

  const results = [];
  for (const call of calls) {
    try {
      const res = await client.callTool({ name: call.name, arguments: call.args ?? {} });
      results.push({ name: call.name, ok: !res.isError, content: res.content });
    } catch (err) {
      results.push({ name: call.name, ok: false, error: err.message });
    }
  }

  await client.close();
  process.stdout.write(JSON.stringify({ tools, results }));
}

main().catch((err) => {
  console.error(`e2e-driver: ${err.message}`);
  process.exit(1);
});
