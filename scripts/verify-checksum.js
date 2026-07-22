"use strict";

const crypto = require("crypto");
const fs = require("fs");

function sha256Buffer(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function sha256File(filePath) {
  return sha256Buffer(fs.readFileSync(filePath));
}

function parseChecksumFile(text) {
  const checksums = new Map();

  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) {
      continue;
    }

    const match = line.match(/^([a-fA-F0-9]{64})\s+\*?(.+)$/);
    if (!match) {
      throw new Error(`Malformed SHA256SUMS line: ${line}`);
    }

    const assetName = match[2].trim();
    if (!assetName || checksums.has(assetName)) {
      throw new Error(`Duplicate or empty SHA256SUMS entry: ${assetName}`);
    }
    checksums.set(assetName, match[1].toLowerCase());
  }

  return checksums;
}

function expectedChecksumForAsset(checksumText, assetName) {
  const checksums = parseChecksumFile(checksumText);
  return checksums.get(assetName) || null;
}

function verifyChecksum(filePath, checksumText, assetName) {
  const expected = expectedChecksumForAsset(checksumText, assetName);
  if (!expected) {
    throw new Error(`SHA256SUMS does not contain an entry for ${assetName}`);
  }

  const actual = sha256File(filePath);
  if (actual !== expected) {
    throw new Error(`Checksum mismatch for ${assetName}`);
  }

  return true;
}

module.exports = {
  expectedChecksumForAsset,
  parseChecksumFile,
  sha256Buffer,
  sha256File,
  verifyChecksum
};
