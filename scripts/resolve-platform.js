"use strict";

class UnsupportedPlatformError extends Error {
  constructor(platform, arch) {
    super(`Unsupported platform: ${platform}/${arch}`);
    this.name = "UnsupportedPlatformError";
    this.platform = platform;
    this.arch = arch;
  }
}

function resolvePlatform(platform = process.platform, arch = process.arch) {
  if (platform === "win32" && arch === "x64") {
    return {
      platform,
      arch,
      target: "x86_64-pc-windows-msvc",
      assetName: "axiom-x86_64-pc-windows-msvc.exe",
      binaryName: "axiom.exe"
    };
  }

  if (platform === "linux" && arch === "x64") {
    return {
      platform,
      arch,
      target: "x86_64-unknown-linux-gnu",
      assetName: "axiom-x86_64-unknown-linux-gnu",
      binaryName: "axiom"
    };
  }

  if (platform === "darwin" && arch === "x64") {
    return {
      platform,
      arch,
      target: "x86_64-apple-darwin",
      assetName: "axiom-x86_64-apple-darwin",
      binaryName: "axiom"
    };
  }

  if (platform === "darwin" && arch === "arm64") {
    return {
      platform,
      arch,
      target: "aarch64-apple-darwin",
      assetName: "axiom-aarch64-apple-darwin",
      binaryName: "axiom"
    };
  }

  throw new UnsupportedPlatformError(platform, arch);
}

module.exports = {
  UnsupportedPlatformError,
  resolvePlatform
};
