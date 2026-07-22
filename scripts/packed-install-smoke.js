"use strict";

const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

const REPO_ROOT = path.resolve(__dirname, "..");

function npmInvocation(env = process.env) {
  if (env.npm_execpath && fs.existsSync(env.npm_execpath)) {
    return { command: process.execPath, prefix: [env.npm_execpath], shell: false };
  }
  return {
    command: process.platform === "win32" ? "npm.cmd" : "npm",
    prefix: [],
    shell: process.platform === "win32"
  };
}

function runChecked(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    shell: false,
    timeout: 120_000,
    ...options
  });
  if (result.error || result.status !== 0) {
    const detail = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(
      `Command failed: ${command} ${args.join(" ")}\n${result.error ? result.error.message : detail}`
    );
  }
  return result;
}

function runPackedInstallSmoke(options = {}) {
  const root = options.repoRoot || REPO_ROOT;
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "axiom-packed-smoke-"));
  const cache = path.join(tempRoot, "npm-cache");
  const prefix = path.join(tempRoot, "prefix");
  const invocation = npmInvocation(options.env || process.env);
  const env = {
    ...(options.env || process.env),
    AXIOM_AGENT_BINARY_PATH: process.execPath,
    NPM_CONFIG_CACHE: cache
  };

  try {
    const packed = runChecked(
      invocation.command,
      [...invocation.prefix, "pack", "--json", "--pack-destination", tempRoot],
      { cwd: root, env, shell: invocation.shell }
    );
    const packResult = JSON.parse(packed.stdout);
    assert(Array.isArray(packResult) && packResult.length === 1, "npm pack returned unexpected JSON");
    const tarball = path.join(tempRoot, packResult[0].filename);
    assert(fs.existsSync(tarball), "npm pack did not create the reported tarball");

    runChecked(
      invocation.command,
      [
        ...invocation.prefix,
        "install",
        "--global",
        "--offline",
        "--ignore-scripts=false",
        "--no-audit",
        "--no-fund",
        "--prefix",
        prefix,
        tarball
      ],
      { cwd: root, env, shell: invocation.shell }
    );

    const shim = process.platform === "win32"
      ? path.join(prefix, "axiom.cmd")
      : path.join(prefix, "bin", "axiom");
    assert(fs.existsSync(shim), `installed npm shim is missing: ${shim}`);
    const shimEnv = { ...env, AXIOM_AGENT_BINARY_PATH: "" };
    const invoked = process.platform === "win32"
      ? runChecked(
          process.env.ComSpec || "cmd.exe",
          ["/d", "/s", "/c", `call "${shim}" --version`],
          { cwd: tempRoot, env: shimEnv, windowsVerbatimArguments: true }
        )
      : runChecked(shim, ["--version"], { cwd: tempRoot, env: shimEnv });
    assert(
      `${invoked.stdout}\n${invoked.stderr}`.includes(process.version),
      "installed shim did not execute the binary copied by postinstall"
    );

    console.log("Packed npm tarball installed and its global axiom shim executed successfully.");
    return true;
  } finally {
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

if (require.main === module) {
  try {
    runPackedInstallSmoke();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}

module.exports = {
  npmInvocation,
  runPackedInstallSmoke
};
