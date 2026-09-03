"use strict";

const crypto = require("crypto");
const fs = require("fs");

function sha256Buffer(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function sha256File(filePath, maxBytes = 512 * 1024 * 1024) {

  const fd = fs.openSync(filePath, "r");
  try {
    const hash = crypto.createHash("sha256");
    const buf = Buffer.allocUnsafe(64 * 1024);
    let seen = 0;
    for (;;) {
      const n = fs.readSync(fd, buf, 0, buf.length, null);
      if (n === 0) break;
      seen += n;
      if (seen > maxBytes) {
        throw new Error(`File exceeds ${maxBytes} byte verification limit: ${filePath}`);
      }
      hash.update(n === buf.length ? buf : buf.subarray(0, n));
    }
    return hash.digest("hex");
  } finally {
    fs.closeSync(fd);
  }
}

function sha256FileAsync(filePath, maxBytes = 512 * 1024 * 1024) {

  return new Promise((resolve, reject) => {
    const hash = crypto.createHash("sha256");
    let seen = 0;
    const stream = fs.createReadStream(filePath, { highWaterMark: 64 * 1024 });
    stream.on("data", (chunk) => {
      seen += chunk.length;
      if (seen > maxBytes) {
        stream.destroy();
        reject(new Error(`File exceeds ${maxBytes} byte verification limit: ${filePath}`));
        return;
      }
      hash.update(chunk);
    });
    stream.on("end", () => resolve(hash.digest("hex")));
    stream.on("error", reject);
  });
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
  sha256FileAsync,
  verifyChecksum
};
