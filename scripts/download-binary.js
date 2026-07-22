"use strict";

const assert = require("assert");
const fs = require("fs");
const https = require("https");
const path = require("path");
const { Transform } = require("stream");
const { pipeline } = require("stream/promises");
const { verifyChecksum } = require("./verify-checksum");

const MAX_REDIRECTS = 5;
const MAX_BINARY_BYTES = 256 * 1024 * 1024;
const MAX_CHECKSUM_BYTES = 1024 * 1024;
const TRUSTED_DOWNLOAD_HOSTS = new Set([
  "github.com",
  "objects.githubusercontent.com",
  "release-assets.githubusercontent.com"
]);

function normalizeReleaseRepo(repo) {
  const normalized = String(repo || "")
    .trim()
    .replace(/^git\+/, "")
    .replace(/\.git$/, "")
    .replace(/\/+$/, "");
  if (!normalized) {
    return "";
  }

  let parsed;
  try {
    parsed = new URL(normalized);
  } catch (error) {
    throw new Error(`Axiom release repository URL is invalid: ${error.message}`);
  }
  const pathParts = parsed.pathname.split("/").filter(Boolean);
  if (
    parsed.protocol !== "https:" ||
    parsed.hostname.toLowerCase() !== "github.com" ||
    parsed.username ||
    parsed.password ||
    parsed.search ||
    parsed.hash ||
    pathParts.length !== 2 ||
    pathParts.some(
      (part) =>
        !/^[A-Za-z0-9_.-]+$/.test(part) || part === "." || part === ".."
    )
  ) {
    throw new Error(
      "Axiom release repository must be an HTTPS GitHub URL in the form https://github.com/owner/repository."
    );
  }
  return `https://github.com/${pathParts[0]}/${pathParts[1]}`;
}

function releaseRepoFromPackage(packageJson, env = process.env) {
  const configured =
    env.AXIOM_AGENT_RELEASE_REPO ||
    (packageJson.axiomAgent && packageJson.axiomAgent.releaseRepo) ||
    (packageJson.repository && packageJson.repository.url);
  const repo = normalizeReleaseRepo(configured);
  if (!repo) {
    throw new Error("Axiom release repository is not configured.");
  }
  return repo;
}

function safeAssetName(assetName) {
  const value = String(assetName || "");
  if (
    !value ||
    value.includes("/") ||
    value.includes("\\") ||
    path.basename(value) !== value ||
    value === "." ||
    value === ".."
  ) {
    throw new Error(`Unsafe Axiom release asset name: ${value}`);
  }
  return value;
}

function releaseAssetUrl(repo, version, assetName) {
  return `${normalizeReleaseRepo(repo)}/releases/download/v${encodeURIComponent(
    version
  )}/${encodeURIComponent(safeAssetName(assetName))}`;
}

function releaseChecksumUrl(repo, version) {
  return `${normalizeReleaseRepo(repo)}/releases/download/v${encodeURIComponent(
    version
  )}/SHA256SUMS`;
}

function parseDownloadUrl(value) {
  let parsed;
  try {
    parsed = new URL(value);
  } catch (error) {
    throw new Error(`Invalid Axiom download URL: ${error.message}`);
  }
  if (parsed.protocol !== "https:" || parsed.username || parsed.password) {
    throw new Error("Axiom downloads require HTTPS URLs without embedded credentials.");
  }
  if (!TRUSTED_DOWNLOAD_HOSTS.has(parsed.hostname.toLowerCase())) {
    throw new Error(`Axiom download host is not trusted: ${parsed.hostname}`);
  }
  return parsed;
}

function nextRedirectUrl(currentUrl, location, redirectCount) {
  if (redirectCount >= MAX_REDIRECTS) {
    throw new Error(`Axiom download exceeded ${MAX_REDIRECTS} redirects.`);
  }
  return parseDownloadUrl(new URL(location, currentUrl).toString());
}

function requestDownload(url, redirectCount = 0) {
  const parsed = parseDownloadUrl(url);
  return new Promise((resolve, reject) => {
    const request = https.get(
      parsed,
      { headers: { "User-Agent": "axiom-agent-installer" } },
      (response) => {
        if (
          response.statusCode >= 300 &&
          response.statusCode < 400 &&
          response.headers.location
        ) {
          let target;
          try {
            target = nextRedirectUrl(parsed, response.headers.location, redirectCount);
          } catch (error) {
            response.resume();
            reject(error);
            return;
          }
          response.resume();
          requestDownload(target, redirectCount + 1).then(resolve, reject);
          return;
        }
        resolve({ response, url: parsed.toString() });
      }
    );
    request.on("error", reject);
    request.setTimeout(60_000, () => {
      request.destroy(new Error("Axiom download timed out after 60 seconds."));
    });
  });
}

function assertSuccessfulResponse(response, url, notFoundMessage) {
  if (response.statusCode === 404 && notFoundMessage) {
    response.resume();
    throw new Error(notFoundMessage);
  }
  if (response.statusCode < 200 || response.statusCode >= 300) {
    response.resume();
    throw new Error(`Download failed with HTTP ${response.statusCode}: ${url}`);
  }
}

function assertDeclaredSize(response, limit, label) {
  const rawLength = response.headers["content-length"];
  if (rawLength === undefined) {
    return;
  }
  const length = Number(rawLength);
  if (!Number.isSafeInteger(length) || length < 0 || length > limit) {
    response.resume();
    throw new Error(`${label} exceeds the ${limit}-byte download limit.`);
  }
}

function byteLimitTransform(limit, label) {
  let received = 0;
  return new Transform({
    transform(chunk, _encoding, callback) {
      received += chunk.length;
      if (received > limit) {
        callback(new Error(`${label} exceeds the ${limit}-byte download limit.`));
      } else {
        callback(null, chunk);
      }
    }
  });
}

async function downloadToFile(url, destination) {
  const { response, url: finalUrl } = await requestDownload(url);
  assertSuccessfulResponse(
    response,
    finalUrl,
    "Prebuilt binary not found for this version/platform. If developing locally, set AXIOM_AGENT_BINARY_PATH to your built binary."
  );
  assertDeclaredSize(response, MAX_BINARY_BYTES, "Axiom binary");

  fs.mkdirSync(path.dirname(destination), { recursive: true });
  const file = fs.createWriteStream(destination, { flags: "wx", mode: 0o600 });
  try {
    await pipeline(response, byteLimitTransform(MAX_BINARY_BYTES, "Axiom binary"), file);
  } catch (error) {
    fs.rmSync(destination, { force: true });
    throw error;
  }
  return destination;
}

async function downloadText(url) {
  const { response, url: finalUrl } = await requestDownload(url);
  assertSuccessfulResponse(response, finalUrl);
  assertDeclaredSize(response, MAX_CHECKSUM_BYTES, "Axiom checksum file");

  const chunks = [];
  let received = 0;
  for await (const chunk of response) {
    received += chunk.length;
    if (received > MAX_CHECKSUM_BYTES) {
      response.destroy();
      throw new Error(
        `Axiom checksum file exceeds the ${MAX_CHECKSUM_BYTES}-byte download limit.`
      );
    }
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString("utf8");
}

function makeExecutable(filePath, platform = process.platform) {
  if (platform !== "win32") {
    fs.chmodSync(filePath, 0o755);
  }
}

function replaceInstalledFile(stagedPath, destination, finalize = () => {}) {
  const backupPath = `${destination}.previous-${process.pid}-${Date.now()}`;
  const hadExisting = fs.existsSync(destination);
  if (fs.existsSync(backupPath)) {
    throw new Error(`Axiom installer backup path already exists: ${backupPath}`);
  }

  if (hadExisting) {
    fs.renameSync(destination, backupPath);
  }

  try {
    fs.renameSync(stagedPath, destination);
    finalize(destination);
  } catch (error) {
    try {
      fs.rmSync(destination, { force: true });
      if (hadExisting) {
        fs.renameSync(backupPath, destination);
      }
    } catch (rollbackError) {
      throw new Error(
        `${error.message}; restoring the previous Axiom binary also failed: ${rollbackError.message}`
      );
    }
    throw error;
  }

  if (hadExisting) {
    fs.rmSync(backupPath, { force: true });
  }
  return destination;
}

async function downloadAndVerifyBinary(options) {
  const { repo, version, assetName, destination, platform } = options;
  const assetUrl = releaseAssetUrl(repo, version, assetName);
  const checksumUrl = releaseChecksumUrl(repo, version);
  const tempDestination = `${destination}.download-${process.pid}-${Date.now()}`;

  fs.rmSync(tempDestination, { force: true });
  try {
    await downloadToFile(assetUrl, tempDestination);
    const checksumText = await downloadText(checksumUrl);
    verifyChecksum(tempDestination, checksumText, safeAssetName(assetName));
    return replaceInstalledFile(tempDestination, destination, (installedPath) =>
      makeExecutable(installedPath, platform)
    );
  } finally {
    fs.rmSync(tempDestination, { force: true });
  }
}

function runSelfTest() {
  assert.strictEqual(
    normalizeReleaseRepo("git+https://github.com/NexaraAI/axiom-agent.git"),
    "https://github.com/NexaraAI/axiom-agent"
  );
  assert.throws(() => normalizeReleaseRepo("http://github.com/owner/repo"), /HTTPS GitHub URL/);
  assert.throws(() => normalizeReleaseRepo("https://github.com/owner/repo/extra"), /HTTPS GitHub URL/);
  assert.throws(() => safeAssetName("../axiom"), /Unsafe/);
  assert.throws(() => safeAssetName("..\\axiom.exe"), /Unsafe/);
  assert.throws(() => parseDownloadUrl("https://example.com/axiom"), /not trusted/);
  assert.strictEqual(
    nextRedirectUrl(
      new URL("https://github.com/owner/repo/releases/download/v1/axiom"),
      "https://release-assets.githubusercontent.com/asset",
      0
    ).hostname,
    "release-assets.githubusercontent.com"
  );
  assert.throws(
    () =>
      nextRedirectUrl(
        new URL("https://github.com/owner/repo/releases/download/v1/axiom"),
        "https://release-assets.githubusercontent.com/asset",
        MAX_REDIRECTS
      ),
    /exceeded/
  );
  return true;
}

module.exports = {
  MAX_BINARY_BYTES,
  MAX_CHECKSUM_BYTES,
  MAX_REDIRECTS,
  downloadAndVerifyBinary,
  downloadText,
  downloadToFile,
  makeExecutable,
  nextRedirectUrl,
  normalizeReleaseRepo,
  parseDownloadUrl,
  releaseAssetUrl,
  releaseChecksumUrl,
  releaseRepoFromPackage,
  replaceInstalledFile,
  runSelfTest,
  safeAssetName
};
