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
    const raw = String(env.AXIOM_AGENT_BINARY_PATH).trim();
    if (!raw) {
      throw new Error(
        "AXIOM_AGENT_BINARY_PATH is set but empty. Unset it to use the installed binary."
      );
    }
    const overridePath = path.resolve(raw);

    if (process.env.AXIOM_ALLOW_UNSAFE_BINARY_PATH !== "1") {
      console.error(
        "[axiom] WARNING: AXIOM_AGENT_BINARY_PATH override in use: " + overridePath + "\n" +
        "[axiom] Only use this for local development. Unset it for normal runs."
      );
    }
    let stat = null;
    try {
      if (typeof fsImpl.statSync === "function") {
        stat = fsImpl.statSync(overridePath);
      } else if (fsImpl.existsSync(overridePath)) {
        return overridePath;
      }
    } catch {
      stat = null;
    }
    if (!stat || !stat.isFile()) {
      throw new Error(
        "Axiom binary override is missing or not a file: " + overridePath + ". Try reinstalling with npm or unset AXIOM_AGENT_BINARY_PATH."
      );
    }
    return overridePath;
  }

  const installedPath = defaultInstalledBinaryPath(baseDir, platform, arch);
  if (!fsImpl.existsSync(installedPath)) {
    const postinstallPath = path.join(baseDir, "..", "scripts", "postinstall.js");
    if (fsImpl.existsSync(postinstallPath)) {
      console.log("[axiom] Downloading native binary for your platform...");
      const { spawnSync } = require("child_process");
      const downloadResult = spawnSync(process.execPath, [postinstallPath], {
        stdio: "inherit"
      });
      if (downloadResult.status === 0 && fsImpl.existsSync(installedPath)) {
        return installedPath;
      }
    }
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
