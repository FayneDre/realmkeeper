#!/usr/bin/env node
// Computes the "full build" plan of a flavor from the catalog: the core source
// and every catalog module compatible with it (ADR 0003). Writes the plan as
// JSON and, on GitHub Actions, exposes key values as step outputs.
//
// Usage: node tools/ci/build-plan.mjs <emulator> <flavor> <out.json> [platform]
// Platform: windows-x86_64 (default) or linux-x86_64 (ADR 0031).

import { readFileSync, readdirSync, writeFileSync, appendFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const [emulator, flavorId, out, platform = "windows-x86_64"] = process.argv.slice(2);
// Each platform is built by its own job; its key hashes that job (up to the
// job after it) and the scripts it runs.
const recipes = {
  "windows-x86_64": { job: ["\n  build:\n", "\n  smoke-test:\n"], scripts: ["fetch-mysql.ps1"] },
  "linux-x86_64": { job: ["\n  build-linux:\n", "\n  smoke-test-linux:\n"], scripts: ["fetch-mysql.sh", "bundle-libs.sh"] },
};
if (!emulator || !flavorId || !out || !recipes[platform]) {
  console.error(`usage: build-plan.mjs <emulator> <flavor> <out.json> [${Object.keys(recipes).join("|")}]`);
  process.exit(2);
}

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..");
const emulatorDir = join(root, "catalog", "emulators", emulator);
const read = (path) => JSON.parse(readFileSync(path, "utf8"));
const text = (path) => readFileSync(path, "utf8").replace(/\r\n/g, "\n");
const sha256 = (value) => createHash("sha256").update(value).digest("hex");

const flavor = read(join(emulatorDir, "flavors", `${flavorId}.json`));
const modules = readdirSync(join(emulatorDir, "modules"))
  .filter((f) => f.endsWith(".json"))
  .sort()
  .map((f) => read(join(emulatorDir, "modules", f)))
  .filter((m) => m.compatibleFlavors.includes(flavorId))
  .map((m) => ({
    id: m.id,
    repo: m.source.repo,
    commit: m.source.commit,
    dbimportId: m.database.dbimportId,
    mandatory: flavor.requiredModules.includes(m.id) || m.removable === false,
    // Files of its `sql` options, packaged next to its SQL (ADR 0027).
    ...((m.options ?? []).some((o) => o.type === "sql")
      ? { optionalSql: m.options.filter((o) => o.type === "sql").map((o) => o.file) }
      : {}),
  }));

for (const required of flavor.requiredModules) {
  if (!modules.some((m) => m.id === required)) {
    console.error(`required module ${required} is missing from the catalog`);
    process.exit(1);
  }
}

// Runtime dependencies, as declared by the emulator.
const emulatorManifest = read(join(emulatorDir, "emulator.json"));
const components = emulatorManifest.components.map((c) => `catalog/components/${c.id}.json`);
const database = components.find((path) => read(join(root, path)).kind === "database");
if (!database) {
  console.error(`${emulator} declares no database component`);
  process.exit(1);
}
const client = `catalog/clients/${emulatorManifest.client}.json`;

// The build key covers every input that changes the artifact: identical inputs
// => identical key => the existing artifact is reused instead of compiling.
// Recipe = the workflow's global env (toolchain versions) + its `build` job
// (comment lines ignored) + every script the build job runs. Smoke-test-only
// edits do not change the key.
const workflow = text(join(root, ".github", "workflows", "build-server.yml"));
const section = (from, to) => {
  const start = workflow.indexOf(from);
  const end = start < 0 ? -1 : workflow.indexOf(to, start + from.length);
  if (end < 0) {
    console.error(`build-server.yml: section '${from.trim()}' .. '${to.trim()}' not found`);
    process.exit(1);
  }
  return workflow.slice(start, end);
};
// Action versions are left out too: actions are pinned by SHA and bumped by
// Dependabot, and checkout or cache never change the binaries; a bump must
// not rebuild the server and offer every realm an identical update.
const withoutComments = (yaml) =>
  yaml
    .split("\n")
    .filter((line) => !/^\s*#/.test(line))
    .map((line) => line.replace(/^(\s*(?:-\s*)?uses:\s*[^@\s]+)@.*$/, "$1"))
    .join("\n");
const recipe = [
  withoutComments(section("\nenv:\n", "\njobs:\n")),
  withoutComments(section(...recipes[platform].job)),
  ...["build-plan.mjs", "package-server.mjs", ...recipes[platform].scripts].map((f) => text(join(here, f))),
].join("\n");
const keyInput = JSON.stringify({
  platform,
  core: [flavor.source.repo, flavor.source.commit],
  // Whole plan entries: package-server.mjs writes them into build-info.json.
  modules,
  // MySQL client libraries linked into the servers.
  database: read(join(root, database)).platforms[platform] ?? null,
  recipe: sha256(recipe),
});
const buildKey = sha256(keyInput).slice(0, 16);
// The same sources on every platform (ADR 0031): player data moves freely
// between builds with the same source key (handoff, exports, restores).
const sourceKey = sha256(JSON.stringify({ core: [flavor.source.repo, flavor.source.commit], modules })).slice(0, 16);

const plan = {
  emulator,
  flavor: flavorId,
  platform,
  buildKey,
  sourceKey,
  core: { repo: flavor.source.repo, commit: flavor.source.commit },
  modules,
  database,
  client,
};
writeFileSync(out, JSON.stringify(plan, null, 2) + "\n");
console.log(JSON.stringify(plan, null, 2));

if (process.env.GITHUB_OUTPUT) {
  appendFileSync(
    process.env.GITHUB_OUTPUT,
    [
      `build-key=${buildKey}`,
      `source-key=${sourceKey}`,
      `core-repo=${flavor.source.repo}`,
      `core-commit=${flavor.source.commit}`,
      `artifact=server-${emulator}-${flavorId}-${buildKey}`,
      `mandatory=${modules.filter((m) => m.mandatory).map((m) => m.id).join(",")}`,
      `optional=${modules.filter((m) => !m.mandatory).map((m) => m.id).join(",")}`,
      `database-manifest=${database}`,
      `client-manifest=${client}`,
      "",
    ].join("\n"),
  );
}
