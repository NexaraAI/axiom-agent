"use strict";

const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

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

function runSelfTest() {
  const unsafeKey = ["OPENAI_API_KEY", "=", "sk-" + "1234567890abcdef1234567890abcdef"].join("");
  const safeDocs = [
    ["OPENAI_API_KEY", "=", "YOUR_OPENAI_API_KEY"].join(""),
    ["Authorization:", "Bearer", "<token>"].join(" "),
    "Use sk-placeholder in examples only."
  ].join(os.EOL);

  assert(scanText(unsafeKey, "temp/config.txt").length > 0);
  assert.strictEqual(scanText(safeDocs, "docs/example.md").length, 0);
}

function main(argv = process.argv.slice(2)) {
  if (argv.includes("--self-test")) {
    runSelfTest();
    console.log("Security check self-test passed.");
    return true;
  }

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
  main,
  projectFiles,
  runSelfTest,
  scanFiles,
  scanText
};
