"use strict";

const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { defaultInstalledBinaryPath, resolveAxiomBinary } = require("../bin/axiom");
const { checkVersionSync } = require("./check-version-sync");
const { resolveDevelopmentBinaryPath } = require("./postinstall");
const { resolvePlatform, UnsupportedPlatformError } = require("./resolve-platform");
const { sha256Buffer, verifyChecksum } = require("./verify-checksum");

function testPlatformResolver() {
  assert.strictEqual(
    resolvePlatform("win32", "x64").assetName,
    "axiom-x86_64-pc-windows-msvc.exe"
  );
  assert.strictEqual(
    resolvePlatform("linux", "x64").assetName,
    "axiom-x86_64-unknown-linux-gnu"
  );
  assert.strictEqual(
    resolvePlatform("darwin", "x64").assetName,
    "axiom-x86_64-apple-darwin"
  );
  assert.strictEqual(
    resolvePlatform("darwin", "arm64").assetName,
    "axiom-aarch64-apple-darwin"
  );
  assert.throws(() => resolvePlatform("freebsd", "x64"), UnsupportedPlatformError);
}

function testChecksumVerification() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "axiom-smoke-"));
  const file = path.join(dir, "axiom-test");
  fs.writeFileSync(file, "hello");
  const checksum = sha256Buffer(Buffer.from("hello"));
  const checksums = `${checksum}  axiom-test\n`;

  assert.strictEqual(verifyChecksum(file, checksums, "axiom-test"), true);
  assert.throws(
    () => verifyChecksum(file, `${"0".repeat(64)}  axiom-test\n`, "axiom-test"),
    /Checksum mismatch/
  );
  fs.rmSync(dir, { recursive: true, force: true });
}

function testWrapperPathResolution() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "axiom-wrapper-"));
  const assetName = resolvePlatform("linux", "x64").assetName;
  const binDir = path.join(dir, "bin");
  const binary = path.join(dir, "vendor", "bin", assetName);
  fs.mkdirSync(path.dirname(binary), { recursive: true });
  fs.writeFileSync(binary, "binary");

  assert.strictEqual(defaultInstalledBinaryPath(binDir, "linux", "x64"), binary);
  assert.strictEqual(
    resolveAxiomBinary({
      env: {},
      baseDir: binDir,
      platform: "linux",
      arch: "x64"
    }),
    binary
  );
  assert.throws(
    () =>
      resolveAxiomBinary({
        env: {},
        baseDir: path.join(dir, "missing", "bin"),
        platform: "linux",
        arch: "x64"
      }),
    /Axiom binary is missing/
  );

  fs.rmSync(dir, { recursive: true, force: true });
}

function testDevelopmentOverrideValidation() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "axiom-override-"));
  const binary = path.join(dir, process.platform === "win32" ? "axiom.exe" : "axiom");
  fs.writeFileSync(binary, "binary");

  assert.strictEqual(resolveDevelopmentBinaryPath(binary), binary);
  assert.throws(
    () => resolveDevelopmentBinaryPath(path.join(dir, "missing")),
    /AXIOM_AGENT_BINARY_PATH does not exist/
  );
  fs.rmSync(dir, { recursive: true, force: true });
}

function main() {
  testPlatformResolver();
  testChecksumVerification();
  testWrapperPathResolution();
  testDevelopmentOverrideValidation();
  checkVersionSync();
  console.log("Node smoke tests passed.");
}

if (require.main === module) {
  main();
}

module.exports = {
  main
};
