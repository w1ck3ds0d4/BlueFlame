#!/usr/bin/env node
// Runs automatically after `pnpm build` (npm/pnpm "postbuild" hook), which
// is also the build CI runs. Fails the build if anything from the
// design-preview mode leaked into the production bundle in dist/.
//
// This is the assertion required for src/design-main.tsx, src/design/*
// and design.html: development-only, never shipped.
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";

const DIST = path.join(process.cwd(), "dist");
const FORBIDDEN_MARKERS = [
  "design-main",
  "installDesignMocks",
  "DesignSwitcher",
  "@tauri-apps/api/mocks",
  "mockIPC",
  "mockWindows",
];

async function walk(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await walk(full)));
    } else {
      files.push(full);
    }
  }
  return files;
}

async function main() {
  let files;
  try {
    files = await walk(DIST);
  } catch (e) {
    console.error(`check-no-design-in-build: could not read ${DIST}: ${e.message}`);
    process.exit(1);
  }

  if (files.some((f) => path.basename(f) === "design.html")) {
    console.error("check-no-design-in-build: design.html was emitted into dist/");
    process.exit(1);
  }

  const textFiles = files.filter((f) => /\.(js|mjs|html|css)$/.test(f));
  const offenders = [];

  for (const file of textFiles) {
    const content = await readFile(file, "utf8");
    for (const marker of FORBIDDEN_MARKERS) {
      if (content.includes(marker)) {
        offenders.push({ file: path.relative(process.cwd(), file), marker });
      }
    }
  }

  if (offenders.length > 0) {
    console.error("check-no-design-in-build: design preview code leaked into dist/");
    for (const o of offenders) {
      console.error(`  ${o.file} contains "${o.marker}"`);
    }
    process.exit(1);
  }

  console.log(`check-no-design-in-build: ok, scanned ${textFiles.length} files in dist/`);
}

main();
