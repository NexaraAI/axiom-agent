"use strict";

const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");
const {
  MAX_BINARY_BYTES,
  MAX_CHECKSUM_BYTES,
  MAX_REDIRECTS,
  runSelfTest: runDownloadSecuritySelfTest
} = require("./download-binary");

const REPO_ROOT = path.resolve(__dirname, "..");

const SECRET_PATTERNS = [
  {
    name: "OPENAI_API_KEY assignment",
    regex: /\bOPENAI_API_KEY\s*=\s*["']?[^"'\s]+/i
  },
  {
    name: "CLOUDFLARE_API_TOKEN assignment",
    regex: /\bCLOUDFLARE_API_TOKEN\s*=\s*["']?[^"'\s]+/i
  },
  {
    name: "Authorization bearer token",
    regex: /\bAuthorization:\s*Bearer\s+[A-Za-z0-9._~+/=-]+/i
  },
  {
    name: "OpenAI style secret key",
    regex: /\bsk-[A-Za-z0-9_-]{16,}/
  },
  {
    name: "GitHub personal access token",
    regex: /\bgh[po]_[A-Za-z0-9_]{16,}/
  },
  {
    name: "private key block",
    regex: /BEGIN\s+PRIVATE\s+KEY/
  }
];

function isSafePlaceholder(line, filePath) {
  const lower = line.toLowerCase();
  const normalizedPath = filePath.replace(/\\/g, "/");
  const isDocs = filePath
    .replace(/\\/g, "/")
    .split("/")
    .some((part) => part === "docs" || part === "README.md");

  if (
    lower.includes("your_") ||
    lower.includes("your-") ||
    lower.includes("<token>") ||
    lower.includes("<api") ||
    lower.includes("placeholder") ||
    lower.includes("example") ||
    lower.includes("not a real") ||
    lower.includes("not-real") ||
    lower.includes("dummy") ||
    lower.includes("[redacted]") ||
    lower.includes("sk-test") ||
    lower.includes("abc123")
  ) {
    return true;
  }

  if (
    (normalizedPath === "crates/axiom-proof/src/recorder.rs" &&
      lower.includes("sk-proof-secret")) ||
    (normalizedPath === "crates/axiom-proof/src/redaction.rs" &&
      lower.includes(["sk-", "abcdefghijklmnopqrstuvwxyz"].join("")))
  ) {
    return true;
  }

  if (isDocs && (lower.includes("fake") || lower.includes("sample"))) {
    return true;
  }

  return false;
}

function scanText(content, filePath = "unknown") {
  const findings = [];
  const lines = content.split(/\r?\n/);

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith("//")) {
      continue;
    }
    if (isSafePlaceholder(line, filePath)) {
      continue;
    }

    if (path.basename(filePath) === ".env") {
      findings.push({
        file: filePath,
        line: index + 1,
        kind: ".env content"
      });
      continue;
    }

    for (const pattern of SECRET_PATTERNS) {
      if (pattern.regex.test(line)) {
        findings.push({
          file: filePath,
          line: index + 1,
          kind: pattern.name
        });
      }
    }
  }

  return findings;
}

function gitTrackedFiles(cwd = REPO_ROOT) {
  const result = spawnSync("git", ["ls-files"], {
    cwd,
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

function walkFiles(dir, root = dir, files = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    const rel = path.relative(root, fullPath).replace(/\\/g, "/");
    if (
      entry.isDirectory() &&
      [".git", "target", "node_modules", "vendor"].includes(entry.name)
    ) {
      continue;
    }
    if (entry.isDirectory()) {
      walkFiles(fullPath, root, files);
    } else if (entry.isFile()) {
      files.push(rel);
    }
  }
  return files;
}

function isProbablyText(filePath) {
  const extension = path.extname(filePath).toLowerCase();
  return ![
    ".exe",
    ".dll",
    ".so",
    ".dylib",
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".webp",
    ".ico",
    ".pdf",
    ".zip",
    ".gz",
    ".tgz",
    ".br"
  ].includes(extension);
}

function projectFiles(cwd = REPO_ROOT) {
  return gitTrackedFiles(cwd) || walkFiles(cwd);
}

function scanFiles(files, cwd = REPO_ROOT) {
  const findings = [];
  for (const file of files) {
    if (!isProbablyText(file)) {
      continue;
    }
    const fullPath = path.join(cwd, file);
    if (!fs.existsSync(fullPath)) {
      continue;
    }
    const content = fs.readFileSync(fullPath, "utf8");
    findings.push(...scanText(content, file));
  }
  return findings;
}

function assertWorkflowSecurity(cwd = REPO_ROOT) {
  const workflowDir = path.join(cwd, ".github", "workflows");
  for (const file of fs.readdirSync(workflowDir)) {
    if (!file.endsWith(".yml") && !file.endsWith(".yaml")) {
      continue;
    }
    const content = fs.readFileSync(path.join(workflowDir, file), "utf8");
    const jobsIndex = content.search(/^jobs:/m);
    const workflowHeader = jobsIndex >= 0 ? content.slice(0, jobsIndex) : content;
    if (!/^permissions:\r?\n  contents: read\s*$/m.test(workflowHeader)) {
      throw new Error(`${file} must default to read-only repository contents permissions.`);
    }
    if (/permissions:\s*write-all/i.test(content)) {
      throw new Error(`${file} must not grant write-all permissions.`);
    }
    const contentWriteGrants = content.match(/^\s+contents: write\s*$/gm) || [];
    if (file === "release.yml") {
      if (contentWriteGrants.length !== 1) {
        throw new Error("release.yml must scope its single contents:write grant to the release job.");
      }
    } else if (contentWriteGrants.length > 0) {
      throw new Error(`${file} must not grant contents:write.`);
    }
    if (file === "ci.yml" && content.includes("id-token: write")) {
      throw new Error("ci.yml must not request an OIDC identity token.");
    }
    if (content.includes("NPM_TOKEN") || content.includes("NODE_AUTH_TOKEN")) {
      throw new Error(`${file} must not use a long-lived npm publishing token.`);
    }
    for (const match of content.matchAll(/^\s*-\s+uses:\s+([^\s#]+)(?:\s+#.*)?\s*$/gm)) {
      if (!/@[0-9a-f]{40}$/i.test(match[1])) {
        throw new Error(`${file} contains a non-SHA-pinned action: ${match[1]}`);
      }
    }
  }

  const npmWorkflow = fs.readFileSync(path.join(workflowDir, "npm-publish.yml"), "utf8");
  if ((npmWorkflow.match(/^\s+id-token: write\s*$/gm) || []).length !== 1) {
    throw new Error("npm trusted publishing must scope one OIDC grant to its publish job.");
  }
  for (const requirement of [
    "name: npm-publish",
    "ref: ${{ needs.smoke.outputs.release_tag }}",
    "gh release view \"$RELEASE_TAG\" --repo NexaraAI/axiom-agent",
    "--assert-unpublished",
    'npm publish --tag "$NPM_DIST_TAG"'
  ]) {
    if (!npmWorkflow.includes(requirement)) {
      throw new Error(`npm trusted publishing is missing fail-closed guard: ${requirement}`);
    }
  }
  const releaseWorkflow = fs.readFileSync(path.join(workflowDir, "release.yml"), "utf8");
  if ((releaseWorkflow.match(/^\s+id-token: write\s*$/gm) || []).length !== 1) {
    throw new Error("binary release attestation must scope one OIDC grant to its release job.");
  }
}

function assertInstallerDownloadSecurity(cwd = REPO_ROOT) {
  const source = fs.readFileSync(path.join(cwd, "scripts", "download-binary.js"), "utf8");
  runDownloadSecuritySelfTest();
  assert(MAX_REDIRECTS <= 5, "installer redirects must remain bounded");
  assert(MAX_BINARY_BYTES <= 256 * 1024 * 1024, "binary downloads must remain bounded");
  assert(MAX_CHECKSUM_BYTES <= 1024 * 1024, "checksum downloads must remain bounded");
  for (const requirement of [
    "TRUSTED_DOWNLOAD_HOSTS",
    "parsed.protocol !== \"https:\"",
    "nextRedirectUrl",
    "request.setTimeout(60_000",
    "byteLimitTransform(MAX_BINARY_BYTES",
    "received > MAX_CHECKSUM_BYTES",
    "replaceInstalledFile",
    ".previous-"
  ]) {
    assert(source.includes(requirement), `installer download security is missing: ${requirement}`);
  }
}

function runSelfTest() {
  const unsafeKey = ["OPENAI_API_KEY", "=", "sk-" + "1234567890abcdef1234567890abcdef"].join("");
  const safeDocs = [
    ["OPENAI_API_KEY", "=", "YOUR_OPENAI_API_KEY"].join(""),
    ["Authorization:", "Bearer", "<token>"].join(" "),
    "Use sk-placeholder in examples only."
  ].join(os.EOL);

  assert(scanText(unsafeKey, "temp/config.txt").length > 0);
  assert.strictEqual(scanText(safeDocs, "docs/example.md").length, 0);
  assertInstallerDownloadSecurity(REPO_ROOT);
  assertWorkflowSecurity(REPO_ROOT);
}

function main(argv = process.argv.slice(2)) {
  if (argv.includes("--self-test")) {
    runSelfTest();
    console.log("Security check self-test passed.");
    return true;
  }

  assertInstallerDownloadSecurity(REPO_ROOT);
  assertWorkflowSecurity(REPO_ROOT);

  const findings = scanFiles(projectFiles(REPO_ROOT), REPO_ROOT);
  if (findings.length > 0) {
    console.error("Potential secrets found:");
    for (const finding of findings) {
      console.error(`- ${finding.file}:${finding.line} ${finding.kind}`);
    }
    process.exit(1);
  }

  console.log("Security check passed.");
  return true;
}

if (require.main === module) {
  main();
}

module.exports = {
  assertInstallerDownloadSecurity,
  assertWorkflowSecurity,
  main,
  projectFiles,
  runSelfTest,
  scanFiles,
  scanText
};
