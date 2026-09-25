import { test } from "node:test";
import assert from "node:assert/strict";

import { TOOLS, TOOL_NAMES, toContent } from "../tools.mjs";

test("every tool has a name, description and object input schema", () => {
  for (const tool of TOOLS) {
    assert.equal(typeof tool.name, "string");
    assert.equal(typeof tool.description, "string");
    assert.equal(tool.inputSchema.type, "object");
  }
});

test("TOOL_NAMES matches the tool list exactly", () => {
  assert.deepEqual([...TOOL_NAMES].sort(), TOOLS.map((t) => t.name).sort());
});

test("toContent wraps a screenshot result as an MCP image block", () => {
  const content = toContent("screenshot", { format: "png", data: "QUJD" });
  assert.deepEqual(content, [{ type: "image", data: "QUJD", mimeType: "image/png" }]);
});

test("toContent wraps other results as text", () => {
  const content = toContent("list_tabs", { tabs: [{ tab_id: 1 }] });
  assert.deepEqual(content, [{ type: "text", text: JSON.stringify({ tabs: [{ tab_id: 1 }] }) }]);
});

test("toContent passes a plain string straight through", () => {
  const content = toContent("get_page_text", "hello world");
  assert.deepEqual(content, [{ type: "text", text: "hello world" }]);
});
