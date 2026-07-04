"use strict";

const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");
const { runReleaseCheck } = require("./release-check");
const { runSelfTest: runSecuritySelfTest } = require("./security-check");

const REPO_ROOT = path.resolve(__dirname, "..");
const AXIOM_BINARY = process.platform === "win32" ? "axiom.exe" : "axiom";

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd || REPO_ROOT,
    env: options.env || process.env,
    encoding: "utf8",
    shell: false
  });
  const output = `${result.stdout || ""}${result.stderr || ""}`;
  if (result.status !== 0) {
    throw new Error(
      `Command failed: ${command} ${args.join(" ")}\nExit: ${result.status}\n${output}`
    );
  }
  return output;
}

function ensureAxiomBinary() {
  if (process.env.AXIOM_E2E_BINARY) {
    assert(fs.existsSync(process.env.AXIOM_E2E_BINARY), "AXIOM_E2E_BINARY does not exist");
    return process.env.AXIOM_E2E_BINARY;
  }

  const debugBinary = path.join(REPO_ROOT, "target", "debug", AXIOM_BINARY);
  run("cargo", ["build", "-p", "axiom-cli"]);
  assert(fs.existsSync(debugBinary), "Axiom debug binary was not built");
  return debugBinary;
}

function runAxiom(binary, args, env, cwd = REPO_ROOT) {
  return run(binary, args, { env, cwd });
}

function assertHelpIncludes(binary, env) {
  const help = runAxiom(binary, ["--help"], env);
  for (const command of [
    "doctor",
    "config",
    "onboarding",
    "chat",
    "run",
    "code",
    "proof",
    "skill",
    "update"
  ]) {
    assert(help.includes(command), `help output is missing ${command}`);
  }
}

function main() {
  const binary = ensureAxiomBinary();
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "axiom-e2e-"));
  const axiomHome = path.join(tempRoot, "home");
  const workspace = path.join(tempRoot, "workspace");
  const registry = path.join(REPO_ROOT, "fixtures", "skill-registry");
  fs.mkdirSync(workspace, { recursive: true });
  fs.writeFileSync(
    path.join(workspace, "README.md"),
    "# E2E Workspace\n\nThis file is read by the mock provider tool loop.\n"
  );

  const env = {
    ...process.env,
    AXIOM_HOME: axiomHome,
    NO_COLOR: "1"
  };

  try {
    assertHelpIncludes(binary, env);

    const onboarding = runAxiom(binary, [
      "onboarding",
      "--non-interactive",
      "--provider",
      "mock",
      "--workspace",
      workspace,
      "--registry",
      registry,
      "--yes"
    ], env);
    assert(onboarding.includes("Saved config:"), "onboarding did not save config");
    assert(fs.existsSync(path.join(axiomHome, "config.toml")), "config was not written to AXIOM_HOME");
    assert(
      fs.existsSync(path.join(axiomHome, "skills", "installed_skills.json")),
      "essential skills were not installed under AXIOM_HOME"
    );

    assert(runAxiom(binary, ["doctor"], env).includes(axiomHome), "doctor did not use AXIOM_HOME");
    assert(runAxiom(binary, ["skill", "installed"], env).includes("file.read"));
    const health = runAxiom(binary, ["skill", "health"], env);
    assert(health.includes("Skill health:"), "skill health heading was missing");
    assert(health.includes("file.read"), "skill health did not list installed skills");
    assert(
      runAxiom(binary, ["skill", "run", "project.scan", "--args", "{\"path\":\".\",\"max_depth\":2}"], env)
        .includes("README.md"),
      "project.scan did not see workspace README.md"
    );

    const chatRun = runAxiom(binary, ["run", "read README.md and summarize it"], env);
    assert(chatRun.includes("Axiom Lens: selected"), "axiom run did not report Skill Lens selection");
    assert(chatRun.includes("Axiom Tool: executed file.read"), "axiom run did not execute file.read");
    assert(chatRun.includes("Tool result received"), "mock provider did not return final tool summary");

    const plan = runAxiom(binary, ["code", "--plan-only", "create a demo plan"], env);
    assert(plan.includes("Plan:"), "coder plan-only did not print a plan");
    assert(plan.includes("Inspect the workspace"), "mock coder plan was not used");

    const proofs = runAxiom(binary, ["proof", "list"], env);
    assert(proofs.includes("chat") || proofs.includes("coder"), "proof list did not include recorded traces");
    assert(fs.existsSync(path.join(axiomHome, "proofs")), "proofs were not stored under AXIOM_HOME");

    assert(runAxiom(binary, ["update", "status"], env).includes("Axiom update status"));
    assert(!fs.existsSync(path.join(workspace, "config.toml")), "config leaked into workspace root");
    assert(fs.existsSync(axiomHome), "AXIOM_HOME was not created");
    assert(fs.existsSync(workspace), "workspace was not created");

    runReleaseCheck();
    runSecuritySelfTest();
    console.log("Axiom E2E tests passed.");
  } finally {
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  main
};
