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
    accent: Color,
    border: Color,
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
            palette: palette_for("axiom"),
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
        effort: &str,
        mode: &str,
        workspace: &str,
        session_id: &str,
    ) -> String {
        let effort_val = if effort.is_empty() { "medium" } else { effort };
        let version = env!("CARGO_PKG_VERSION");
        let ws_display = if let Some(stripped) = workspace.strip_prefix(r"C:\Users\") {
            if let Some((_user, rest)) = stripped.split_once('\\') {
                format!(r"~\{rest}")
            } else {
                workspace.to_string()
            }
        } else if let Some(stripped) = workspace.strip_prefix("/home/") {
            if let Some((_user, rest)) = stripped.split_once('/') {
                format!("~/{rest}")
            } else {
                workspace.to_string()
            }
        } else {
            workspace.to_string()
        };

        let logo_lines = [
            r"        ▄▄▄       ██   ██ ██  ▄██████▄   ███    ███",
            r"       █████       ██ ██  ██ ███    ███  ████  ████",
            r"      ██   ██       ███   ██ ███    ███  ██ ████ ██",
            r"     █████████     ▄███▄  ██ ███    ███  ██  ██  ██",
            r"    ██       ██   ██   ██ ██  ▀██████▀   ██      ██",
        ];
        let subtitle = "a  x  i  o  m     a  g  e  n  t";

        let border_top = "  ┌─────────────────────────────────────────────────────────────┐";
        let border_bottom = "  └─────────────────────────────────────────────────────────────┘";
        let card_empty = pad_card_line("│", "", 58);

        let logo_colors = [
            Color::Fixed(39),
            Color::Fixed(75),
            Color::Fixed(75),
            Color::Fixed(81),
            Color::Fixed(45),
        ];
        let mut out = Vec::new();
        out.push(String::new());
        for (idx, line) in logo_lines.iter().enumerate() {
            let col = if self.palette.primary == Color::Fixed(75) {
                logo_colors[idx % logo_colors.len()]
            } else {
                self.palette.primary
            };
            out.push(format!("  {}", self.paint(col, line)));
        }
        out.push(format!("                    {}", self.smoke(subtitle)));
        out.push(String::new());

        out.push(self.border(border_top));
        out.push(self.border(&card_empty));

        // Input prompt line
        let input_accent = self.paint(self.palette.primary, "│");
        let input_cursor = self.paint(self.palette.primary, "▌");
        let input_hint = self.smoke("Ask anything... \"Fix a TODO in the codebase\"");
        let input_content = format!("{input_accent} {input_cursor} {input_hint}");
        out.push(pad_card_line(&self.border("│"), &input_content, 58));

        out.push(self.border(&card_empty));

        // Status line
        let status_role = self.smoke("Agent ·");
        let status_model = self.bone(model);
        let status_variant = self.paint(self.palette.warning, &format!("[variant: {effort_val}]"));
        let status_provider = self.smoke(&format!("· {provider}"));
        let status_content =
            format!("{status_role} {status_model} {status_variant} {status_provider}");
        out.push(pad_card_line(&self.border("│"), &status_content, 58));

        out.push(self.border(&card_empty));

        // Mode & Session line
        let mode_norm = if mode.is_empty() { "velocity" } else { mode };
        let mode_styled = match mode_norm {
            "full_machine" => self.red("full_machine"),
            "strict" => self.ember("strict"),
            _ => self.green("velocity"),
        };
        let mode_label = self.smoke("mode:");
        let session_line_content = if !session_id.is_empty() {
            let session_label = self.smoke("session:");
            let session_val = self.smoke(session_id);
            format!("{mode_label} {mode_styled}   {session_label} {session_val}")
        } else {
            format!("{mode_label} {mode_styled}")
        };
        out.push(pad_card_line(&self.border("│"), &session_line_content, 58));
        out.push(self.border(&card_empty));

        // Keybindings hints
        let kb_tab = self.smoke("tab");
        let kb_tab_label = self.ash("skills");
        let kb_cancel = self.smoke("ctrl+c");
        let kb_cancel_label = self.ash("cancel / esc");
        let kb_slash = self.smoke("/");
        let kb_slash_label = self.ash("commands");
        let kb_content = format!(
            "{kb_tab} {kb_tab_label}   {kb_cancel} {kb_cancel_label}   {kb_slash} {kb_slash_label}"
        );
        out.push(pad_card_line(&self.border("│"), &kb_content, 58));

        out.push(self.border(&card_empty));
        out.push(self.border(border_bottom));

        // Footer line
        let total_chars = ws_display
            .len()
            .saturating_add(version.len().saturating_add(1));
        let footer_spaces = " ".repeat(63_usize.saturating_sub(total_chars));
        out.push(format!(
            "  {}{footer_spaces}{}",
            self.smoke(&ws_display),
            self.smoke(&format!("v{version}"))
        ));

        out.join("\n")
    }

    pub(crate) fn command_palette(&self) -> String {
        let border_top = "  ┌─────────────────────────────────────────────────────────────┐";
        let border_bottom = "  └─────────────────────────────────────────────────────────────┘";
        let card_empty = pad_card_line("│", "", 58);

        let mut out = Vec::new();
        out.push(self.border(border_top));
        let prompt_row = format!("> {}", self.bone("/"));
        out.push(pad_card_line(&self.border("│"), &prompt_row, 58));
        out.push(self.border(&card_empty));

        let commands = [
            (
                "/variant",
                "Select variant (Default, low, medium, high)",
                true,
            ),
            ("/model", "Configure or inspect the active model", false),
            ("/permission", "Switch execution permission mode", false),
            ("/theme", "Switch terminal visual color theme", false),
            (
                "/update",
                "Check and install the latest Axiom version",
                false,
            ),
            ("/provider", "Configure or inspect the active LLM", false),
            ("/queue", "Manage pending task queue", false),
            ("/skills", "List and manage installed skills", false),
            ("/proof", "Toggle verifiable proof recording", false),
            ("/checkpoints", "List saved session checkpoints", false),
            ("/undo", "Restore latest workspace checkpoint", false),
            ("/restore", "Restore workspace to a checkpoint", false),
            ("/clear", "Clear conversation history", false),
            ("/help", "Show detailed help and command reference", false),
            ("/exit", "Leave the Axiom session", false),
        ];

        for (cmd, desc, active) in commands {
            if active && self.color_enabled {
                let highlight_style = nu_ansi_term::Style::new()
                    .on(nu_ansi_term::Color::Fixed(215))
                    .fg(nu_ansi_term::Color::Fixed(16))
                    .bold();
                let raw_content = format!("  {:<13} {}", cmd, desc);
                let padding = 59_usize.saturating_sub(raw_content.len());
                let padded = format!("{raw_content}{:>padding$}", "", padding = padding);
                out.push(format!(
                    "  {} {} {}",
                    self.border("│"),
                    highlight_style.paint(padded),
                    self.border("│")
                ));
            } else {
                let cmd_styled = self.bone(cmd);
                let desc_styled = self.smoke(desc);
                let content = format!("{:<14} {}", cmd_styled, desc_styled);
                out.push(pad_card_line(&self.border("│"), &content, 58));
            }
        }

        out.push(self.border(border_bottom));
        out.join("\n")
    }

    pub(crate) fn mcq_card(
        &self,
        question: &str,
        options: &[String],
        allow_custom: bool,
    ) -> String {
        let border_top = "  ┌─────────────────────────────────────────────────────────────┐";
        let border_bottom = "  └─────────────────────────────────────────────────────────────┘";
        let border_mid = "  ├─────────────────────────────────────────────────────────────┤";
        let card_empty = pad_card_line("│", "", 58);
        let mut lines = Vec::new();
        lines.push(self.border(border_top));

        let title = self.bone("Clarification");
        let esc = self.smoke("esc");
        let header_vis = visible_width("Clarification") + visible_width("esc");
        let header_spaces = " ".repeat(58_usize.saturating_sub(header_vis));
        let header_line = format!("{title}{header_spaces}{esc}");
        lines.push(pad_card_line(&self.border("│"), &header_line, 58));
        lines.push(self.border(border_mid));

        let q_prefix = format!("{}  ", self.cyan("?"));
        lines.extend(wrap_card_lines(&self.border("│"), &q_prefix, question, 58));
        lines.push(self.border(&card_empty));

        for (i, opt) in options.iter().enumerate() {
            let badge = self.paint(self.palette.warning, &format!("[{}]", i + 1));
            let opt_prefix = format!("{badge} ");
            lines.extend(wrap_card_lines(&self.border("│"), &opt_prefix, opt, 58));
        }

        if allow_custom {
            let badge = self.smoke(&format!("[{}]", options.len() + 1));
            let custom_prefix = format!("{badge} ");
            lines.extend(wrap_card_lines(
                &self.border("│"),
                &custom_prefix,
                "Type custom answer...",
                58,
            ));
        }

        lines.push(self.border(border_bottom));
        lines.join("\n")
    }

    pub(crate) fn header(&self, label: &str, value: impl std::fmt::Display) -> String {
        format!(
            "{} {}",
            self.smoke(&format!("{label}:")),
            self.bone(&value.to_string())
        )
    }

    pub(crate) fn prompt(&self) -> String {
        if self.color_enabled {
            format!(
                "{} {} ",
                self.paint(self.palette.primary, "│"),
                self.paint(self.palette.primary, "axiom ❯")
            )
        } else {
            "│ axiom ❯ ".to_string()
        }
    }

    pub(crate) fn orchestrator_notice(&self, message: &str) -> String {
        format!("{} {}", self.primary("⟡ Orchestrator:"), self.bone(message))
    }

    pub(crate) fn update_notification_card(&self, current: &str, latest: &str) -> Vec<String> {
        let border_top = "  ┌───────────────────── Update Available ──────────────────────┐";
        let border_bottom = "  └─────────────────────────────────────────────────────────────┘";
        let border_empty = pad_card_line("│", "", 58);
        let msg = format!("A new version of Axiom is available: v{current} -> v{latest}");
        let cmd = "Run /update or npm install -g @nexara/axiom-agent to upgrade";
        let line1 = pad_card_line(&self.border("│"), &self.accent(&msg), 58);
        let line2 = pad_card_line(&self.border("│"), &self.bone(cmd), 58);
        vec![
            self.paint(self.palette.warning, border_top),
            self.border(&border_empty),
            line1,
            line2,
            self.border(&border_empty),
            self.paint(self.palette.warning, border_bottom),
        ]
    }

    #[allow(dead_code)]
    pub(crate) fn tool_notice(&self, skill_id: &str, high_risk: bool) -> String {
        self.tool_notice_with_summary(skill_id, high_risk, None)
    }

    pub(crate) fn tool_notice_with_summary(
        &self,
        skill_id: &str,
        high_risk: bool,
        summary: Option<&str>,
    ) -> String {
        let detail = match summary {
            Some(s) if !s.is_empty() => format!("executed {skill_id} → {s}"),
            _ => format!("executed {skill_id}"),
        };
        if high_risk {
            format!(
                "  {} {}",
                self.ember("▲ [HIGH RISK] Axiom Tool:"),
                self.bone(&detail)
            )
        } else {
            format!("  {} {}", self.green("✔ Axiom Tool:"), self.bone(&detail))
        }
    }

    pub(crate) fn thinking_prefix(&self) -> String {
        format!("  {} ", self.smoke("💭 Thinking:"))
    }

    pub(crate) fn thinking_delta(&self, content: &str) -> String {
        self.smoke(content)
    }

    pub(crate) fn thinking(&self, content: &str) -> String {
        format!("  {} {}", self.smoke("💭 Thinking:"), self.smoke(content))
    }

    pub(crate) fn assistant(&self, content: &str) -> String {
        if let Some(rest) = content.strip_prefix("<think>") {
            if let Some((thought, answer)) = rest.split_once("</think>") {
                let thought = thought.trim();
                let answer = answer.trim();
                if !thought.is_empty() && !answer.is_empty() {
                    return format!(
                        "{}\n{} {}",
                        self.thinking(thought),
                        self.primary("◆ Axiom:"),
                        self.ash(answer)
                    );
                } else if answer.is_empty() {
                    return self.thinking(thought);
                }
            }
        }
        format!("{} {}", self.primary("◆ Axiom:"), self.ash(content))
    }

    pub(crate) fn assistant_prefix(&self) -> String {
        format!("{} ", self.primary("◆ Axiom:"))
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

    pub(crate) fn cyan(&self, text: &str) -> String {
        self.paint(self.palette.accent, text)
    }

    pub(crate) fn border(&self, text: &str) -> String {
        self.paint(self.palette.border, text)
    }

    pub(crate) fn primary(&self, text: &str) -> String {
        self.paint(self.palette.primary, text)
    }

    pub(crate) fn accent(&self, text: &str) -> String {
        self.paint(self.palette.accent, text)
    }

    pub(crate) fn red(&self, text: &str) -> String {
        self.paint(self.palette.primary, text)
    }

    pub(crate) fn ember(&self, text: &str) -> String {
        self.paint(self.palette.warning, text)
    }

    pub(crate) fn ash(&self, text: &str) -> String {
        self.paint(self.palette.text, text)
    }

    pub(crate) fn smoke(&self, text: &str) -> String {
        self.paint(self.palette.muted, text)
    }

    pub(crate) fn green(&self, text: &str) -> String {
        self.paint(self.palette.success, text)
    }

    pub(crate) fn bone(&self, text: &str) -> String {
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
        "axiom" => Palette {
            primary: Color::Fixed(75),
            warning: Color::Fixed(208),
            text: Color::Fixed(255),
            muted: Color::Fixed(243),
            success: Color::Fixed(114),
            accent: Color::Fixed(215),
            border: Color::Fixed(240),
        },
        "ash" => Palette {
            primary: Color::Fixed(252),
            warning: Color::Fixed(214),
            text: Color::Fixed(255),
            muted: Color::Fixed(248),
            success: Color::Fixed(151),
            accent: Color::Fixed(117),
            border: Color::Fixed(240),
        },
        "high_contrast" => Palette {
            primary: Color::Fixed(15),
            warning: Color::Fixed(11),
            text: Color::Fixed(15),
            muted: Color::Fixed(15),
            success: Color::Fixed(10),
            accent: Color::Fixed(14),
            border: Color::Fixed(15),
        },
        "blood_red" => Palette {
            primary: Color::Fixed(196),
            warning: Color::Fixed(202),
            text: Color::Fixed(254),
            muted: Color::Fixed(245),
            success: Color::Fixed(113),
            accent: Color::Fixed(39),
            border: Color::Fixed(240),
        },
        _ => Palette {
            primary: Color::Fixed(75),
            warning: Color::Fixed(208),
            text: Color::Fixed(255),
            muted: Color::Fixed(243),
            success: Color::Fixed(114),
            accent: Color::Fixed(215),
            border: Color::Fixed(240),
        },
    }
}

pub(crate) fn visible_width(s: &str) -> usize {
    let mut in_escape = false;
    let mut count = 0;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            count += 1;
        }
    }
    count
}

pub(crate) fn pad_card_line(border_char: &str, content: &str, target_inner_width: usize) -> String {
    let vis = visible_width(content);
    let padding = target_inner_width.saturating_sub(vis);
    format!(
        "  {border_char}  {content}{:>padding$} {border_char}",
        "",
        padding = padding
    )
}

fn wrap_card_lines(
    border_char: &str,
    prefix: &str,
    text: &str,
    target_inner_width: usize,
) -> Vec<String> {
    let prefix_vis = visible_width(prefix);
    let indent = " ".repeat(prefix_vis);
    let words = text.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return vec![pad_card_line(border_char, prefix, target_inner_width)];
    }

    let mut lines = Vec::new();
    let mut current_line = prefix.to_string();
    let mut current_vis = prefix_vis;

    for word in words {
        let word_vis = visible_width(word);
        if current_vis.saturating_add(1).saturating_add(word_vis) <= target_inner_width {
            if current_vis > prefix_vis {
                current_line.push(' ');
                current_line.push_str(word);
                current_vis = current_vis.saturating_add(1).saturating_add(word_vis);
            } else {
                current_line.push_str(word);
                current_vis = current_vis.saturating_add(word_vis);
            }
        } else {
            lines.push(pad_card_line(
                border_char,
                &current_line,
                target_inner_width,
            ));
            current_line = format!("{indent}{word}");
            current_vis = prefix_vis.saturating_add(word_vis);
        }
    }
    if !current_line.is_empty() {
        lines.push(pad_card_line(
            border_char,
            &current_line,
            target_inner_width,
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn renderer_uses_blood_red_ansi_color_when_configured() {
        let mut config = AxiomConfig::default();
        config.ui.theme = "blood_red".to_string();
        config.ui.color = true;
        let _guard = EnvVarGuard::remove("NO_COLOR");

        assert!(Renderer::from_config_with_terminal(&config, true)
            .prompt()
            .contains("\u{1b}[38;5;196m"));
    }

    #[test]
    fn renderer_uses_axiom_ansi_color_by_default() {
        let mut config = AxiomConfig::default();
        config.ui.color = true;
        let _guard = EnvVarGuard::remove("NO_COLOR");

        assert!(Renderer::from_config_with_terminal(&config, true)
            .prompt()
            .contains("\u{1b}[38;5;75m"));
    }

    #[test]
    fn renderer_respects_color_config_and_no_color() {
        let mut config = AxiomConfig::default();
        config.ui.color = false;
        assert_eq!(
            Renderer::from_config_with_terminal(&config, true).prompt(),
            "│ axiom ❯ "
        );

        config.ui.color = true;
        let _guard = EnvVarGuard::set("NO_COLOR", "1");
        assert_eq!(
            Renderer::from_config_with_terminal(&config, true).prompt(),
            "│ axiom ❯ "
        );
    }

    #[test]
    fn none_theme_is_plain_and_high_contrast_avoids_dim_colors() {
        let _guard = EnvVarGuard::remove("NO_COLOR");
        let mut config = AxiomConfig::default();
        config.ui.theme = "none".to_string();
        assert_eq!(
            Renderer::from_config_with_terminal(&config, true).prompt(),
            "│ axiom ❯ "
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
            "│ axiom ❯ "
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

    #[test]
    fn mcq_card_renders_flush_65_char_box_with_wrapping() {
        let mut config = AxiomConfig::default();
        config.ui.color = false;
        let renderer = Renderer::from_config_with_terminal(&config, true);
        let q = concat!(
            "Which of the following statements about ",
            "the DemonZ-Development Geo-Restrict plugin is false?"
        );
        let opt1 = "It can block or allow players based on country.".to_string();
        let opt2 = concat!(
            "It supports ASN (Autonomous System Number) filtering ",
            "to block entire ISPs."
        )
        .to_string();
        let card = renderer.mcq_card(q, &[opt1, opt2], true);

        for line in card.lines() {
            assert_eq!(
                visible_width(line),
                65,
                "Card line failed 65-char width constraint: {line:?}"
            );
        }
        assert!(card.contains("Clarification"));
        assert!(card.contains("esc"));
        assert!(card.contains("[1]"));
        assert!(card.contains("[2]"));
        assert!(card.contains("[3]"));
        assert!(card.contains("Type custom answer..."));
    }

    #[test]
    fn dashboard_banner_renders_all_permission_modes() {
        let mut config = AxiomConfig::default();
        config.ui.color = false;
        let renderer = Renderer::from_config_with_terminal(&config, true);
        for mode in &["velocity", "full_machine", "strict"] {
            let banner = renderer.dashboard_banner(
                "openai",
                "gpt-4o",
                "medium",
                mode,
                "/home/user/project",
                "session-12345678-abcdef01",
            );
            assert!(banner.contains(&format!("mode: {mode}")));
            assert!(banner.contains("session: session-12345678-abcdef01"));
        }
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
