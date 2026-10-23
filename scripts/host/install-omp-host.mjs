#!/usr/bin/env node
import { createHash, randomUUID } from "node:crypto";
import { constants, createReadStream, createWriteStream } from "node:fs";
import { chmod, lstat, mkdir, readFile, realpath, rename, rm } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { Readable, Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

const version = "18.12.3";
// Published release digests, pinned with the SDK rather than trusted at download time.
const digests = {
  "omp-darwin-arm64": "f2a657b3320fcafa23055053940a45b26a8d3254e1412a158838013d670d6ec5",
  "omp-darwin-x64": "73918ae225fa63bc0686081dce4eb90716540ef94e55185b999c697999f7b2c9",
  "omp-linux-arm64": "58f7c25e9b729b082192049ada07a31820c8b257325545a5fe232ff1679f1093",
  "omp-linux-x64": "065b5079d65ff6b77d961c935ba808e8f0f569ca6a2cb7b5fb6a1e8da72d7dec",
  "omp-linux-musl-arm64": "cd5497cf754c2278f62fbd1d0c0e5e3dbb01c55201eeb4d20ffba877de00af6d",
  "omp-linux-musl-x64": "8e1dfb5aa052600ba17f28be5af74626cc8377c7fd5025d72cd7ce910dc83e39",
  "omp-windows-x64.exe": "5e22e9ff5d93bde52a402d4ec4cfd073b2f0f02233181f0b7634c6441e61a108",
};

function confinedPath(directory, name) {
  const candidate = resolve(directory, name);
  const remainder = relative(directory, candidate);
  if (remainder === "" || remainder === ".." || remainder.startsWith(`..${sep}`) || isAbsolute(remainder)) {
    throw new Error(`Path escapes the OMP host cache: ${candidate}`);
  }
  return candidate;
}

function assertWithinRoot(root, candidate) {
  const remainder = relative(root, candidate);
  if (remainder === ".." || remainder.startsWith(`..${sep}`) || isAbsolute(remainder)) {
    throw new Error(`OMP host cache escapes the Rerail package root: ${candidate}`);
  }
}

async function validateDirectory(root, directory) {
  const info = await lstat(directory);
  if (info.isSymbolicLink() || !info.isDirectory()) {
    throw new Error(`OMP host cache component is not a real directory: ${directory}`);
  }
  if (process.platform !== "win32") {
    if (info.uid !== process.getuid()) {
      throw new Error(`OMP host cache component is not owned by the current user: ${directory}`);
    }
    if ((info.mode & 0o022) !== 0) {
      throw new Error(`OMP host cache component is writable by another user: ${directory}`);
    }
  }
  const canonical = await realpath(directory);
  assertWithinRoot(root, canonical);
  if (relative(directory, canonical) !== "") {
    throw new Error(`OMP host cache component changed identity: ${directory}`);
  }
  return canonical;
}

async function ensureCacheDirectory(root, parent, name) {
  const directory = confinedPath(parent, name);
  let created = false;
  try {
    await mkdir(directory, { mode: 0o700 });
    created = true;
  } catch (error) {
    if (error.code !== "EEXIST") throw error;
  }
  let canonical = await validateDirectory(root, directory);
  if (created && process.platform !== "win32") {
    await chmod(canonical, 0o700);
    canonical = await validateDirectory(root, canonical);
    if (((await lstat(canonical)).mode & 0o777) !== 0o700) {
      throw new Error(`New OMP host cache component is not private: ${canonical}`);
    }
  }
  return canonical;
}
const root = await realpath(resolve(dirname(fileURLToPath(import.meta.url)), "..", ".."));
const manifest = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
if (manifest.devDependencies["@oh-my-pi/pi-coding-agent"] !== version) {
  throw new Error("The pinned OMP host is out of step with the SDK dependency: run npm run deps:sync.");
}
if (process.argv.length !== 2) throw new Error("Usage: node scripts/host/install-omp-host.mjs");
const platform = process.platform === "win32" ? "windows" : process.platform;
const libc = platform === "linux" && !process.report.getReport().header.glibcVersionRuntime ? "-musl" : "";
const extension = platform === "windows" ? ".exe" : "";
const asset = `omp-${platform}${libc}-${process.arch}${extension}`;
const expected = digests[asset];
if (expected === undefined) throw new Error(`No pinned standalone OMP host is available for ${platform}/${process.arch}${libc}.`);
let output = root;
for (const component of ["node_modules", ".cache", "rerail", `${version}-${asset}`]) {
  output = await ensureCacheDirectory(root, output, component);
}
const executable = confinedPath(output, `omp${extension}`);
let installed = false;
try {
  const executableInfo = await lstat(executable);
  if (executableInfo.isSymbolicLink() || !executableInfo.isFile()) {
    throw new Error(`Cached OMP host is not a regular file: ${executable}`);
  }
  const existingHash = createHash("sha256");
  for await (const chunk of createReadStream(executable)) existingHash.update(chunk);
  installed = existingHash.digest("hex") === expected;
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
if (!installed) {
  const response = await fetch(`https://github.com/can1357/oh-my-pi/releases/download/v${version}/${asset}`, {
    signal: AbortSignal.timeout(180_000),
  });
  if (!response.ok || response.body === null) throw new Error(`OMP host download failed: HTTP ${response.status}`);
  await validateDirectory(root, output);
  const temporary = confinedPath(output, `.omp-${randomUUID()}.download`);
  try {
    const hash = createHash("sha256");
    await pipeline(
      Readable.fromWeb(response.body),
      new Transform({
        transform(chunk, _encoding, callback) {
          hash.update(chunk);
          callback(null, chunk);
        },
      }),
      createWriteStream(temporary, {
        flags: constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | (constants.O_NOFOLLOW ?? 0),
        mode: 0o700,
      }),
    );
    if (hash.digest("hex") !== expected) throw new Error(`OMP host checksum mismatch for ${asset}`);
    const temporaryInfo = await lstat(temporary);
    if (temporaryInfo.isSymbolicLink() || !temporaryInfo.isFile()) {
      throw new Error(`Downloaded OMP host is not a regular file: ${temporary}`);
    }
    await validateDirectory(root, output);
    await rename(temporary, executable);
  } finally {
    await rm(temporary, { force: true });
  }
}
await validateDirectory(root, output);
const finalExecutableInfo = await lstat(executable);
if (finalExecutableInfo.isSymbolicLink() || !finalExecutableInfo.isFile()) {
  throw new Error(`Cached OMP host is not a regular file: ${executable}`);
}
await chmod(executable, 0o700);
process.stdout.write(`${executable}\n`);
