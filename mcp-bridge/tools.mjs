// Tool definitions shared between the MCP server wiring (index.mjs) and its
// tests: names, JSON-schema-ish descriptions for MCP's tools/list, and how
// each one's arguments map onto a control-pipe request. Kept data-only (no
// pipe I/O) so the mapping itself is easy to unit test.

export const TOOLS = [
  {
    name: "list_tabs",
    description: "List the tabs Claude currently has open in BlueFlame.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
  },
  {
    name: "open_tab",
    description:
      "Open a new BlueFlame tab for Claude to drive. Always InPrivate, with none of Daniel's logins.",
    inputSchema: {
      type: "object",
      properties: { url: { type: "string", description: "URL to open, default about:blank" } },
      additionalProperties: false,
    },
  },
  {
    name: "navigate",
    description: "Navigate a Claude-owned tab to a URL.",
    inputSchema: {
      type: "object",
      properties: {
        tab_id: { type: "integer" },
        url: { type: "string" },
      },
      required: ["tab_id", "url"],
      additionalProperties: false,
    },
  },
  {
    name: "get_page_text",
    description: "Return the visible text of the page (document.body.innerText).",
    inputSchema: {
      type: "object",
      properties: { tab_id: { type: "integer" } },
      required: ["tab_id"],
      additionalProperties: false,
    },
  },
  {
    name: "read_page",
    description:
      "Return the page's accessibility tree: a flat list of {ref, role, name, value, childIds}. `ref` is what click/type/scroll take to target an element.",
    inputSchema: {
      type: "object",
      properties: { tab_id: { type: "integer" } },
      required: ["tab_id"],
      additionalProperties: false,
    },
  },
  {
    name: "find",
    description:
      "Search the last read_page tree for elements whose role, name or value contains `query` (case-insensitive).",
    inputSchema: {
      type: "object",
      properties: { tab_id: { type: "integer" }, query: { type: "string" } },
      required: ["tab_id", "query"],
      additionalProperties: false,
    },
  },
  {
    name: "click",
    description: "Click an element by ref (from read_page/find) or by viewport coordinates.",
    inputSchema: {
      type: "object",
      properties: {
        tab_id: { type: "integer" },
        ref: { type: "integer" },
        x: { type: "number" },
        y: { type: "number" },
      },
      required: ["tab_id"],
      additionalProperties: false,
    },
  },
  {
    name: "type",
    description: "Type text into an element (by ref) or whatever currently has focus.",
    inputSchema: {
      type: "object",
      properties: {
        tab_id: { type: "integer" },
        ref: { type: "integer" },
        text: { type: "string" },
      },
      required: ["tab_id", "text"],
      additionalProperties: false,
    },
  },
  {
    name: "key",
    description:
      "Press a named key (Enter, Tab, Escape, Backspace, Delete, ArrowUp/Down/Left/Right, Home, End, PageUp, PageDown, Space).",
    inputSchema: {
      type: "object",
      properties: { tab_id: { type: "integer" }, key: { type: "string" } },
      required: ["tab_id", "key"],
      additionalProperties: false,
    },
  },
  {
    name: "scroll",
    description: "Scroll the page by dx/dy, or scroll a specific element (by ref) into view.",
    inputSchema: {
      type: "object",
      properties: {
        tab_id: { type: "integer" },
        ref: { type: "integer" },
        dx: { type: "number" },
        dy: { type: "number" },
      },
      required: ["tab_id"],
      additionalProperties: false,
    },
  },
  {
    name: "screenshot",
    description: "Capture a PNG screenshot of the tab.",
    inputSchema: {
      type: "object",
      properties: { tab_id: { type: "integer" } },
      required: ["tab_id"],
      additionalProperties: false,
    },
  },
];

export const TOOL_NAMES = new Set(TOOLS.map((t) => t.name));

/** Turn a control-pipe tool result into an MCP `CallToolResult` content array. */
export function toContent(toolName, result) {
  if (toolName === "screenshot" && result && typeof result === "object" && result.data) {
    return [{ type: "image", data: result.data, mimeType: "image/png" }];
  }
  const text = typeof result === "string" ? result : JSON.stringify(result ?? null);
  return [{ type: "text", text }];
}
