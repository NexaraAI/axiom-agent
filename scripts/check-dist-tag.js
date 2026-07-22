"use strict";

const assert = require("assert");
const packageJson = require("../package.json");

const SEMVER_PATTERN =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
const ALLOWED_DIST_TAGS = new Set(["beta", "rc", "latest"]);

function releaseChannel(version) {
  const match = SEMVER_PATTERN.exec(version);
  if (!match) {
    throw new Error(`Invalid semantic version: ${version}`);
  }
  return match[4] ? match[4].split(".")[0].toLowerCase() : "stable";
}

function assertDistTagForVersion(version, distTag) {
  if (!ALLOWED_DIST_TAGS.has(distTag)) {
    throw new Error(`Unsupported npm dist-tag: ${distTag}`);
  }

  const channel = releaseChannel(version);
  const expectedTag = channel === "stable" ? "latest" : channel;
  if (expectedTag !== distTag || !ALLOWED_DIST_TAGS.has(expectedTag)) {
    throw new Error(
      `npm dist-tag ${distTag} does not match package version ${version}; expected ${expectedTag}`
    );
  }
  return true;
}

function runSelfTest() {
  assert.strictEqual(assertDistTagForVersion("1.0.0", "latest"), true);
  assert.strictEqual(assertDistTagForVersion("1.0.0-rc.1", "rc"), true);
  assert.strictEqual(assertDistTagForVersion("1.0.0-beta", "beta"), true);
  assert.strictEqual(assertDistTagForVersion("1.0.0-beta.2+build.7", "beta"), true);
  assert.throws(() => assertDistTagForVersion("1.0.0-rc.1", "latest"), /does not match/);
  assert.throws(() => assertDistTagForVersion("1.0.0-beta", "rc"), /does not match/);
  assert.throws(() => assertDistTagForVersion("1.0.0", "beta"), /does not match/);
  assert.throws(() => assertDistTagForVersion("1.0.0-alpha.1", "rc"), /expected alpha/);
  assert.throws(() => assertDistTagForVersion("v1.0.0", "latest"), /Invalid semantic version/);
}

function requestedNpmDistTag(env = process.env, manifest = packageJson) {
  const requested = env.npm_config_tag || env.NPM_CONFIG_TAG;
  const configured = manifest.publishConfig && manifest.publishConfig.tag;
  const distTag = requested || configured;
  if (!distTag) {
    throw new Error(
      "npm publish must provide a dist-tag or package.json publishConfig.tag."
    );
  }
  return distTag;
}

function main(argv = process.argv.slice(2)) {
  if (argv.includes("--self-test")) {
    runSelfTest();
    console.log("npm dist-tag policy self-test passed.");
    return true;
  }
  if (argv.length === 1 && argv[0] === "--from-npm") {
    const distTag = requestedNpmDistTag();
    assertDistTagForVersion(packageJson.version, distTag);
    console.log(
      `npm publish guard accepted ${packageJson.version} for dist-tag ${distTag}.`
    );
    return true;
  }
  if (argv.length !== 2) {
    throw new Error("Usage: node scripts/check-dist-tag.js <version> <beta|rc|latest>");
  }
  assertDistTagForVersion(argv[0], argv[1]);
  console.log(`npm dist-tag ${argv[1]} matches package version ${argv[0]}.`);
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
  assertDistTagForVersion,
  main,
  releaseChannel,
  requestedNpmDistTag,
  runSelfTest
};
