#!/usr/bin/env node
// Stdio MCP server that bridges Claude Code to BlueFlame's local control
// pipe. Claude Code starts this process directly (see README.md "Adding
// the bridge to Claude Code"); it never listens on a network port itself,
// it only reads BlueFlame's token file and talks to the named pipe.

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { ListToolsRequestSchema, CallToolRequestSchema } from "@modelcontextprotocol/sdk/types.js";

import { ControlPipeClient, PIPE_NAME, defaultTokenPath, readToken } from "./pipe-client.mjs";
import { TOOLS, TOOL_NAMES, toContent } from "./tools.mjs";

async function main() {
  const tokenPath = defaultTokenPath();
  const token = await readToken(tokenPath).catch((err) => {
    throw new Error(
      `could not read BlueFlame's control token at ${tokenPath}: ${err.message}. ` +
        "Start BlueFlame first; it writes this file at launch.",
    );
  });

  // BLUEFLAME_PIPE_NAME lets a test point the bridge at a disposable pipe
  // instead of the one real BlueFlame instance uses. Daniel never sets
  // this himself; leaving it unset keeps the real, fixed pipe name.
  const pipeName = process.env.BLUEFLAME_PIPE_NAME || PIPE_NAME;
  const client = new ControlPipeClient({ token, pipeName });

  const server = new Server(
    { name: "blueflame", version: "0.1.0" },
    { capabilities: { tools: {} } },
  );

  server.setRequestHandler(ListToolsRequestSchema, async () => ({ tools: TOOLS }));

  server.setRequestHandler(CallToolRequestSchema, async (request) => {
    const { name, arguments: args } = request.params;
    if (!TOOL_NAMES.has(name)) {
      return { isError: true, content: [{ type: "text", text: `unknown tool: ${name}` }] };
    }
    try {
      const result = await client.call(name, args ?? {});
      return { content: toContent(name, result) };
    } catch (err) {
      return { isError: true, content: [{ type: "text", text: err.message }] };
    }
  });

  const transport = new StdioServerTransport();
  await server.connect(transport);
}

main().catch((err) => {
  console.error(`blueflame-mcp-bridge: ${err.message}`);
  process.exit(1);
});
