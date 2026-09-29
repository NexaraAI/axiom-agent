use axiom_core::AxiomConfig;
use axiom_upd::{detect_installation_mode, InstallationMode};

/// Install-health snapshot for the chat `/status` command.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InstallStatus {
    pub(crate) mode: InstallationMode,
    pub(crate) binary_path: Option<String>,
    pub(crate) update_state: String,
    pub(crate) latest_version: Option<String>,
    pub(crate) notes: Option<String>,
}

pub(crate) fn npm_package_version(binary_path: Option<&std::path::Path>) -> Option<String> {
    let path = binary_path?;
    let package_dir = crate::update_commands::find_axiom_package_dir(path)?;
    crate::update_commands::npm_package_version_in(&package_dir)
}

pub(crate) async fn run_status_report(config: &AxiomConfig) -> InstallStatus {
    let binary_path = std::env::current_exe().ok();
    let mode = binary_path
        .as_ref()
        .map(detect_installation_mode)
        .unwrap_or(InstallationMode::Unknown);

    let mut notes = None;
    if mode == InstallationMode::NpmGlobal {
        if let Some(path) = &binary_path {
            if !path.exists() {
                notes = Some(
                    "npm shim reported a binary but it is missing; reinstall with npm.".to_string(),
                );
            }
        }
        let package_version = npm_package_version(binary_path.as_deref());
        if let Some(package_version) = package_version {
            if package_version != env!("CARGO_PKG_VERSION") {
                notes = Some(format!(
                    "npm package v{package_version} is present but the running binary is v{} — the postinstall step was blocked or failed, so the old binary was kept. Reinstall with: npm install -g axiom-agent --allow-scripts=axiom-agent",
                    env!("CARGO_PKG_VERSION")
                ));
            }
        }
    }

    let client = axiom_upd::GitHubReleaseClient::new(&config.update.release_repo).with_timeout(3);
    let (update_state, latest_version) = match client.fetch_releases().await {
        Ok(releases) => {
            let latest = releases.first().and_then(|release| {
                axiom_upd::parse_version(release.tag_name.trim_start_matches('v')).ok()
            });
            let current = axiom_upd::parse_version(env!("CARGO_PKG_VERSION")).ok();
            match (current, latest) {
                (Some(current), Some(latest)) => {
                    if axiom_upd::is_newer_version(&current, &latest) {
                        ("update_available".to_string(), Some(latest.to_string()))
                    } else {
                        ("up_to_date".to_string(), Some(latest.to_string()))
                    }
                }
                _ => ("unknown".to_string(), None),
            }
        }
        Err(_) => ("unknown".to_string(), None),
    };

    InstallStatus {
        mode,
        binary_path: binary_path.map(|path| path.display().to_string()),
        update_state,
        latest_version,
        notes,
    }
}
