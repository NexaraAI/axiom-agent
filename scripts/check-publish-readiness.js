"use strict";

const assert = require("assert");
const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");
const packageJson = require("../package.json");
const { assertDistTagForVersion } = require("./check-dist-tag");

const REPO_ROOT = path.resolve(__dirname, "..");
const REQUIRED_RELEASE_ASSETS = Object.freeze([
  "axiom-x86_64-pc-windows-msvc.exe",
  "axiom-x86_64-unknown-linux-gnu",
  "axiom-x86_64-apple-darwin",
  "axiom-aarch64-apple-darwin",
  "SHA256SUMS"
]);

function assertExactChangelogHeading(content, version) {
  const expected = `## ${version}`;
  if (!content.split(/\r?\n/).some((line) => line === expected)) {
    throw new Error(`CHANGELOG.md must contain the exact heading: ${expected}`);
  }
  return true;
}

function assertPackagePublishPolicy(manifest = packageJson) {
  if (!manifest.publishConfig || manifest.publishConfig.access !== "public") {
    throw new Error("package.json publishConfig.access must be public.");
  }
  if (!manifest.publishConfig.tag) {
    throw new Error("package.json publishConfig.tag is required.");
  }
  assertDistTagForVersion(manifest.version, manifest.publishConfig.tag);
  return true;
}

function assertReleaseMetadata(metadata, version) {
  const expectedTag = `v${version}`;
  if (!metadata || metadata.tagName !== expectedTag) {
    throw new Error(
      `GitHub Release tag must be exactly ${expectedTag}; found ${metadata && metadata.tagName}.`
    );
  }
  if (metadata.isDraft) {
    throw new Error(`GitHub Release ${expectedTag} is still a draft.`);
  }

  const names = new Set((metadata.assets || []).map((asset) => asset && asset.name));
  const missing = REQUIRED_RELEASE_ASSETS.filter((asset) => !names.has(asset));
  if (missing.length > 0) {
    throw new Error(
      `GitHub Release ${expectedTag} is missing required assets: ${missing.join(", ")}`
    );
  }
  return true;
}

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

function assertVersionIsUnpublished(manifest = packageJson, options = {}) {
  const invocation = options.invocation || npmInvocation(options.env);
  const result = (options.spawnSync || spawnSync)(
    invocation.command,
    [...invocation.prefix, "view", `${manifest.name}@${manifest.version}`, "version", "--json"],
    {
      cwd: options.cwd || REPO_ROOT,
      encoding: "utf8",
      env: options.env || process.env,
      shell: invocation.shell,
      timeout: 60_000
    }
  );

  if (result.error) {
    throw new Error(`Could not query npm for an existing version: ${result.error.message}`);
  }
  if (result.status === 0) {
    throw new Error(
      `${manifest.name}@${manifest.version} is already published; npm versions are immutable. Cut a unique prerelease version before publishing again.`
    );
  }

  const output = `${result.stdout || ""}\n${result.stderr || ""}`;
  if (!/\bE404\b|404\s+Not\s+Found|is not in this registry/i.test(output)) {
    throw new Error(
      `npm availability check failed without a definitive not-found response:\n${output.trim()}`
    );
  }
  return true;
}

function parseArgs(argv) {
  const options = {
    assertUnpublished: false,
    distTag: null,
    releaseJson: null,
    selfTest: false,
    tag: null
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--assert-unpublished") {
      options.assertUnpublished = true;
    } else if (argument === "--self-test") {
      options.selfTest = true;
    } else if (["--dist-tag", "--release-json", "--tag"].includes(argument)) {
      const value = argv[index + 1];
      if (!value) {
        throw new Error(`${argument} requires a value.`);
      }
      options[argument === "--dist-tag" ? "distTag" : argument === "--tag" ? "tag" : "releaseJson"] = value;
      index += 1;
    } else {
      throw new Error(`Unknown argument: ${argument}`);
    }
  }
  return options;
}

function runSelfTest() {
  const version = "1.0.0-rc.1";
  const metadata = {
    assets: REQUIRED_RELEASE_ASSETS.map((name) => ({ name })),
    isDraft: false,
    tagName: `v${version}`
  };
  assert.strictEqual(assertExactChangelogHeading(`## Unreleased\n\n## ${version}\n`, version), true);
  assert.throws(
    () => assertExactChangelogHeading(`## ${version} - today\n`, version),
    /exact heading/
  );
  assert.strictEqual(assertReleaseMetadata(metadata, version), true);
  assert.throws(
    () => assertReleaseMetadata({ ...metadata, assets: [] }, version),
    /missing required assets/
  );
  assert.throws(
    () => assertReleaseMetadata({ ...metadata, tagName: "v1.0.0" }, version),
    /must be exactly/
  );
  assert.strictEqual(
    assertVersionIsUnpublished(
      { name: "axiom-agent", version },
      {
        invocation: { command: "npm", prefix: [], shell: false },
        spawnSync: () => ({ status: 1, stderr: "npm error code E404", stdout: "" })
      }
    ),
    true
  );
  assert.throws(
    () =>
      assertVersionIsUnpublished(
        { name: "axiom-agent", version },
        {
          invocation: { command: "npm", prefix: [], shell: false },
          spawnSync: () => ({ status: 0, stderr: "", stdout: `"${version}"` })
        }
      ),
    /already published/
  );
}

function main(argv = process.argv.slice(2)) {
  const options = parseArgs(argv);
  if (options.selfTest) {
    runSelfTest();
    console.log("npm publish-readiness self-test passed.");
    return true;
  }

  assertPackagePublishPolicy(packageJson);
  const expectedTag = `v${packageJson.version}`;
  if (options.tag !== expectedTag) {
    throw new Error(`Release tag must be exactly ${expectedTag}; found ${options.tag || "none"}.`);
  }
  if (options.distTag) {
    assertDistTagForVersion(packageJson.version, options.distTag);
  }
  const changelog = fs.readFileSync(path.join(REPO_ROOT, "CHANGELOG.md"), "utf8");
  assertExactChangelogHeading(changelog, packageJson.version);
  if (!options.releaseJson) {
    throw new Error("--release-json is required to bind npm publishing to a GitHub Release.");
  }
  const metadata = JSON.parse(fs.readFileSync(options.releaseJson, "utf8"));
  assertReleaseMetadata(metadata, packageJson.version);
  if (options.assertUnpublished) {
    assertVersionIsUnpublished(packageJson);
  }
  console.log(
    `npm publish readiness verified for ${packageJson.name}@${packageJson.version} from ${expectedTag}.`
  );
  return true;
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}

module.exports = {
  REQUIRED_RELEASE_ASSETS,
  assertExactChangelogHeading,
  assertPackagePublishPolicy,
  assertReleaseMetadata,
  assertVersionIsUnpublished,
  main,
  npmInvocation,
  runSelfTest
};
