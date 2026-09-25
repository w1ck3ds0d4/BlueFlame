// Talks newline-delimited JSON to BlueFlame's control pipe: one handshake
// line with the session token, then one JSON request per line answered by
// exactly one JSON response line. See src-tauri/src/control/protocol.rs for
// the Rust side of this wire format.

import net from "node:net";
import fs from "node:fs/promises";
import path from "node:path";

export const PIPE_NAME = String.raw`\\.\pipe\blueflame-control`;

const APP_IDENTIFIER = "com.w1ck3ds0d4.blueflame";

/**
 * Where BlueFlame writes its per-session token, mirroring
 * `tauri::path().app_data_dir()` on Windows (`%APPDATA%\<identifier>`).
 * `BLUEFLAME_APP_DATA` overrides the app-data root, for a non-default
 * profile or for tests.
 */
export function defaultTokenPath(env = process.env) {
  const appData = env.BLUEFLAME_APP_DATA || env.APPDATA;
  if (!appData) {
    throw new Error(
      "APPDATA is not set and BLUEFLAME_APP_DATA was not provided; " +
        "this bridge only supports Windows, matching BlueFlame's phase 1 control channel",
    );
  }
  return path.join(appData, APP_IDENTIFIER, "control", "token");
}

export async function readToken(tokenPath) {
  const raw = await fs.readFile(tokenPath, "utf8");
  return raw.trim();
}

/**
 * A small client for the control pipe. `socketFactory` is overridable so
 * tests can exercise the framing and handshake logic against a fake duplex
 * stream instead of a real named pipe.
 */
export class ControlPipeClient {
  constructor({ token, pipeName = PIPE_NAME, socketFactory } = {}) {
    this.token = token;
    this.pipeName = pipeName;
    this.socketFactory = socketFactory ?? (() => net.createConnection({ path: this.pipeName }));
    this.socket = null;
    this.buffer = "";
    this.handshake = null;
    this.pending = new Map();
    this.nextId = 1;
    this.readyPromise = null;
  }

  connect() {
    if (!this.readyPromise) {
      this.readyPromise = this._connect();
    }
    return this.readyPromise;
  }

  async _connect() {
    const socket = this.socketFactory();
    this.socket = socket;

    await new Promise((resolve, reject) => {
      socket.once("connect", resolve);
      socket.once("error", reject);
    });

    if (typeof socket.setEncoding === "function") {
      socket.setEncoding("utf8");
    }
    socket.on("data", (chunk) => this._onData(chunk));
    socket.on("close", () => this._failAll(new Error("control pipe closed")));
    socket.on("error", (err) => this._failAll(err));

    const handshakeDone = new Promise((resolve, reject) => {
      this.handshake = { resolve, reject };
    });
    socket.write(JSON.stringify({ token: this.token }) + "\n");
    await handshakeDone;
  }

  _onData(chunk) {
    this.buffer += chunk;
    let idx;
    while ((idx = this.buffer.indexOf("\n")) !== -1) {
      const line = this.buffer.slice(0, idx);
      this.buffer = this.buffer.slice(idx + 1);
      if (line.trim().length > 0) {
        this._onLine(line);
      }
    }
  }

  _onLine(line) {
    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      return; // malformed line from the server; nothing sane to do but drop it
    }

    if (this.handshake) {
      const { resolve, reject } = this.handshake;
      this.handshake = null;
      if (msg.ok) {
        resolve(msg);
      } else {
        reject(new Error(msg.error || "handshake rejected"));
      }
      return;
    }

    const pending = this.pending.get(msg.id);
    if (!pending) {
      return; // response to a call we're no longer waiting on
    }
    this.pending.delete(msg.id);
    if (msg.ok) {
      pending.resolve(msg.result);
    } else {
      pending.reject(new Error(msg.error || "tool call failed"));
    }
  }

  _failAll(err) {
    if (this.handshake) {
      this.handshake.reject(err);
      this.handshake = null;
    }
    for (const { reject } of this.pending.values()) {
      reject(err);
    }
    this.pending.clear();
  }

  async call(tool, args = {}) {
    await this.connect();
    const id = this.nextId++;
    const result = new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
    this.socket.write(JSON.stringify({ id, tool, args }) + "\n");
    return result;
  }

  close() {
    this.socket?.end();
  }
}
