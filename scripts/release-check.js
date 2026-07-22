"use strict";

const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");
const packageJson = require("../package.json");
const { runSelfTest: runDistTagPolicySelfTest } = require("./check-dist-tag");
const { checkVersionSync } = require("./check-version-sync");
const {
  assertExactChangelogHeading,
  assertPackagePublishPolicy,
  runSelfTest: runPublishReadinessSelfTest
} = require("./check-publish-readiness");
const {
  MAX_BINARY_BYTES,
  MAX_CHECKSUM_BYTES,
  MAX_REDIRECTS,
  runSelfTest: runDownloadSecuritySelfTest
} = require("./download-binary");

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

    if (/\.(exe|pdb|dll|so|dylib)$/i.test(normalized)) {
      forbidden.push(file);
    }
  }

  if (forbidden.length > 0) {
    fail(`Forbidden tracked release artifacts found:\n${forbidden.join("\n")}`);
  }
}

function assertPackageMetadata() {
  checkVersionSync();
  assertPackagePublishPolicy(packageJson);

  if (!packageJson.engines || packageJson.engines.node !== ">=20") {
    fail("package.json engines.node must declare the supported minimum as >=20.");
  }
  if (
    !packageJson.scripts ||
    packageJson.scripts.prepublishOnly !== "node scripts/check-dist-tag.js --from-npm"
  ) {
    fail("package.json must enforce the version/dist-tag policy through prepublishOnly.");
  }
  if (packageJson.scripts["packed-smoke"] !== "node scripts/packed-install-smoke.js") {
    fail("package.json must expose the packed-tarball install smoke test.");
  }

  const repository = packageJson.repository && packageJson.repository.url;
  if (!repository || !repository.includes("github.com/NexaraAI/axiom-agent")) {
    fail("package.json repository must point to NexaraAI/axiom-agent.");
  }

  const releaseRepo = packageJson.axiomAgent && packageJson.axiomAgent.releaseRepo;
  if (!releaseRepo || !releaseRepo.includes("github.com/NexaraAI/axiom-agent")) {
    fail("package.json axiomAgent.releaseRepo must point to NexaraAI/axiom-agent.");
  }

  const changelog = fs.readFileSync(path.join(REPO_ROOT, "CHANGELOG.md"), "utf8");
  assertExactChangelogHeading(changelog, packageJson.version);
}

function assertGeneratedCompilerArtifactsIgnored() {
  const ignore = fs.readFileSync(path.join(REPO_ROOT, ".gitignore"), "utf8");
  for (const artifact of ["/rust_out.exe", "/rust_out.pdb"]) {
    if (!ignore.split(/\r?\n/).includes(artifact)) {
      fail(`.gitignore must explicitly exclude root compiler artifact ${artifact}.`);
    }
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
  if (!readme.includes(packageJson.version.toLowerCase())) {
    fail(`README.md must identify the current workspace/package version (${packageJson.version}).`);
  }
  if (packageJson.version.includes("-beta") && !readme.includes("npm install -g axiom-agent@beta")) {
    fail("README.md must retain the published beta installation command while the package version is beta.");
  }
  if (packageJson.version.includes("-rc") && !readme.includes("npm install -g axiom-agent@rc")) {
    fail("README.md must document the rc installation command for an RC package version.");
  }
}

function assertConfigSchemaDocumentation() {
  const configSource = fs.readFileSync(
    path.join(REPO_ROOT, "crates", "axiom-core", "src", "config.rs"),
    "utf8"
  );
  const versionMatch = configSource.match(/CURRENT_CONFIG_VERSION:\s*u32\s*=\s*(\d+)/);
  if (!versionMatch) {
    fail("Could not determine CURRENT_CONFIG_VERSION from axiom-core.");
  }

  const currentVersion = versionMatch[1];
  const installation = fs.readFileSync(path.join(REPO_ROOT, "docs", "INSTALLATION.md"), "utf8");
  const e2e = fs.readFileSync(path.join(REPO_ROOT, "scripts", "e2e-test.js"), "utf8");
  if (!installation.includes("config_version") || !installation.toLowerCase().includes("current schema")) {
    fail(`INSTALLATION.md must explain the current config schema (currently v${currentVersion}).`);
  }
  if (!e2e.includes("CURRENT_CONFIG_VERSION") || !e2e.includes("CONFIG_VERSION_MATCH")) {
    fail("E2E onboarding must derive its expected config schema from axiom-core.");
  }
}

function assertCostBudgetContract() {
  expectFile("crates/axiom-cli/src/cost_commands.rs");
  const documents = ["README.md", "docs/INSTALLATION.md", "docs/PRD.md", "docs/V1_RC_CHECKLIST.md"];
  for (const document of documents) {
    const content = fs.readFileSync(path.join(REPO_ROOT, document), "utf8");
    if (!content.includes("axiom cost")) {
      fail(`${document} must document the persistent cost report.`);
    }
  }
  const readmeAndInstall = documents
    .slice(0, 2)
    .map((document) => fs.readFileSync(path.join(REPO_ROOT, document), "utf8"))
    .join("\n");
  for (const field of [
    "session_budget_usd",
    "monthly_budget_usd",
    "input_cost_per_million_tokens",
    "output_cost_per_million_tokens",
    "cost-ledger.json"
  ]) {
    if (!readmeAndInstall.includes(field)) {
      fail(`cost budget documentation is missing ${field}.`);
    }
  }
  const e2e = fs.readFileSync(path.join(REPO_ROOT, "scripts", "e2e-test.js"), "utf8");
  if (!e2e.includes('runAxiom(binary, ["cost"], env)')) {
    fail("E2E must exercise the cost-ledger status command.");
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
  expectFile("docs/V1_PLAN.md");
  expectFile("docs/V1_RC_CHECKLIST.md");
  expectFile("docs/THREAT_MODEL.md");
  expectFile("scripts/check-dist-tag.js");
  expectFile("scripts/check-publish-readiness.js");
  expectFile("scripts/packed-install-smoke.js");
  expectFile("CHANGELOG.md");
  expectFile("SECURITY.md");
  expectFile("CONTRIBUTING.md");
  expectFile("rust-toolchain.toml");
  expectFile("deny.toml");
  expectFile(".github/dependabot.yml");
}

function assertInstallerDownloadPolicy() {
  const source = fs.readFileSync(path.join(REPO_ROOT, "scripts", "download-binary.js"), "utf8");
  runDownloadSecuritySelfTest();
  if (MAX_REDIRECTS > 5 || MAX_BINARY_BYTES > 256 * 1024 * 1024 || MAX_CHECKSUM_BYTES > 1024 * 1024) {
    fail("installer download redirect/size limits exceed the reviewed release policy.");
  }
  for (const requirement of [
    "TRUSTED_DOWNLOAD_HOSTS",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
    "parsed.protocol !== \"https:\"",
    "request.setTimeout(60_000",
    "byteLimitTransform(MAX_BINARY_BYTES",
    "assertDeclaredSize(response, MAX_CHECKSUM_BYTES",
    "received > MAX_CHECKSUM_BYTES",
    "flags: \"wx\"",
    "replaceInstalledFile",
    ".previous-"
  ]) {
    if (!source.includes(requirement)) {
      fail(`installer download policy is missing: ${requirement}`);
    }
  }
}

function assertDependencySecurityPolicy() {
  const workflow = fs.readFileSync(
    path.join(REPO_ROOT, ".github", "workflows", "ci.yml"),
    "utf8"
  );
  const denyConfig = fs.readFileSync(path.join(REPO_ROOT, "deny.toml"), "utf8");
  const dependabot = fs.readFileSync(
    path.join(REPO_ROOT, ".github", "dependabot.yml"),
    "utf8"
  );

  for (const command of [
    "cargo metadata --locked --format-version 1",
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features --locked -- -D warnings",
    "cargo test --workspace --all-features --locked"
  ]) {
    if (!workflow.includes(command)) {
      fail(`CI is missing the full locked-workspace gate: ${command}`);
    }
  }
  if (!workflow.includes('node-version: 20') && !workflow.includes('node-version: "20"')) {
    fail("CI must exercise the npm wrapper on the declared minimum Node 20 runtime.");
  }
  if (!workflow.includes("npm run packed-smoke")) {
    fail("CI must install and invoke the packed npm tarball on the minimum Node runtime.");
  }
  if (!workflow.includes("EmbarkStudios/cargo-deny-action@")) {
    fail("CI must run cargo-deny for dependency policy enforcement.");
  }
  for (const runner of ["ubuntu-latest", "windows-latest", "macos-latest"]) {
    if (!workflow.includes(runner)) {
      fail(`CI test matrix is missing ${runner}.`);
    }
  }
  for (const section of ["[advisories]", "[licenses]", "[bans]", "[sources]"]) {
    if (!denyConfig.includes(section)) {
      fail(`deny.toml is missing required policy section: ${section}`);
    }
  }
  for (const ecosystem of ["cargo", "npm", "github-actions"]) {
    if (!dependabot.includes(`package-ecosystem: ${ecosystem}`)) {
      fail(`Dependabot is missing the ${ecosystem} ecosystem.`);
    }
  }
}

function assertWorkflowActionsAreShaPinned() {
  const workflowDir = path.join(REPO_ROOT, ".github", "workflows");
  for (const file of fs.readdirSync(workflowDir)) {
    if (!file.endsWith(".yml") && !file.endsWith(".yaml")) {
      continue;
    }
    const content = fs.readFileSync(path.join(workflowDir, file), "utf8");
    for (const match of content.matchAll(/^\s*-\s+uses:\s+([^\s#]+)(?:\s+#.*)?\s*$/gm)) {
      const action = match[1];
      if (!/@[0-9a-f]{40}$/i.test(action)) {
        fail(`${file} uses an action that is not pinned to a full commit SHA: ${action}`);
      }
    }
  }
}

function assertReleaseProvenance() {
  const workflow = fs.readFileSync(
    path.join(REPO_ROOT, ".github", "workflows", "release.yml"),
    "utf8"
  );
  for (const permission of ["id-token: write", "attestations: write", "artifact-metadata: write"]) {
    if (!workflow.includes(permission)) {
      fail(`release workflow is missing required attestation permission: ${permission}`);
    }
  }
  if (!workflow.includes("anchore/sbom-action@") || !workflow.includes("actions/attest@")) {
    fail("release workflow must generate an SBOM and attest release artifacts.");
  }
  if (!workflow.includes("axiom-agent.spdx.json")) {
    fail("release workflow must publish the SPDX JSON SBOM.");
  }
  const validateStart = workflow.indexOf("  validate:");
  const buildStart = workflow.indexOf("\n  build:", validateStart);
  const releaseStart = workflow.indexOf("\n  release:", buildStart);
  if (validateStart < 0 || buildStart < 0 || releaseStart < 0) {
    fail("release workflow must define validate, build, and release jobs in order.");
  }
  const validateJob = workflow.slice(validateStart, buildStart);
  const buildJob = workflow.slice(buildStart, releaseStart);
  const releaseJob = workflow.slice(releaseStart);

  for (const gate of [
    "npm run check-version-sync",
    "cargo metadata --locked --format-version 1",
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features --locked -- -D warnings",
    "cargo test --workspace --all-features --locked",
    "EmbarkStudios/cargo-deny-action@bb137d7af7e4fb67e5f82a49c4fce4fad40782fe",
    "node scripts/smoke-test.js",
    "node scripts/e2e-test.js",
    "node scripts/release-check.js",
    "node scripts/security-check.js",
    "node scripts/check-dist-tag.js --self-test",
    "node scripts/check-publish-readiness.js --self-test",
    "npm run packed-smoke",
    "npm pack --dry-run",
    "Verify tag and changelog version",
  ]) {
    if (!validateJob.includes(gate)) {
      fail(`release validation job is missing mandatory gate: ${gate}`);
    }
  }
  if (!validateJob.includes('grep -Fx "## ${package_version}" CHANGELOG.md')) {
    fail("release validation must require an exact changelog heading match.");
  }
  if (!/build:\s+name: Build[^\n]*\s+runs-on:[^\n]*\s+needs: validate\s+strategy:/m.test(workflow)) {
    fail("release build job must depend on validation before defining its matrix.");
  }
  for (const [runner, target] of [
    ["windows-latest", "x86_64-pc-windows-msvc"],
    ["ubuntu-22.04", "x86_64-unknown-linux-gnu"],
    ["macos-15-intel", "x86_64-apple-darwin"],
    ["macos-15", "aarch64-apple-darwin"]
  ]) {
    const nativePair = new RegExp(`- os: ${runner}\\s+target: ${target}`);
    if (!nativePair.test(buildJob)) {
      fail(`release workflow must build ${target} on native runner ${runner}.`);
    }
  }
  for (const smokeRequirement of [
    "cargo build -p axiom-cli --release --locked --target ${{ matrix.target }}",
    "actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020",
    "AXIOM_E2E_BINARY:",
    "node scripts/e2e-test.js"
  ]) {
    if (!buildJob.includes(smokeRequirement)) {
      fail(`release build job is missing native binary smoke coverage: ${smokeRequirement}`);
    }
  }
  if (buildJob.indexOf("node scripts/e2e-test.js") > buildJob.indexOf("actions/upload-artifact@")) {
    fail("native release binary smoke tests must pass before artifacts are uploaded.");
  }
  if (!releaseJob.includes("prerelease: ${{ contains(github.ref_name, '-') }}")) {
    fail("hyphenated release versions must be marked as GitHub prereleases.");
  }
  if (!releaseJob.includes("make_latest: ${{ contains(github.ref_name, '-') && 'false' || 'true' }}")) {
    fail("GitHub prereleases must not be promoted as the latest release.");
  }
}

function assertNpmTrustedPublishing() {
  const workflow = fs.readFileSync(
    path.join(REPO_ROOT, ".github", "workflows", "npm-publish.yml"),
    "utf8"
  );
  if (workflow.includes("NPM_TOKEN") || workflow.includes("NODE_AUTH_TOKEN")) {
    fail("npm publish workflow must not use a long-lived publish token.");
  }
  if (!workflow.includes("id-token: write")) {
    fail("npm publish workflow must request an OIDC identity token.");
  }
  if (!workflow.includes('node-version: "24"') || !workflow.includes("npm@11.5.1")) {
    fail("npm publish workflow must pin a trusted-publishing-compatible Node/npm toolchain.");
  }
  if (!workflow.includes("dist-tag:") || !workflow.includes('default: "beta"')) {
    fail("npm publish workflow must require an explicit dist-tag and default to beta.");
  }
  if (!workflow.includes('npm publish --tag "$NPM_DIST_TAG"')) {
    fail("npm publish must use the validated beta/rc/latest dist-tag.");
  }
  const smokeStart = workflow.indexOf("  smoke:");
  const publishStart = workflow.indexOf("\n  publish:", smokeStart);
  if (smokeStart < 0 || publishStart < 0) {
    fail("npm workflow must define smoke and publish jobs.");
  }
  const smokeJob = workflow.slice(smokeStart, publishStart);
  const publishJob = workflow.slice(publishStart);

  if (!workflow.includes("release-tag:")) {
    fail("npm workflow dispatch must require an existing GitHub Release tag.");
  }

  for (const requirement of [
    "ref: ${{ github.event.release.tag_name || inputs['release-tag'] }}",
    'expected_tag="v${package_version}"',
    'test "$RELEASE_TAG" = "$expected_tag"',
    'git rev-list -n 1 "$RELEASE_TAG"',
    "gh release view \"$RELEASE_TAG\" --repo NexaraAI/axiom-agent",
    "--json tagName,isDraft,isPrerelease,assets",
    "node scripts/check-publish-readiness.js",
    "npm run packed-smoke"
  ]) {
    if (!smokeJob.includes(requirement)) {
      fail(`npm validation job is missing release-bound gate: ${requirement}`);
    }
  }

  for (const requirement of [
    "environment:",
    "name: npm-publish",
    "ref: ${{ needs.smoke.outputs.release_tag }}",
    "gh release view \"$RELEASE_TAG\" --repo NexaraAI/axiom-agent",
    "node scripts/check-publish-readiness.js",
    "--assert-unpublished",
    "npm run packed-smoke",
    'npm publish --tag "$NPM_DIST_TAG"'
  ]) {
    if (!publishJob.includes(requirement)) {
      fail(`npm publish job is missing mandatory guard: ${requirement}`);
    }
  }

  const publishIndex = publishJob.indexOf('npm publish --tag "$NPM_DIST_TAG"');
  const unpublishedIndex = publishJob.indexOf("--assert-unpublished");
  if (publishIndex < 0 || unpublishedIndex < 0 || unpublishedIndex > publishIndex) {
    fail("the immutable-version and release-asset guard must run immediately before npm publish.");
  }
  if (!workflow.includes("cancel-in-progress: false")) {
    fail("concurrent npm publication attempts for one release must be serialized.");
  }
  runDistTagPolicySelfTest();
  runPublishReadinessSelfTest();
}

function runReleaseCheck() {
  assertPackageMetadata();
  assertGeneratedCompilerArtifactsIgnored();
  assertDefaultRegistry();
  assertNoForbiddenTrackedFiles(trackedFiles());
  assertReadmeStatus();
  assertConfigSchemaDocumentation();
  assertCostBudgetContract();
  assertReleaseFiles();
  assertInstallerDownloadPolicy();
  assertWorkflowActionsAreShaPinned();
  assertReleaseProvenance();
  assertNpmTrustedPublishing();
  assertDependencySecurityPolicy();
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
