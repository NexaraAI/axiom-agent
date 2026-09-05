use nu_ansi_term::{Color, Style};
use std::io::IsTerminal;

use axiom_core::AxiomConfig;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Renderer {
    color_enabled: bool,
    palette: Palette,
}

#[derive(Debug, Clone, Copy)]
struct Palette {
    primary: Color,
    warning: Color,
    text: Color,
    muted: Color,
    success: Color,
}

impl Renderer {
    pub(crate) fn from_config(config: &AxiomConfig) -> Self {
        Self::from_config_with_terminal(config, std::io::stdout().is_terminal())
    }

    fn from_config_with_terminal(config: &AxiomConfig, terminal: bool) -> Self {
        Self {
            color_enabled: config.ui.color
                && config.ui.theme != "none"
                && terminal
                && std::env::var_os("NO_COLOR").is_none(),
            palette: palette_for(&config.ui.theme),
        }
    }

    pub(crate) fn for_onboarding() -> Self {
        Self {
            color_enabled: std::io::stdout().is_terminal()
                && std::env::var_os("NO_COLOR").is_none(),
            palette: palette_for("blood_red"),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn banner(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}\n{}",
            self.red("NEXARA AI / AXIOM"),
            self.bone("The coding agent built to prove every action."),
            self.bone("Welcome! I'll explain as I go — try \"summarize this folder\"."),
            self.smoke("Type !help for commands, !exit to leave."),
            self.smoke("© 2026 DemonZDevelopment")
        )
    }

    pub(crate) fn onboarding_banner(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}\n{}",
            self.red("NEXARA AI / AXIOM"),
            self.bone("The coding agent built to prove every action."),
            self.bone("Welcome! Quick friendly setup: 1) workspace → 2) provider → 3) skills."),
            self.bone("Takes ~1 minute. You can Skip anytime and re-run with `axiom onboarding`."),
            self.smoke("© 2026 DemonZDevelopment")
        )
    }

    pub(crate) fn primary_color(&self) -> Color {
        self.palette.primary
    }

    pub(crate) fn dashboard_banner(
        &self,
        provider: &str,
        model: &str,
        tier: &str,
        workspace: &str,
        session_id: &str,
    ) -> String {
        let tier_val = if tier.is_empty() { "medium" } else { tier };
        let border = "────────────────────────────────────────────────────────────";
        format!(
            "{}\n  {}  {}\n{}\n  {} {}\n  {} {} {}\n  {} {}\n  {} {}\n{}\n  {}\n  {}\n{}",
            self.smoke(&format!("╭─{border}")),
            self.red("◆ AXIOM AGENT"),
            self.smoke("v1.0.2 · Autonomous Workspace Harness"),
            self.smoke(&format!("├─{border}")),
            self.smoke("provider:"),
            self.bone(provider),
            self.smoke("model:"),
            self.bone(model),
            self.red(&format!("[tier: {tier_val}]")),
            self.smoke("workspace:"),
            self.bone(workspace),
            self.smoke("session:"),
            self.smoke(session_id),
            self.smoke(&format!("├─{border}")),
            self.smoke("Commands: /tier · /model · /skills · /clear · /help · /exit"),
            self.smoke("© 2026 DemonZDevelopment"),
            self.smoke(&format!("╰─{border}")),
        )
    }

    pub(crate) fn header(&self, label: &str, value: impl std::fmt::Display) -> String {
        format!(
            "{} {}",
            self.smoke(&format!("{label}:")),
            self.bone(&value.to_string())
        )
    }

    pub(crate) fn prompt(&self) -> String {
        format!("{} ", self.red("axiom>"))
    }

    pub(crate) fn lens_notice(&self, message: &str) -> String {
        format!("{} {}", self.smoke("◈ Axiom Lens:"), self.bone(message))
    }

    pub(crate) fn tool_notice(&self, skill_id: &str, high_risk: bool) -> String {
        if high_risk {
            format!(
                "  {} {}",
                self.ember("▲ [HIGH RISK] Axiom Tool:"),
                self.bone(&format!("executed {skill_id}"))
            )
        } else {
            format!(
                "  {} {}",
                self.green("✔ Axiom Tool:"),
                self.bone(&format!("executed {skill_id}"))
            )
        }
    }

    pub(crate) fn assistant(&self, content: &str) -> String {
        format!("{} {}", self.red("◆ Axiom:"), self.ash(content))
    }

    pub(crate) fn assistant_prefix(&self) -> String {
        format!("{} ", self.red("◆ Axiom:"))
    }

    pub(crate) fn assistant_delta(&self, content: &str) -> String {
        self.ash(content)
    }

    pub(crate) fn error(&self, error: impl std::fmt::Display) -> String {
        format!(
            "  {} {}",
            self.ember("✖ Error:"),
            self.bone(&error.to_string())
        )
    }

    pub(crate) fn success(&self, message: &str) -> String {
        format!("  {} {}", self.green("✔"), self.bone(message))
    }

    pub(crate) fn warning(&self, message: &str) -> String {
        format!("  {} {}", self.ember("▲ Warning:"), self.bone(message))
    }

    pub(crate) fn status_line(&self, message: &str) -> String {
        self.smoke(&format!("  ✦ {message}"))
    }

    pub(crate) fn plain(&self, message: &str) -> String {
        self.bone(message)
    }

    fn red(&self, text: &str) -> String {
        self.paint(self.palette.primary, text)
    }

    fn ember(&self, text: &str) -> String {
        self.paint(self.palette.warning, text)
    }

    fn ash(&self, text: &str) -> String {
        self.paint(self.palette.text, text)
    }

    fn smoke(&self, text: &str) -> String {
        self.paint(self.palette.muted, text)
    }

    fn green(&self, text: &str) -> String {
        self.paint(self.palette.success, text)
    }

    fn bone(&self, text: &str) -> String {
        self.paint(self.palette.text, text)
    }

    fn paint(&self, color: Color, text: &str) -> String {
        if self.color_enabled {
            Style::new().fg(color).paint(text).to_string()
        } else {
            text.to_string()
        }
    }
}

fn palette_for(theme: &str) -> Palette {
    match theme {
        "ash" => Palette {
            primary: Color::Fixed(252),
            warning: Color::Fixed(214),
            text: Color::Fixed(255),
            muted: Color::Fixed(248),
            success: Color::Fixed(151),
        },
        "high_contrast" => Palette {
            primary: Color::Fixed(15),
            warning: Color::Fixed(11),
            text: Color::Fixed(15),
            muted: Color::Fixed(15),
            success: Color::Fixed(10),
        },
        _ => Palette {
            primary: Color::Fixed(196),
            warning: Color::Fixed(202),
            text: Color::Fixed(254),
            muted: Color::Fixed(245),
            success: Color::Fixed(113),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn renderer_uses_blood_red_ansi_color_when_enabled() {
        let mut config = AxiomConfig::default();
        config.ui.color = true;
        let _guard = EnvVarGuard::remove("NO_COLOR");

        assert!(Renderer::from_config_with_terminal(&config, true)
            .prompt()
            .contains("\u{1b}[38;5;196m"));
    }

    #[test]
    fn renderer_respects_color_config_and_no_color() {
        let mut config = AxiomConfig::default();
        config.ui.color = false;
        assert_eq!(
            Renderer::from_config_with_terminal(&config, true).prompt(),
            "axiom> "
        );

        config.ui.color = true;
        let _guard = EnvVarGuard::set("NO_COLOR", "1");
        assert_eq!(
            Renderer::from_config_with_terminal(&config, true).prompt(),
            "axiom> "
        );
    }

    #[test]
    fn none_theme_is_plain_and_high_contrast_avoids_dim_colors() {
        let _guard = EnvVarGuard::remove("NO_COLOR");
        let mut config = AxiomConfig::default();
        config.ui.theme = "none".to_string();
        assert_eq!(
            Renderer::from_config_with_terminal(&config, true).prompt(),
            "axiom> "
        );

        config.ui.theme = "high_contrast".to_string();
        let prompt = Renderer::from_config_with_terminal(&config, true).prompt();
        assert!(prompt.contains("38;5;15m"));
        assert!(!prompt.contains("38;5;240m"));
    }

    #[test]
    fn redirected_output_is_plain_even_when_color_is_enabled() {
        let _guard = EnvVarGuard::remove("NO_COLOR");
        let config = AxiomConfig::default();
        assert_eq!(
            Renderer::from_config_with_terminal(&config, false).prompt(),
            "axiom> "
        );
    }

    #[test]
    fn plain_banner_uses_final_brand_tagline_and_copyright() {
        let mut config = AxiomConfig::default();
        config.ui.color = false;
        let banner = Renderer::from_config_with_terminal(&config, true).banner();
        assert!(banner.contains("NEXARA AI / AXIOM"));
        assert!(banner.contains("The coding agent built to prove every action."));
        assert!(banner.contains("© 2026 DemonZDevelopment"));
    }

    #[test]
    fn onboarding_banner_does_not_advertise_chat_commands() {
        let mut config = AxiomConfig::default();
        config.ui.color = false;
        let banner = Renderer::from_config_with_terminal(&config, true).onboarding_banner();

        assert!(banner.contains("NEXARA AI / AXIOM"));
        assert!(!banner.contains("!help"));
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}
