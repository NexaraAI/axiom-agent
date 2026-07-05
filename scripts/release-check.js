"use strict";

const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");
const packageJson = require("../package.json");
const { checkVersionSync } = require("./check-version-sync");

const REPO_ROOT = path.resolve(__dirname, "..");

function fail(message) {
  throw new Error(message);
}

function expectFile(relativePath, message = `${relativePath} is missing.`) {
  if (!fs.existsSync(path.join(REPO_ROOT, relativePath))) {
    fail(message);
  }
}

function trackedFiles() {
  const result = spawnSync("git", ["ls-files"], {
    cwd: REPO_ROOT,
    encoding: "utf8",
    shell: false
  });
  if (result.status !== 0) {
    return null;
  }
  return result.stdout
    .split(/\r?\n/)
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function assertNoForbiddenTrackedFiles(files) {
  if (!files) {
    console.log("git is not available; skipped tracked-file checks.");
    return;
  }

  const forbidden = [];
  for (const file of files) {
    const normalized = file.replace(/\\/g, "/");
    if (
      normalized.startsWith("target/") ||
      normalized.includes("/target/") ||
      normalized.startsWith("node_modules/") ||
      normalized.includes("/node_modules/") ||
      normalized === ".env" ||
      normalized.endsWith("/.env") ||
      normalized.startsWith("proofs/") ||
      normalized.includes("/proofs/") ||
      normalized.startsWith("vendor/bin/") ||
      normalized.includes("/vendor/bin/")
    ) {
      forbidden.push(file);
      continue;
    }

    if (/\.(exe|dll|so|dylib)$/i.test(normalized)) {
      forbidden.push(file);
    }
  }

  if (forbidden.length > 0) {
    fail(`Forbidden tracked release artifacts found:\n${forbidden.join("\n")}`);
  }
}

function assertPackageMetadata() {
  checkVersionSync();

  const repository = packageJson.repository && packageJson.repository.url;
  if (!repository || !repository.includes("github.com/NexaraAI/axiom-agent")) {
    fail("package.json repository must point to NexaraAI/axiom-agent.");
  }

  const releaseRepo = packageJson.axiomAgent && packageJson.axiomAgent.releaseRepo;
  if (!releaseRepo || !releaseRepo.includes("github.com/NexaraAI/axiom-agent")) {
    fail("package.json axiomAgent.releaseRepo must point to NexaraAI/axiom-agent.");
  }
}

function assertDefaultRegistry() {
  const config = fs.readFileSync(
    path.join(REPO_ROOT, "crates", "axiom-core", "src", "config.rs"),
    "utf8"
  );
  if (!config.includes("NexaraAI/axiom-skills")) {
    fail("Default skills registry must point to NexaraAI/axiom-skills.");
  }
}

function assertReadmeStatus() {
  const readme = fs.readFileSync(path.join(REPO_ROOT, "README.md"), "utf8").toLowerCase();
  if (readme.includes("not published yet")) {
    fail("README.md still says npm is not published, but axiom-agent@beta is live on npm.");
  }
}

function assertReleaseFiles() {
  expectFile(".github/workflows/ci.yml");
  expectFile(".github/workflows/release.yml");
  expectFile(".github/workflows/npm-publish.yml");
  expectFile("LICENSE");
  expectFile("docs/INSTALLATION.md");
  expectFile("docs/ARCHITECTURE.md");
  expectFile("docs/RELEASE.md");
  expectFile("docs/TESTING.md");
  expectFile("docs/DEMO.md");
  expectFile("docs/V0_5_BETA_RELEASE_CHECKLIST.md");
}

function runReleaseCheck() {
  assertPackageMetadata();
  assertDefaultRegistry();
  assertNoForbiddenTrackedFiles(trackedFiles());
  assertReadmeStatus();
  assertReleaseFiles();
  return true;
}

function main() {
  try {
    runReleaseCheck();
    console.log("Release check passed.");
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  runReleaseCheck,
  trackedFiles
};
