#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const { spawn, spawnSync } = require("child_process");
const { resolvePlatform } = require("../scripts/resolve-platform");

function defaultInstalledBinaryPath(baseDir = __dirname, platform = process.platform, arch = process.arch) {
  const platformInfo = resolvePlatform(platform, arch);
  return path.join(baseDir, "..", "vendor", "bin", platformInfo.assetName);
}

function readInstalledPackageVersion(baseDir = __dirname) {
  try {
    const manifest = JSON.parse(
      fs.readFileSync(path.join(baseDir, "..", "package.json"), "utf8")
    );
    return typeof manifest.version === "string" ? manifest.version : null;
  } catch {
    return null;
  }
}

function versionFromOutput(text) {
  const trimmed = String(text || "").trim();
  if (!trimmed) {
    return null;
  }
  const last = trimmed.split(/\s+/).pop();
  return /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/.test(last) ? last : null;
}

// Verify a freshly installed npm package can actually run: the shim must
// report exactly the version the package declares. A mismatch means the
// vendored binary is stale (a blocked or failed postinstall left the old
// binary in place) or broken, and the update must not be reported as a
// success.
function verifyInstalledUpdate(baseDir = __dirname) {
  const declared = readInstalledPackageVersion(baseDir);
  if (!declared) {
    return { ok: false, reason: "package manifest (package.json) is unreadable" };
  }
  const shimResult = spawnSync(
    process.execPath,
    [path.join(baseDir, "axiom.js"), "--version"],
    { encoding: "utf8" }
  );
  if (shimResult.error) {
    return { ok: false, reason: `shim launch failed: ${shimResult.error.message}` };
  }
  if (shimResult.status !== 0) {
    const stderr = (shimResult.stderr || "").trim();
    return {
      ok: false,
      reason: `shim --version exited with status ${shimResult.status}: ${stderr}`
    };
  }
  const reported = versionFromOutput(shimResult.stdout);
  if (!reported) {
    return { ok: false, reason: "shim --version produced no semantic version" };
  }
  if (reported !== declared) {
    return {
      ok: false,
      reason: `vendored binary reports v${reported} but the npm package is v${declared} — the binary is stale (postinstall was blocked or failed)`
    };
  }
  return { ok: true, version: reported };
}

function rollbackNpmInstall(previousVersion) {
  const npmCmd = process.platform === "win32" ? "npm.cmd" : "npm";
  const result = spawnSync(npmCmd, ["install", "-g", `axiom-agent@${previousVersion}`], {
    stdio: "inherit",
    shell: process.platform === "win32"
  });
  return result.status === 0;
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
    if (code === 42) {
      console.log("\n[axiom] Updating Axiom globally via npm (binary unlocked)...");
      const npmCmd = process.platform === "win32" ? "npm.cmd" : "npm";
      const binDir = options.baseDir || __dirname;
      // Capture the currently installed version BEFORE the update so a failed
      // post-install verification can restore it.
      const previousVersion = readInstalledPackageVersion(binDir);
      const updateResult = spawnSync(npmCmd, ["install", "-g", "axiom-agent@latest"], {
        stdio: "inherit",
        shell: process.platform === "win32"
      });
      if (updateResult.status !== 0) {
        console.error("\n[axiom] Automatic update failed. Try running `npm install -g axiom-agent@latest` manually.");
        process.exit(1);
      }
      const verification = verifyInstalledUpdate(binDir);
      if (!verification.ok) {
        console.error("\n[axiom] Update verification failed: " + verification.reason);
        if (previousVersion) {
          console.error(`[axiom] Rolling back to axiom-agent@${previousVersion}...`);
          if (rollbackNpmInstall(previousVersion)) {
            console.error(`[axiom] Rollback complete. Axiom remains on v${previousVersion}; the new version failed verification.`);
          } else {
            console.error(`[axiom] Rollback failed. Restore manually with: npm install -g axiom-agent@${previousVersion}`);
          }
        } else {
          console.error("[axiom] Could not determine the previous version; restore manually with: npm install -g axiom-agent@<previous-version>");
        }
        process.exit(1);
      }
      console.log(`\n[axiom] Update verified (v${verification.version}). Restarting Axiom...\n`);
      const nextCode = run(argv, options);
      if (nextCode !== 0) {
        process.exit(nextCode);
      }
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
  readInstalledPackageVersion,
  versionFromOutput,
  verifyInstalledUpdate,
  rollbackNpmInstall,
  run
};
