#!/usr/bin/env node

import fs from "node:fs";
import crypto from "node:crypto";
import process from "node:process";

const wasmPath = process.argv[2];
if (!wasmPath) {
  throw new Error("usage: node tools/wasm_cpu_bench.mjs <aemu.wasm> [memory-iterations] [cpu-iterations] [runs]");
}

const memoryIterations = Number(process.argv[3] ?? 10_000_000);
const cpuIterations = Number(process.argv[4] ?? 1_000_000);
const runs = Number(process.argv[5] ?? 7);
if (
  ![memoryIterations, cpuIterations, runs].every(
    (value) => Number.isSafeInteger(value) && value > 0 && value <= 0xffff_ffff,
  )
) {
  throw new Error("iterations and runs must be positive safe integers");
}

const bytes = fs.readFileSync(wasmPath);
const module = await WebAssembly.compile(bytes);
const imports = {};
for (const entry of WebAssembly.Module.imports(module)) {
  if (entry.kind !== "function") {
    throw new Error(`unsupported Wasm import ${entry.module}.${entry.name}: ${entry.kind}`);
  }
  imports[entry.module] ??= {};
  imports[entry.module][entry.name] = () => 0;
}
const { exports } = await WebAssembly.instantiate(module, imports);

function median(values) {
  const ordered = [...values].sort((a, b) => a - b);
  const middle = Math.floor(ordered.length / 2);
  return ordered.length % 2 === 0
    ? (ordered[middle - 1] + ordered[middle]) / 2
    : ordered[middle];
}

function measure(name, iterations) {
  const fn = exports[name];
  if (typeof fn !== "function") {
    throw new Error(`missing Wasm export: ${name}`);
  }
  const expected = fn(iterations) >>> 0;
  fn(iterations);
  const elapsedMs = [];
  for (let run = 0; run < runs; run += 1) {
    const start = process.hrtime.bigint();
    const checksum = fn(iterations) >>> 0;
    const end = process.hrtime.bigint();
    if (checksum !== expected) {
      throw new Error(`${name} checksum changed: ${checksum} != ${expected}`);
    }
    elapsedMs.push(Number(end - start) / 1e6);
  }
  return {
    iterations,
    checksum: `0x${expected.toString(16).padStart(8, "0")}`,
    elapsedMs,
    medianMs: median(elapsedMs),
  };
}

const result = {
  wasm: {
    path: wasmPath,
    bytes: bytes.length,
    sha256: crypto.createHash("sha256").update(bytes).digest("hex"),
  },
  runtime: {
    node: process.version,
    v8: process.versions.v8,
  },
  runs,
  memory: measure("aemu_wasm_memory_benchmark", memoryIterations),
  cpu: measure("aemu_wasm_cpu_benchmark", cpuIterations),
};
console.log(JSON.stringify(result, null, 2));
