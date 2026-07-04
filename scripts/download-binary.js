"use strict";

const fs = require("fs");
const https = require("https");
const path = require("path");
const { verifyChecksum } = require("./verify-checksum");

function normalizeReleaseRepo(repo) {
  return String(repo || "")
    .trim()
    .replace(/\.git$/, "")
    .replace(/\/+$/, "");
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

function releaseAssetUrl(repo, version, assetName) {
  return `${normalizeReleaseRepo(repo)}/releases/download/v${version}/${encodeURIComponent(assetName)}`;
}

function releaseChecksumUrl(repo, version) {
  return `${normalizeReleaseRepo(repo)}/releases/download/v${version}/SHA256SUMS`;
}

function downloadToFile(url, destination) {
  return new Promise((resolve, reject) => {
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    const file = fs.createWriteStream(destination);

    const request = https.get(url, { headers: { "User-Agent": "axiom-agent-installer" } }, (response) => {
      if (
        response.statusCode >= 300 &&
        response.statusCode < 400 &&
        response.headers.location
      ) {
        file.close();
        fs.rmSync(destination, { force: true });
        downloadToFile(response.headers.location, destination).then(resolve, reject);
        return;
      }

      if (response.statusCode === 404) {
        file.close();
        fs.rmSync(destination, { force: true });
        reject(
          new Error(
            "Prebuilt binary not found for this version/platform. If developing locally, set AXIOM_AGENT_BINARY_PATH to your built binary."
          )
        );
        return;
      }

      if (response.statusCode < 200 || response.statusCode >= 300) {
        file.close();
        fs.rmSync(destination, { force: true });
        reject(new Error(`Download failed with HTTP ${response.statusCode}: ${url}`));
        return;
      }

      response.pipe(file);
      file.on("finish", () => {
        file.close(resolve);
      });
    });

    request.on("error", (error) => {
      file.close();
      fs.rmSync(destination, { force: true });
      reject(error);
    });
  });
}

function downloadText(url) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "axiom-agent-installer" } }, (response) => {
        if (
          response.statusCode >= 300 &&
          response.statusCode < 400 &&
          response.headers.location
        ) {
          downloadText(response.headers.location).then(resolve, reject);
          return;
        }

        if (response.statusCode < 200 || response.statusCode >= 300) {
          reject(new Error(`Download failed with HTTP ${response.statusCode}: ${url}`));
          response.resume();
          return;
        }

        response.setEncoding("utf8");
        let data = "";
        response.on("data", (chunk) => {
          data += chunk;
        });
        response.on("end", () => resolve(data));
      })
      .on("error", reject);
  });
}

function makeExecutable(filePath, platform = process.platform) {
  if (platform !== "win32") {
    fs.chmodSync(filePath, 0o755);
  }
}

async function downloadAndVerifyBinary(options) {
  const { repo, version, assetName, destination, platform } = options;
  const assetUrl = releaseAssetUrl(repo, version, assetName);
  const checksumUrl = releaseChecksumUrl(repo, version);
  const tempDestination = `${destination}.download`;

  fs.rmSync(tempDestination, { force: true });
  await downloadToFile(assetUrl, tempDestination);
  const checksumText = await downloadText(checksumUrl);
  verifyChecksum(tempDestination, checksumText, assetName);
  fs.renameSync(tempDestination, destination);
  makeExecutable(destination, platform);
  return destination;
}

module.exports = {
  downloadAndVerifyBinary,
  downloadText,
  downloadToFile,
  makeExecutable,
  normalizeReleaseRepo,
  releaseAssetUrl,
  releaseChecksumUrl,
  releaseRepoFromPackage
};
