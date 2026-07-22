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
const OFFICIAL_REGISTRY_URL = "https://raw.githubusercontent.com/NexaraAI/axiom-skills/main/registry.json";
const CONFIG_SOURCE = fs.readFileSync(
  path.join(REPO_ROOT, "crates", "axiom-core", "src", "config.rs"),
  "utf8"
);
const CONFIG_VERSION_MATCH = CONFIG_SOURCE.match(/CURRENT_CONFIG_VERSION:\s*u32\s*=\s*(\d+)/);
assert(CONFIG_VERSION_MATCH, "could not determine CURRENT_CONFIG_VERSION");
const CURRENT_CONFIG_VERSION = Number(CONFIG_VERSION_MATCH[1]);

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd || REPO_ROOT,
    env: options.env || process.env,
    encoding: "utf8",
    shell: false,
    input: options.input
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
  run("cargo", ["build", "-p", "axiom-cli", "--locked"]);
  assert(fs.existsSync(debugBinary), "Axiom debug binary was not built");
  return debugBinary;
}

function runAxiom(binary, args, env, cwd, input) {
  // A copied executable must be able to run without its build checkout as the
  // working directory. The optional override is only for an explicit test
  // case; normal E2E commands execute from the isolated temporary root.
  return run(binary, args, { env, cwd: cwd || env.AXIOM_E2E_CWD || REPO_ROOT, input });
}

function assertHelpIncludes(binary, env) {
  const help = runAxiom(binary, ["--help"], env);
  for (const command of [
    "doctor",
    "config",
    "onboarding",
    "chat",
    "resume",
    "sessions",
    "cost",
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
  const sourceBinary = ensureAxiomBinary();
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "axiom-e2e-"));
  const relocatedDir = path.join(tempRoot, "relocated-bin");
  fs.mkdirSync(relocatedDir, { recursive: true });
  const binary = path.join(relocatedDir, AXIOM_BINARY);
  fs.copyFileSync(sourceBinary, binary);
  if (process.platform !== "win32") {
    fs.chmodSync(binary, 0o755);
  }
  const axiomHome = path.join(tempRoot, "home");
  const workspace = path.join(tempRoot, "workspace");
  fs.mkdirSync(workspace, { recursive: true });
  fs.writeFileSync(
    path.join(workspace, "README.md"),
    "# E2E Workspace\n\nThis file is read by the mock provider tool loop.\n"
  );

  const env = {
    ...process.env,
    AXIOM_HOME: axiomHome,
    AXIOM_E2E_CWD: tempRoot,
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
      "--yes"
    ], env, tempRoot);
    assert(onboarding.includes("Saved config:"), "onboarding did not save config");
    assert(fs.existsSync(path.join(axiomHome, "config.toml")), "config was not written to AXIOM_HOME");
    const freshConfig = fs.readFileSync(path.join(axiomHome, "config.toml"), "utf8");
    assert(
      freshConfig.includes(`config_version = ${CURRENT_CONFIG_VERSION}`),
      "onboarding did not create a current config schema"
    );
    assert(
      freshConfig.includes(`registry_url = "${OFFICIAL_REGISTRY_URL}"`),
      "relocated binary did not retain the official configured registry"
    );
    assert(
      !freshConfig.includes("fixtures/skill-registry") && !freshConfig.includes("bundled-registry"),
      "relocated binary persisted an internal or source-checkout registry path"
    );
    const bundledRegistryRoot = path.join(axiomHome, "bundled-registry");
    const registryGenerations = fs.readdirSync(bundledRegistryRoot).filter((entry) =>
      fs.existsSync(path.join(bundledRegistryRoot, entry, "registry.json")) &&
      fs.existsSync(path.join(bundledRegistryRoot, entry, ".complete"))
    );
    assert(
      registryGenerations.length === 1,
      "relocated binary did not materialize its embedded starter registry"
    );
    assert(
      fs.existsSync(path.join(axiomHome, "skills", "installed_skills.json")),
      "essential skills were not installed under AXIOM_HOME"
    );

    assert(runAxiom(binary, ["doctor"], env).includes(axiomHome), "doctor did not use AXIOM_HOME");
    const doctorJson = JSON.parse(runAxiom(binary, ["doctor", "--json"], env));
    assert.strictEqual(
      doctorJson.config_schema_version,
      CURRENT_CONFIG_VERSION,
      "doctor did not report config schema"
    );
    assert.strictEqual(doctorJson.config_migration_required, false, "fresh config requires migration");
    assert(runAxiom(binary, ["provider", "list"], env).includes("mock"), "provider list omitted mock");
    const modelCatalog = runAxiom(binary, ["model", "list"], env);
    assert(modelCatalog.includes("mock-model"), "model catalog omitted mock-model");
    assert(modelCatalog.includes("models: 1"), "model catalog count was incorrect");
    runAxiom(binary, ["model", "use", "mock-model"], env);
    assert(
      runAxiom(binary, ["model", "current"], env).includes("model: mock-model"),
      "model selection did not persist"
    );
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
    assert(chatRun.includes("Result verified and summarized."), "offline provider did not return final tool summary");
    assert(chatRun.includes("model calls"), "axiom run did not print runtime status");
    const cost = runAxiom(binary, ["cost"], env);
    assert(cost.includes("Axiom cost ledger"), "cost report heading was missing");
    assert(cost.includes("budget enforcement") && cost.includes("unavailable"), "unknown pricing was not reported safely");
    const sessions = runAxiom(binary, ["sessions"], env);
    const sessionId = sessions.match(/session-[0-9a-f]+-[0-9a-f]+/)?.[0];
    assert(sessionId, "saved session was not listed");
    assert(
      fs.existsSync(path.join(axiomHome, "sessions", `${sessionId}.json`)),
      "session state was not persisted atomically"
    );
    const resumed = runAxiom(binary, ["resume", sessionId], env);
    assert(resumed.includes(`session: ${sessionId}`), "resume did not load the saved session");

    const multiline = runAxiom(
      binary,
      ["chat"],
      env,
      tempRoot,
      "!multi\nhello from\nmultiple lines\n!send\n!exit\n"
    );
    assert(multiline.includes("Multiline mode:"), "chat did not enter multiline mode");
    assert(
      multiline.includes("Axiom (offline): hello from\nmultiple lines"),
      "chat did not submit the multiline prompt as one turn"
    );
    const inlineModels = runAxiom(binary, ["chat"], env, tempRoot, "!model list\n!exit\n");
    assert(inlineModels.includes("mock-model"), "inline model catalog command failed");

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

    const legacyHome = path.join(tempRoot, "legacy-home");
    fs.mkdirSync(legacyHome, { recursive: true });
    fs.writeFileSync(
      path.join(legacyHome, "config.toml"),
      `[agent]\nname = "Axiom Agent"\nchannel = "stable"\nfirst_run_completed = true\ndefault_workspace = "${workspace.replace(/\\/g, "\\\\")}"\nauto_update_policy = "notify"\n\n[llm]\nactive_provider = "mock"\nactive_model = "mock-model"\nstream = false\n\n[skills]\nauto_update_policy = "notify"\nlocal_dir = "skills"\n\n[coder]\napproval_mode = "safe"\nworkspace_only = true\nallow_shell = true\nmax_file_read_bytes = 2000000\n\n[proof]\nenabled = false\nformat = "json"\n`
    );
    const legacyEnv = { ...env, AXIOM_HOME: legacyHome };
    const migrate = runAxiom(binary, ["config", "migrate"], legacyEnv);
    assert(
      migrate.includes(`Migrated config schema v0 to v${CURRENT_CONFIG_VERSION}`),
      "legacy config did not migrate"
    );
    assert(fs.existsSync(path.join(legacyHome, "config.toml.v0.bak")), "migration backup missing");

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
