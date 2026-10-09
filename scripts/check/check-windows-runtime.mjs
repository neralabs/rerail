// Release guard: a Windows engine must load on a clean machine, so it may not import the
// Visual C++ redistributable (VCRUNTIME140.dll, MSVCP140.dll). .cargo/config.toml links
// the CRT statically; this fails the build if that ever stops being true.
import { readFileSync } from "node:fs";

const REDISTRIBUTABLE = /^(?:vcruntime|msvcp|concrt|vcomp)\d+(?:_\w+)?\.dll$/iu;

/** DLL names in a PE image's import directory. */
function importedDlls(image) {
  const header = image.readUInt32LE(0x3c);
  if (image.toString("latin1", header, header + 4) !== "PE\0\0") throw new Error("not a PE image");
  const sectionCount = image.readUInt16LE(header + 6);
  const optionalHeader = header + 24;
  const optionalSize = image.readUInt16LE(header + 20);
  const dataDirectories = optionalHeader + (image.readUInt16LE(optionalHeader) === 0x20b ? 112 : 96);
  const importDirectory = image.readUInt32LE(dataDirectories + 8);
  const sections = Array.from({ length: sectionCount }, (_, index) => {
    const section = optionalHeader + optionalSize + index * 40;
    return { address: image.readUInt32LE(section + 12), size: image.readUInt32LE(section + 8), raw: image.readUInt32LE(section + 20) };
  });
  const offset = address => {
    const section = sections.find(candidate => address >= candidate.address && address < candidate.address + candidate.size);
    if (section === undefined) throw new Error(`address 0x${address.toString(16)} is outside every section`);
    return address - section.address + section.raw;
  };
  const names = [];
  for (let descriptor = offset(importDirectory); image.readUInt32LE(descriptor + 12) !== 0; descriptor += 20) {
    const name = offset(image.readUInt32LE(descriptor + 12));
    names.push(image.toString("latin1", name, image.indexOf(0, name)));
  }
  return names;
}

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("Usage: node scripts/check-windows-runtime.mjs <kernel.node>...");
  process.exit(2);
}
let failed = false;
for (const file of files) {
  const redistributable = importedDlls(readFileSync(file)).filter(name => REDISTRIBUTABLE.test(name));
  if (redistributable.length > 0) {
    console.error(`${file} imports the Visual C++ redistributable (${redistributable.join(", ")}); it will not load on a clean Windows install.`);
    failed = true;
  } else {
    console.log(`${file}: no Visual C++ redistributable imports`);
  }
}
process.exitCode = failed ? 1 : 0;
