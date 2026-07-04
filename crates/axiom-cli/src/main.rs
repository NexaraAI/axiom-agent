mod chat;
mod code_commands;
mod onboarding;
mod proof_commands;
mod skill_commands;
mod startup;

use std::{path::PathBuf, process::Command};

use anyhow::Result;
use axiom_core::{AxiomConfig, Workspace};
use clap::{Args, Parser, Subcommand};
use startup::StartupRoute;

#[derive(Debug, Parser)]
#[command(name = "axiom", version, about = "Axiom Agent terminal CLI")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run local environment checks.
    Doctor,
    /// Read and write Axiom configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Run or update terminal onboarding.
    Onboarding,
    /// Open the terminal chat interface.
    Chat,
    /// Open Axiom Coder mode.
    Code(CodeCommand),
    /// Inspect Axiom proof traces and reports.
    Proof {
        #[command(subcommand)]
        command: ProofCommands,
    },
    /// Inspect and install Axiom skills.
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },
}

#[derive(Debug, Subcommand)]
enum ProofCommands {
    /// List proof traces.
    List,
    /// Show the latest proof summary.
    Latest,
    /// Show a proof report.
    Show { proof_id: String },
    /// Export a proof in a chosen format.
    Export {
        proof_id: String,
        #[arg(long, default_value = "markdown")]
        format: String,
    },
    /// Print the proof report path.
    Open { proof_id: String },
    /// Delete proof files older than DAYS.
    Clean {
        #[arg(long = "older-than")]
        older_than: u64,
    },
}

#[derive(Debug, Args)]
struct CodeCommand {
    /// Create a plan and stop before proposing edits.
    #[arg(long)]
    plan_only: bool,
    /// Scan the workspace and print a project summary.
    #[arg(long)]
    scan: bool,
    /// Show current git diff if this workspace is a git repository.
    #[arg(long)]
    diff: bool,
    /// Propose and apply an approved patch for TASK.
    #[arg(long)]
    apply: bool,
    /// Detect and optionally run a safe test command.
    #[arg(long = "test")]
    test: bool,
    /// Explain the project structure.
    #[arg(long)]
    explain: bool,
    /// Coding task.
    #[arg(value_name = "TASK", trailing_var_arg = true)]
    task: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum ConfigCommands {
    /// Print the active TOML configuration.
    List,
}

#[derive(Debug, Subcommand)]
enum SkillCommands {
    /// Manage the configured skills registry.
    Registry {
        #[command(subcommand)]
        command: SkillRegistryCommands,
    },
    /// List skills available in the configured registry.
    List,
    /// Search skills in the configured registry.
    Search { query: String },
    /// Show installed skills.
    Installed,
    /// Show available skill bundles.
    Bundles,
    /// Show information for a skill.
    Info { skill_id: String },
    /// Run an installed tool skill with JSON arguments.
    Run {
        skill_id: String,
        #[arg(long)]
        args: Option<String>,
    },
    /// Install a skill from the configured registry.
    Install {
        skill_id: String,
        #[arg(long)]
        registry: Option<String>,
        #[arg(long = "from-local-registry")]
        from_local_registry: Option<PathBuf>,
    },
    /// Install every skill in a registry bundle.
    InstallBundle {
        bundle_id: String,
        #[arg(long)]
        registry: Option<String>,
        #[arg(long = "from-local-registry")]
        from_local_registry: Option<PathBuf>,
    },
    /// Check for available skill updates.
    Update {
        #[arg(long)]
        check: bool,
    },
}

#[derive(Debug, Subcommand)]
enum SkillRegistryCommands {
    /// Print the configured registry URL.
    Current,
    /// Set the configured registry URL.
    Set { url: String },
    /// Load the configured registry and print a summary.
    Refresh,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .without_time()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Doctor) => doctor(),
        Some(Commands::Config {
            command: ConfigCommands::List,
        }) => config_list(),
        Some(Commands::Onboarding) => run_onboarding_then_doctor().await,
        Some(Commands::Chat) => chat().await,
        Some(Commands::Code(command)) => code_commands::run(command).await,
        Some(Commands::Proof { command }) => proof_commands::run(command),
        Some(Commands::Skill { command }) => skill_commands::run(command).await,
        None => startup().await,
    }
}

fn config_list() -> Result<()> {
    let path = AxiomConfig::default_config_path()?;
    let config = AxiomConfig::load_or_create(&path)?;
    println!("{}", config.to_toml_string()?);
    Ok(())
}

async fn startup() -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    match startup::route_for_config_path(&config_path)? {
        StartupRoute::Onboarding => {
            println!("Axiom Agent setup is not complete. Starting onboarding.");
            run_onboarding_then_doctor().await
        }
        StartupRoute::Chat => chat().await,
    }
}

async fn run_onboarding_then_doctor() -> Result<()> {
    onboarding::run_terminal_onboarding().await?;
    doctor()
}

async fn chat() -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    if startup::route_for_config_path(&config_path)? == StartupRoute::Onboarding {
        println!("Onboarding is required before chat can start.");
        if chat::confirm("Start onboarding now?", true)? {
            run_onboarding_then_doctor().await?;
        } else {
            println!("Run `axiom onboarding` when you are ready.");
            return Ok(());
        }
    }

    chat::run_terminal_chat().await
}

fn doctor() -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    let config_exists = config_path.exists();
    let config = if config_exists {
        AxiomConfig::load_from_path(&config_path)?
    } else {
        AxiomConfig::default()
    };

    let workspace_root = config.default_workspace_path();
    let workspace_status = Workspace::new(&workspace_root)
        .map(|workspace| format!("ok ({})", workspace.root().display()))
        .unwrap_or_else(|error| format!("error ({error})"));

    println!("Axiom doctor");
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    println!("os: {}", std::env::consts::OS);
    println!("arch: {}", std::env::consts::ARCH);
    println!("shell: {}", detect_shell());
    println!("git: {}", command_available("git"));
    println!("node: {}", command_available("node"));
    println!("rust: {}", command_available("rustc"));
    println!(
        "config: {} ({})",
        config_path.display(),
        if config_exists { "exists" } else { "missing" }
    );
    println!("workspace: {workspace_status}");
    println!("status: foundation checks completed");

    Ok(())
}

fn detect_shell() -> String {
    std::env::var("SHELL")
        .or_else(|_| std::env::var("COMSPEC"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn command_available(program: &str) -> &'static str {
    match Command::new(program).arg("--version").output() {
        Ok(output) if output.status.success() => "available",
        Ok(_) => "found but returned an error",
        Err(_) => "not found",
    }
}
