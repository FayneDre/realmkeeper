#!/usr/bin/env node
// Assembles the server artifact staging tree from a finished AzerothCore build:
//
//   <staging>/
//     dist/                     everything `cmake --install` produced (executables, DLLs, .conf.dist)
//     sql/data/sql/**           core SQL (read by dbimport through `SourceDirectory`)
//     sql/modules/<id>/data/sql/**   module SQL, one directory per catalog module id
//     sql/modules/<id>/<file>   files of the module's `sql` options (ADR 0027;
//                               emulator `instance.optionalSql.source`)
//     build-info.json           plan (exact source commits) + toolchain information
//     files.json                every file with size and sha256 (integrity + differential updates)
//
// Usage: node tools/ci/package-server.mjs <plan.json> <source-dir> <dist-dir> <staging-dir>

import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, relative, sep } from "node:path";

const [planPath, sourceDir, distDir, staging] = process.argv.slice(2);
if (!planPath || !sourceDir || !distDir || !staging) {
  console.error("usage: package-server.mjs <plan.json> <source-dir> <dist-dir> <staging-dir>");
  process.exit(2);
}
const plan = JSON.parse(readFileSync(planPath, "utf8"));

mkdirSync(staging, { recursive: true });
cpSync(distDir, join(staging, "dist"), { recursive: true });
cpSync(join(sourceDir, "data", "sql"), join(staging, "sql", "data", "sql"), { recursive: true });
for (const module of plan.modules) {
  const moduleSql = join(sourceDir, "modules", module.id, "data", "sql");
  if (existsSync(moduleSql)) {
    cpSync(moduleSql, join(staging, "sql", "modules", module.id, "data", "sql"), { recursive: true });
  } else {
    console.warn(`note: ${module.id} has no data/sql directory`);
  }
  for (const file of module.optionalSql ?? []) {
    const source = join(sourceDir, "modules", module.id, ...file.split("/"));
    if (!existsSync(source)) {
      console.error(`${module.id}: optional SQL file ${file} is missing at the pinned commit`);
      process.exit(1);
    }
    const target = join(staging, "sql", "modules", module.id, ...file.split("/"));
    mkdirSync(dirname(target), { recursive: true });
    cpSync(source, target);
  }
}

// Executables plus the runtime libraries they load from their own directory.
// Windows: legacy.dll is the OpenSSL 3 legacy provider, asserted at startup by
// OpenSSLCrypto.cpp. Linux (ADR 0031): libraries outside the system base are
// bundled in lib/ by bundle-libs.sh; OpenSSL 3 comes from the system. Both
// layouts are the same: executables at the root, configs/ with the .conf.dist.
const required = {
  "windows-x86_64": [
    "worldserver.exe",
    "authserver.exe",
    "dbimport.exe",
    "libmysql.dll",
    "libssl-3-x64.dll",
    "libcrypto-3-x64.dll",
    "legacy.dll",
  ],
  "linux-x86_64": ["worldserver", "authserver", "dbimport", "lib/libmysqlclient.so.24", "configs/worldserver.conf.dist"],
}[plan.platform ?? "windows-x86_64"];
if (!required) {
  console.error(`unknown platform ${plan.platform}`);
  process.exit(1);
}
const walk = (dir) =>
  readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
const distFiles = walk(join(staging, "dist")).map((p) => relative(join(staging, "dist"), p).split(sep).join("/"));
for (const exe of required) {
  if (!distFiles.some((f) => f.endsWith(exe))) {
    console.error(`missing ${exe} in build output`);
    process.exit(1);
  }
}

writeFileSync(
  join(staging, "build-info.json"),
  JSON.stringify(
    {
      ...plan,
      builtAt: new Date().toISOString(),
      runner: { os: process.env.RUNNER_OS ?? null, image: process.env.ImageOS ?? null },
      ci: process.env.GITHUB_RUN_ID
        ? { repository: process.env.GITHUB_REPOSITORY, runId: process.env.GITHUB_RUN_ID, sha: process.env.GITHUB_SHA }
        : null,
    },
    null,
    2,
  ) + "\n",
);

const files = walk(staging)
  .map((path) => {
    const rel = relative(staging, path).split(sep).join("/");
    const data = readFileSync(path);
    return { path: rel, size: data.length, sha256: createHash("sha256").update(data).digest("hex") };
  })
  .filter((f) => f.path !== "files.json")
  .sort((a, b) => a.path.localeCompare(b.path));
writeFileSync(join(staging, "files.json"), JSON.stringify({ buildKey: plan.buildKey, files }, null, 2) + "\n");

const total = files.reduce((sum, f) => sum + f.size, 0);
console.log(`packaged ${files.length} files, ${(total / 1048576).toFixed(1)} MB`);
console.log("dist layout:");
for (const f of distFiles.filter((f) => !f.includes("/") || f.split("/").length <= 2).slice(0, 80)) console.log(`  ${f}`);
