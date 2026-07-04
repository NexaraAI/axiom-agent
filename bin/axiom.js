#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const { spawn } = require("child_process");
const { resolvePlatform } = require("../scripts/resolve-platform");

function defaultInstalledBinaryPath(baseDir = __dirname, platform = process.platform, arch = process.arch) {
  const platformInfo = resolvePlatform(platform, arch);
  return path.join(baseDir, "..", "vendor", "bin", platformInfo.assetName);
}

function resolveAxiomBinary(options = {}) {
  const env = options.env || process.env;
  const fsImpl = options.fsImpl || fs;
  const baseDir = options.baseDir || __dirname;
  const platform = options.platform || process.platform;
  const arch = options.arch || process.arch;

  if (env.AXIOM_AGENT_BINARY_PATH) {
    const overridePath = path.resolve(env.AXIOM_AGENT_BINARY_PATH);
    if (!fsImpl.existsSync(overridePath)) {
      throw new Error(
        "Axiom binary is missing. Try reinstalling with npm or set AXIOM_AGENT_BINARY_PATH during development."
      );
    }
    return overridePath;
  }

  const installedPath = defaultInstalledBinaryPath(baseDir, platform, arch);
  if (!fsImpl.existsSync(installedPath)) {
    throw new Error(
      "Axiom binary is missing. Try reinstalling with npm or set AXIOM_AGENT_BINARY_PATH during development."
    );
  }

  return installedPath;
}

function run(argv = process.argv.slice(2), options = {}) {
  let binaryPath;
  try {
    binaryPath = resolveAxiomBinary(options);
  } catch (error) {
    console.error(error.message);
    return 1;
  }

  const child = spawn(binaryPath, argv, {
    stdio: "inherit",
    windowsHide: false
  });

  child.on("error", (error) => {
    console.error(`Failed to start Axiom binary: ${error.message}`);
    process.exit(1);
  });

  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code === null ? 1 : code);
  });

  return 0;
}

if (require.main === module) {
  const immediateExitCode = run();
  if (immediateExitCode !== 0) {
    process.exit(immediateExitCode);
  }
}

module.exports = {
  defaultInstalledBinaryPath,
  resolveAxiomBinary,
  run
};
