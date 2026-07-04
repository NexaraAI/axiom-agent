use std::path::Path;

use anyhow::Result;
use axiom_core::AxiomConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupRoute {
    Onboarding,
    Chat,
}

pub(crate) fn route_for_config_path(path: impl AsRef<Path>) -> Result<StartupRoute> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(StartupRoute::Onboarding);
    }

    let config = AxiomConfig::load_from_path(path)?;
    Ok(route_for_config(&config))
}

pub(crate) fn route_for_config(config: &AxiomConfig) -> StartupRoute {
    if config.agent.first_run_completed {
        StartupRoute::Chat
    } else {
        StartupRoute::Onboarding
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn no_config_routes_to_onboarding() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");

        let route = route_for_config_path(&config_path).expect("route missing config");

        assert_eq!(route, StartupRoute::Onboarding);
    }

    #[test]
    fn incomplete_config_routes_to_onboarding() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.agent.first_run_completed = false;
        config.save_to_path(&config_path).expect("save config");

        let route = route_for_config_path(&config_path).expect("route incomplete config");

        assert_eq!(route, StartupRoute::Onboarding);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn completed_config_routes_to_chat() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.agent.first_run_completed = true;
        config.save_to_path(&config_path).expect("save config");

        let route = route_for_config_path(&config_path).expect("route completed config");

        assert_eq!(route, StartupRoute::Chat);
        let _ = fs::remove_dir_all(dir);
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-cli-startup-test-{nanos}"))
    }
}
