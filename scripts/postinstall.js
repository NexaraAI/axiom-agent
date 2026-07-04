"use strict";

const fs = require("fs");
const path = require("path");
const packageJson = require("../package.json");
const { downloadAndVerifyBinary, makeExecutable, releaseRepoFromPackage } = require("./download-binary");
const { resolvePlatform } = require("./resolve-platform");

function packageRoot() {
  return path.resolve(__dirname, "..");
}

function installedBinaryPath(platformInfo, root = packageRoot()) {
  return path.join(root, "vendor", "bin", platformInfo.assetName);
}

function resolveDevelopmentBinaryPath(binaryPath, fsImpl = fs) {
  if (!binaryPath) {
    return null;
  }

  const resolved = path.resolve(binaryPath);
  if (!fsImpl.existsSync(resolved)) {
    throw new Error(`AXIOM_AGENT_BINARY_PATH does not exist: ${resolved}`);
  }

  const stats = fsImpl.statSync(resolved);
  if (!stats.isFile()) {
    throw new Error(`AXIOM_AGENT_BINARY_PATH is not a file: ${resolved}`);
  }

  return resolved;
}

function installFromDevelopmentOverride(sourcePath, destination, platform = process.platform) {
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.copyFileSync(sourcePath, destination);
  makeExecutable(destination, platform);
  return destination;
}

async function main(env = process.env) {
  const platformInfo = resolvePlatform();
  const destination = installedBinaryPath(platformInfo);
  const overridePath = resolveDevelopmentBinaryPath(env.AXIOM_AGENT_BINARY_PATH);

  if (overridePath) {
    installFromDevelopmentOverride(overridePath, destination);
    console.log(`Axiom binary installed from AXIOM_AGENT_BINARY_PATH: ${destination}`);
    return;
  }

  const repo = releaseRepoFromPackage(packageJson, env);
  await downloadAndVerifyBinary({
    repo,
    version: packageJson.version,
    assetName: platformInfo.assetName,
    destination,
    platform: process.platform
  });
  console.log(`Axiom binary installed: ${destination}`);
}

if (require.main === module) {
  main().catch((error) => {
    console.error(`Axiom install failed: ${error.message}`);
    process.exit(1);
  });
}

module.exports = {
  installFromDevelopmentOverride,
  installedBinaryPath,
  main,
  resolveDevelopmentBinaryPath
};
