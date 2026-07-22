"use strict";

const fs = require("fs");
const path = require("path");
const packageJson = require("../package.json");

const REPO_ROOT = path.resolve(__dirname, "..");

function manifestSection(content, sectionName) {
  const escaped = sectionName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = content.match(new RegExp(`\\[${escaped}\\]([\\s\\S]*?)(?:\\n\\[|$)`));
  if (!match) {
    throw new Error(`Cargo manifest is missing [${sectionName}].`);
  }
  return match[1];
}

function rustWorkspaceVersion(cargoTomlPath = path.join(REPO_ROOT, "Cargo.toml")) {
  const cargoToml = fs.readFileSync(cargoTomlPath, "utf8");
  const workspacePackage = manifestSection(cargoToml, "workspace.package");
  const version = workspacePackage.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!version) {
    throw new Error("Cargo.toml [workspace.package] is missing version.");
  }
  return version[1];
}

function workspaceMemberPaths(cargoTomlPath = path.join(REPO_ROOT, "Cargo.toml")) {
  const cargoToml = fs.readFileSync(cargoTomlPath, "utf8");
  const workspace = manifestSection(cargoToml, "workspace");
  const members = workspace.match(/\bmembers\s*=\s*\[([\s\S]*?)\]/);
  if (!members) {
    throw new Error("Cargo.toml [workspace] is missing members.");
  }
  const paths = Array.from(members[1].matchAll(/"([^"]+)"/g), (match) => match[1]);
  if (paths.length === 0) {
    throw new Error("Cargo.toml workspace has no members.");
  }
  return paths;
}

function packageIdentity(manifestPath) {
  const content = fs.readFileSync(manifestPath, "utf8");
  const packageSection = manifestSection(content, "package");
  const name = packageSection.match(/^\s*name\s*=\s*"([^"]+)"/m);
  const exactVersion = packageSection.match(/^\s*version\s*=\s*"([^"]+)"/m);
  const inheritsVersion = /^\s*version\.workspace\s*=\s*true\s*$/m.test(packageSection);
  if (!name) {
    throw new Error(`${manifestPath} [package] is missing name.`);
  }
  return {
    content,
    exactVersion: exactVersion && exactVersion[1],
    inheritsVersion,
    name: name[1]
  };
}

function workspacePackages(repoRoot = REPO_ROOT) {
  return workspaceMemberPaths(path.join(repoRoot, "Cargo.toml")).map((memberPath) => {
    const manifestPath = path.resolve(repoRoot, memberPath, "Cargo.toml");
    if (!fs.existsSync(manifestPath)) {
      throw new Error(`Workspace member manifest is missing: ${manifestPath}`);
    }
    return {
      ...packageIdentity(manifestPath),
      manifestPath,
      memberPath,
      root: path.dirname(manifestPath)
    };
  });
}

function assertWorkspaceManifestVersions(packages, workspaceVersion) {
  for (const pkg of packages) {
    if (!pkg.inheritsVersion && pkg.exactVersion !== workspaceVersion) {
      throw new Error(
        `${pkg.manifestPath} package version must inherit the workspace version or equal ${workspaceVersion}.`
      );
    }
  }
}

function assertInternalPathDependencyPins(packages, workspaceVersion) {
  const packageRoots = new Map(
    packages.map((pkg) => [path.resolve(pkg.root).toLowerCase(), pkg])
  );

  for (const pkg of packages) {
    for (const match of pkg.content.matchAll(/^\s*([A-Za-z0-9_-]+)\s*=\s*\{([^}]*)\}/gm)) {
      const dependencyName = match[1];
      const fields = match[2];
      const pathMatch = fields.match(/\bpath\s*=\s*"([^"]+)"/);
      if (!pathMatch) {
        continue;
      }

      const dependencyRoot = path.resolve(pkg.root, pathMatch[1]).toLowerCase();
      const target = packageRoots.get(dependencyRoot);
      if (!target) {
        continue;
      }

      const versionMatch = fields.match(/\bversion\s*=\s*"([^"]+)"/);
      const required = `=${workspaceVersion}`;
      if (!versionMatch || versionMatch[1] !== required) {
        throw new Error(
          `${pkg.manifestPath} internal dependency ${dependencyName} (${target.name}) must use exact version ${required}.`
        );
      }
    }
  }
}

function lockedPackageVersions(lockPath = path.join(REPO_ROOT, "Cargo.lock")) {
  const lock = fs.readFileSync(lockPath, "utf8");
  const result = new Map();
  for (const block of lock.split(/^\[\[package\]\]\s*$/m).slice(1)) {
    const name = block.match(/^name\s*=\s*"([^"]+)"/m);
    const version = block.match(/^version\s*=\s*"([^"]+)"/m);
    const source = block.match(/^source\s*=/m);
    if (name && version && !source) {
      if (!result.has(name[1])) {
        result.set(name[1], []);
      }
      result.get(name[1]).push(version[1]);
    }
  }
  return result;
}

function assertCargoLockWorkspaceVersions(packages, workspaceVersion, lockPath) {
  const locked = lockedPackageVersions(lockPath);
  for (const pkg of packages) {
    const versions = locked.get(pkg.name) || [];
    if (versions.length !== 1 || versions[0] !== workspaceVersion) {
      throw new Error(
        `Cargo.lock must contain exactly one workspace package ${pkg.name} at ${workspaceVersion}; found ${versions.join(", ") || "none"}.`
      );
    }
  }
}

function checkVersionSync(repoRoot = REPO_ROOT, manifest = packageJson) {
  const cargoTomlPath = path.join(repoRoot, "Cargo.toml");
  const workspaceVersion = rustWorkspaceVersion(cargoTomlPath);
  if (manifest.version !== workspaceVersion) {
    throw new Error(
      `package.json version (${manifest.version}) does not match Cargo workspace version (${workspaceVersion}).`
    );
  }

  const packages = workspacePackages(repoRoot);
  assertWorkspaceManifestVersions(packages, workspaceVersion);
  assertInternalPathDependencyPins(packages, workspaceVersion);
  assertCargoLockWorkspaceVersions(packages, workspaceVersion, path.join(repoRoot, "Cargo.lock"));
  return true;
}

if (require.main === module) {
  try {
    checkVersionSync();
    console.log(
      "npm, Cargo workspace, internal path dependency, and Cargo.lock versions match."
    );
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}

module.exports = {
  assertCargoLockWorkspaceVersions,
  assertInternalPathDependencyPins,
  checkVersionSync,
  lockedPackageVersions,
  rustWorkspaceVersion,
  workspaceMemberPaths,
  workspacePackages
};
