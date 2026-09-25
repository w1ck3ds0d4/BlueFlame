import { test } from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";

import { ControlPipeClient } from "../pipe-client.mjs";

/**
 * A fake duplex "socket": an EventEmitter with a `write()` that records
 * every line the client sends, so tests can script the server's replies by
 * calling `fake.emit("data", ...)` directly instead of standing up a real
 * named pipe.
 */
function fakeSocket() {
  const socket = new EventEmitter();
  socket.written = [];
  socket.setEncoding = () => {};
  socket.write = (data) => {
    socket.written.push(data);
    return true;
  };
  socket.end = () => {};
  return socket;
}

/** Flush the microtask queue: lets `connect()`'s internal `await`s catch up
 * with an event we just emitted synchronously, before the test emits the
 * next one. A macrotask boundary (`setImmediate`) flushes every pending
 * microtask regardless of how many hops are in between, so this is not
 * sensitive to `pipe-client.mjs`'s internal `await` count. */
function flush() {
  return new Promise((resolve) => setImmediate(resolve));
}

/** Connects `client` against `socket` and completes the handshake with an
 * `{ok: true}` reply, returning once `connect()` has resolved. */
async function connectAndHandshake(client, socket) {
  const connecting = client.connect();
  socket.emit("connect");
  await flush();
  socket.emit("data", JSON.stringify({ ok: true, result: "ready" }) + "\n");
  await connecting;
}

test("sends the token as the first line and resolves on an ok handshake", async () => {
  const socket = fakeSocket();
  const client = new ControlPipeClient({ token: "abc123", socketFactory: () => socket });

  await connectAndHandshake(client, socket);

  assert.equal(socket.written.length, 1);
  assert.deepEqual(JSON.parse(socket.written[0]), { token: "abc123" });
});

test("rejects connect() when the handshake is refused", async () => {
  const socket = fakeSocket();
  const client = new ControlPipeClient({ token: "wrong", socketFactory: () => socket });

  const connecting = client.connect();
  socket.emit("connect");
  await flush();
  socket.emit("data", JSON.stringify({ ok: false, error: "invalid token" }) + "\n");

  await assert.rejects(connecting, /invalid token/);
});

test("call() matches a response to its request id and returns the result", async () => {
  const socket = fakeSocket();
  const client = new ControlPipeClient({ token: "abc123", socketFactory: () => socket });
  await connectAndHandshake(client, socket);

  const callPromise = client.call("get_page_text", { tab_id: 1 });
  await flush();
  const sent = JSON.parse(socket.written.at(-1));
  assert.equal(sent.tool, "get_page_text");
  assert.deepEqual(sent.args, { tab_id: 1 });

  socket.emit("data", JSON.stringify({ id: sent.id, ok: true, result: "hello" }) + "\n");
  assert.equal(await callPromise, "hello");
});

test("call() rejects when the server reports a tool error", async () => {
  const socket = fakeSocket();
  const client = new ControlPipeClient({ token: "abc123", socketFactory: () => socket });
  await connectAndHandshake(client, socket);

  const callPromise = client.call("navigate", { tab_id: 9, url: "https://example.com" });
  await flush();
  const sent = JSON.parse(socket.written.at(-1));
  socket.emit(
    "data",
    JSON.stringify({ id: sent.id, ok: false, error: "tab 9 is not a Claude-controlled tab" }) +
      "\n",
  );

  await assert.rejects(callPromise, /not a Claude-controlled tab/);
});

test("handles two requests split across multiple data chunks", async () => {
  const socket = fakeSocket();
  const client = new ControlPipeClient({ token: "abc123", socketFactory: () => socket });
  await connectAndHandshake(client, socket);

  const a = client.call("list_tabs");
  const b = client.call("list_tabs");
  await flush();
  const idA = JSON.parse(socket.written.at(-2)).id;
  const idB = JSON.parse(socket.written.at(-1)).id;

  const line1 = JSON.stringify({ id: idA, ok: true, result: { tabs: [] } });
  const line2 = JSON.stringify({ id: idB, ok: true, result: { tabs: [1] } });
  // Split the two response lines across arbitrary chunk boundaries.
  socket.emit("data", line1.slice(0, 5));
  socket.emit("data", line1.slice(5) + "\n" + line2.slice(0, 3));
  socket.emit("data", line2.slice(3) + "\n");

  assert.deepEqual(await a, { tabs: [] });
  assert.deepEqual(await b, { tabs: [1] });
});

test("a closed pipe fails every pending call", async () => {
  const socket = fakeSocket();
  const client = new ControlPipeClient({ token: "abc123", socketFactory: () => socket });
  await connectAndHandshake(client, socket);

  const pending = client.call("screenshot", { tab_id: 1 });
  await flush();
  socket.emit("close");
  await assert.rejects(pending, /closed/);
});
