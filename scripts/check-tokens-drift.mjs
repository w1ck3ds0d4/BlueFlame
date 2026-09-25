#!/usr/bin/env node
// src/tokens.css is a verbatim copy of the WickIT design system's kit
// tokens (Design System/Kits/tokens.css in the WickIT-HQ repo). This
// script pins the hash it was copied at, so an edit to src/tokens.css
// that was not also made in the kit fails the build instead of quietly
// drifting the two apart.
//
// CI has no access to Daniel's local WickIT-HQ clone, so this cannot
// diff against the kit directly; it only catches a local edit to this
// file. To pull in a real update from the kit: copy the kit's
// tokens.css over src/tokens.css again, then run this script with
// --write to record the new hash.
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const root = process.cwd();
const tokensPath = path.join(root, "src", "tokens.css");
const hashPath = path.join(root, "src", "tokens.css.sha256");

async function main() {
  const write = process.argv.includes("--write");
  // Normalize CRLF to LF before hashing. A Windows checkout with
  // core.autocrlf=true keeps this file as CRLF on disk while git
  // stores it (and CI checks it out) as LF; hashing the raw bytes made
  // this check fail on CI for a file nobody had actually edited.
  const content = (await readFile(tokensPath, "utf8")).replace(/\r\n/g, "\n");
  const actual = createHash("sha256").update(content).digest("hex");

  if (write) {
    await writeFile(hashPath, `${actual}\n`);
    console.log(`check-tokens-drift: wrote ${hashPath}`);
    return;
  }

  let expected;
  try {
    expected = (await readFile(hashPath, "utf8")).trim();
  } catch (e) {
    console.error(`check-tokens-drift: could not read ${hashPath}: ${e.message}`);
    process.exit(1);
  }

  if (actual !== expected) {
    console.error("check-tokens-drift: src/tokens.css does not match its pinned hash.");
    console.error("It was edited without being re-copied from the WickIT design system kit");
    console.error("(Design System/Kits/tokens.css), or the pin is stale after a real kit update.");
    console.error("Copy the kit's tokens.css over src/tokens.css, then run:");
    console.error("  node scripts/check-tokens-drift.mjs --write");
    process.exit(1);
  }

  console.log("check-tokens-drift: ok, src/tokens.css matches its pinned hash");
}

main();
