use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "axiom", version, about = "Axiom Agent terminal CLI")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    /// Diagnose your setup: config, workspace, provider, credentials, policy
    Doctor(DoctorCommand),

    /// Show or migrate the config file
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// Run or update terminal onboarding (`setup` works too).
    #[command(alias = "setup")]
    Onboarding(OnboardingCommand),

    /// Start an interactive chat session with the agent
    Chat(ChatCommand),

    /// Continue a previous conversation by session id (see `axiom sessions`)
    Resume {
        /// Session id, as listed by `axiom sessions`
        session_id: String,
    },

    /// List past chat sessions you can resume
    Sessions,

    /// Show token and dollar cost tracked for this month
    Cost,

    /// Inspect or switch the active LLM model (alias: models)
    #[command(alias = "models")]
    Model {
        #[command(subcommand)]
        command: ModelCommands,
    },

    /// Inspect, switch, or add LLM providers (alias: providers)
    #[command(alias = "providers")]
    Provider {
        #[command(subcommand)]
        command: ProviderCommands,
    },

    /// Send one message to the agent and print the result, then exit
    Run(RunCommand),

    /// Guided coding session: scan, plan, patch, and test a task
    Code(CodeCommand),

    /// Browse and verify the recorded audit trail for agent actions
    Proof {
        #[command(subcommand)]
        command: ProofCommands,
    },

    /// Install, update, and manage the agent's skills
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },

    /// Check for, install, or roll back Axiom updates
    Update {
        #[command(subcommand)]
        command: UpdateCommands,
    },

    /// Manage the Telegram/Discord messaging gateway (tokens, status).
    Gateway {
        #[command(subcommand)]
        command: GatewayCommands,
    },

    /// Connect external MCP servers, or expose Axiom's tools over MCP.
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },

    /// Remove Axiom (binary via npm, local data on request).
    Uninstall(UninstallCommand),
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProofCommands {
    /// List recorded proof traces, newest first
    List,

    /// Show the most recent proof trace
    Latest,

    /// Print one proof trace
    Show {
        /// Id as shown by `axiom proof list`
        proof_id: String,
    },

    /// Save a proof trace as markdown or JSON
    Export {
        /// Id as shown by `axiom proof list`
        proof_id: String,
        /// Output format: markdown (default) or json
        #[arg(long, default_value = "markdown")]
        format: String,
    },

    /// Open a proof trace in the default viewer
    Open {
        /// Id as shown by `axiom proof list`
        proof_id: String,
    },

    /// Delete proof traces older than the given number of days
    Clean {
        /// Delete traces older than this many days
        #[arg(long = "older-than")]
        older_than: u64,
    },
}

#[derive(Debug, Args)]
pub(crate) struct CodeCommand {
    /// Produce a plan without applying anything
    #[arg(long)]
    pub(crate) plan_only: bool,

    /// Print a project overview: languages, entry points, layout
    #[arg(long)]
    pub(crate) scan: bool,

    /// Print the current uncommitted git diff
    #[arg(long)]
    pub(crate) diff: bool,

    /// Plan and apply a task in one step (skips plan review)
    #[arg(long)]
    pub(crate) apply: bool,

    /// Auto-detect and run the workspace test suite
    #[arg(long = "test")]
    pub(crate) test: bool,

    /// Explain what this project does and how it is structured
    #[arg(long)]
    pub(crate) explain: bool,

    /// The coding task (omit to enter interactive mode)
    #[arg(value_name = "TASK", trailing_var_arg = true)]
    pub(crate) task: Vec<String>,
}

#[derive(Debug, Default, Args)]
pub(crate) struct ChatCommand {
    /// Directory for agent file, shell, git, and test operations.
    ///
    /// Overrides `agent.default_workspace` for this session only; use `/workspace`
    /// inside the session to change it persistently.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Use the classic inline prompt instead of the full-screen TUI.
    #[arg(long)]
    pub(crate) inline: bool,
}

#[derive(Debug, Default, Args)]
pub(crate) struct OnboardingCommand {
    /// Answer prompts from flags instead of interactively
    #[arg(long)]
    pub(crate) non_interactive: bool,

    /// Workspace directory for agent file operations
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Provider id to configure (for example groq, openrouter, mock)
    #[arg(long)]
    pub(crate) provider: Option<String>,

    /// Model id to activate for the provider
    #[arg(long)]
    pub(crate) model: Option<String>,

    /// Cloudflare account id (Cloudflare AI Gateway only)
    #[arg(long)]
    pub(crate) account_id: Option<String>,

    /// Skill registry URL to use instead of the default
    #[arg(long)]
    pub(crate) registry: Option<String>,

    /// Skip provider setup (offline/mock usage)
    #[arg(long)]
    pub(crate) skip_provider: bool,

    /// Accept defaults without prompting
    #[arg(long)]
    pub(crate) yes: bool,
}

#[derive(Debug, Args)]
pub(crate) struct RunCommand {
    /// The message to send to the agent
    pub(crate) message: String,

    /// Disable tool use; the model can only answer with text
    #[arg(long = "no-tools")]
    pub(crate) no_tools: bool,

    /// Skip writing the proof trace for this run
    #[arg(long = "no-proof")]
    pub(crate) no_proof: bool,

    /// Use this configured provider for the run
    #[arg(long)]
    pub(crate) provider: Option<String>,

    /// Use this model for the run
    #[arg(long)]
    pub(crate) model: Option<String>,
}

#[derive(Debug, Default, Args)]
pub(crate) struct DoctorCommand {
    /// Print machine-readable output for scripts
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommands {
    /// Print the current config file contents
    List,

    /// Print the config file location without reading it
    Path,

    /// Upgrade an older config file to the current schema (backs it up first)
    Migrate,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ModelCommands {
    /// Show the active provider and model
    Current,

    /// List models offered by the provider catalog
    List {
        /// Query a specific provider instead of the active one
        #[arg(long)]
        provider: Option<String>,

        /// Only show models whose id contains this text
        #[arg(long)]
        filter: Option<String>,
    },

    /// Switch the active model (fuzzy-matches; use --force to skip checks)
    Use {
        /// Model id (partial ids are resolved against the catalog)
        model: String,
        /// Switch this provider first
        #[arg(long)]
        provider: Option<String>,
        /// Accept an unknown model id without catalog verification
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProviderCommands {
    /// Show the active provider
    Current,

    /// List configured providers and their models
    List,

    /// Switch the active provider (optionally setting a model too)
    Use {
        /// Provider name as shown by `axiom provider list`
        provider: String,

        /// Also switch to this model on the provider
        #[arg(long)]
        model: Option<String>,
    },

    /// Configure an additional provider (interactive when --name is omitted)
    Add {
        /// Provider id or preset name (for example groq, openrouter)
        #[arg(long)]
        name: Option<String>,

        /// Default model id for the provider
        #[arg(long)]
        model: Option<String>,

        /// API key to store in the OS credential manager
        #[arg(long)]
        api_key: Option<String>,

        /// OpenAI-compatible endpoint base URL (custom providers)
        #[arg(long)]
        base_url: Option<String>,

        /// Make this the active provider after configuring it
        #[arg(long)]
        activate: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum SkillCommands {
    /// Show, set, or refresh the skill registry
    Registry {
        #[command(subcommand)]
        command: SkillRegistryCommands,
    },

    /// List skills available in the registry
    List,

    /// Search registry skills by name or description
    Search {
        /// Search text
        query: String,
    },

    /// List installed skills and their health
    Installed,

    /// List installable skill bundles
    Bundles,

    /// Show one skill's manifest, permissions, and stats
    Info {
        /// Skill id, as shown by `axiom skill list`
        skill_id: String,
    },

    /// Run one skill directly with a JSON arguments blob
    Run {
        /// Skill id to execute
        skill_id: String,
        /// Arguments as JSON, for example '{"path":"README.md"}'
        #[arg(long)]
        args: Option<String>,
    },

    /// Install a skill from the registry
    Install {
        /// Skill id to install
        skill_id: String,
        /// Registry URL to fetch from
        #[arg(long)]
        registry: Option<String>,
        /// Install from a registry directory on disk instead
        #[arg(long = "from-local-registry")]
        from_local_registry: Option<PathBuf>,
    },

    /// Install a bundle of skills from the registry
    InstallBundle {
        /// Bundle id to install
        bundle_id: String,
        /// Registry URL to fetch from
        #[arg(long)]
        registry: Option<String>,
        /// Install from a registry directory on disk instead
        #[arg(long = "from-local-registry")]
        from_local_registry: Option<PathBuf>,
    },

    /// Check for skill updates, optionally applying them
    Update {
        /// Only report available updates
        #[arg(long)]
        check: bool,
        /// Update every installed skill
        #[arg(long)]
        all: bool,
        /// Apply patch-version updates during the check
        #[arg(long = "apply-patches")]
        apply_patches: bool,
        /// Restrict to one skill
        skill_id: Option<String>,
    },

    /// Report the health of installed skills
    Health,

    /// Re-enable a disabled skill
    Enable {
        /// Skill id to enable
        skill_id: String,
    },

    /// Disable a skill without uninstalling it
    Disable {
        /// Skill id to disable
        skill_id: String,
    },

    /// Reset a skill's recorded execution statistics
    ResetStats {
        /// Skill id whose stats to reset
        skill_id: String,
    },

    /// Uninstall a skill and remove its files
    Remove {
        /// Skill id to remove
        skill_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum SkillRegistryCommands {
    /// Show the registry currently in use
    Current,

    /// Point Axiom at a different registry URL
    Set { url: String },

    /// Re-fetch the registry catalog
    Refresh,
}

#[derive(Debug, Subcommand)]
pub(crate) enum UpdateCommands {
    /// Show the current update channel and last installed version
    Status,

    /// Check for a newer release without installing it
    Check,

    /// Download and install the latest release
    Install,

    /// Revert to the backed-up previous binary
    Rollback,

    /// Switch release channel (stable, beta, nightly)
    SetChannel { channel: String },

    /// Set when updates are applied (notify, auto, off)
    SetPolicy { policy: String },
}

#[derive(Debug, Subcommand)]
pub(crate) enum McpCommands {
    /// Show the configured MCP servers without starting them.
    List,

    /// Connect the configured servers and list the tools they contribute.
    Tools {
        /// Inspect a single server, even when it is disabled in config.
        #[arg(long)]
        server: Option<String>,
        /// Also connect servers marked `enabled = false`.
        #[arg(long = "include-disabled")]
        include_disabled: bool,
    },

    /// Expose Axiom's tools to other MCP clients over stdio.
    Serve {
        /// Approve (`ask`) policy decisions instead of refusing them.
        #[arg(long)]
        approve: bool,
        /// Expose only tools that cannot mutate anything outside the workspace.
        #[arg(long = "read-only")]
        read_only: bool,
        /// Restrict the server to these tool ids (repeatable).
        #[arg(long = "allow", value_name = "TOOL")]
        allow: Vec<String>,
        /// Never expose these tool ids (repeatable).
        #[arg(long = "deny", value_name = "TOOL")]
        deny: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum GatewayCommands {
    /// Show gateway state: saved tokens, active provider/model the bots will use.
    Status,

    /// (Re)run the Telegram/Discord token setup.
    Setup,

    /// Forget saved gateway tokens (keep everything else).
    Disable {
        /// Only forget the Telegram token.
        #[arg(long)]
        telegram: bool,
        /// Only forget the Discord token.
        #[arg(long)]
        discord: bool,
    },

    /// Run the messaging bot (Telegram and Discord).
    Run {
        /// Run the Telegram bot.
        #[arg(long)]
        telegram: bool,
        /// Run the Discord bot.
        #[arg(long)]
        discord: bool,
    },
}

#[derive(Debug, Args)]
pub(crate) struct UninstallCommand {
    /// Also delete local data: config, skills, sessions, proofs, saved keys.
    #[arg(long = "delete-config")]
    pub(crate) delete_config: bool,

    /// Confirm without prompting.
    #[arg(long)]
    pub(crate) yes: bool,
}
