"use strict";

const fs = require("fs");
const path = require("path");
const packageJson = require("../package.json");

function rustWorkspaceVersion(cargoTomlPath = path.join(__dirname, "..", "Cargo.toml")) {
  const cargoToml = fs.readFileSync(cargoTomlPath, "utf8");
  const workspacePackage = cargoToml.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/);
  if (!workspacePackage) {
    throw new Error("Cargo.toml is missing [workspace.package].");
  }

  const version = workspacePackage[1].match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!version) {
    throw new Error("Cargo.toml [workspace.package] is missing version.");
  }

  return version[1];
}

function checkVersionSync() {
  const rustVersion = rustWorkspaceVersion();
  if (packageJson.version !== rustVersion) {
    throw new Error(
      `package.json version (${packageJson.version}) does not match Cargo workspace version (${rustVersion}).`
    );
  }
  return true;
}

if (require.main === module) {
  try {
    checkVersionSync();
    console.log("package.json and Cargo workspace versions match.");
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}

module.exports = {
  checkVersionSync,
  rustWorkspaceVersion
};
