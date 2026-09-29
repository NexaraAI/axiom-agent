pub(crate) struct SlashCommandDef {
    pub(crate) name: &'static str,
    pub(crate) args: &'static str,
    pub(crate) desc: &'static str,
}

pub(crate) const SLASH_COMMANDS: &[SlashCommandDef] = &[
    SlashCommandDef {
        name: "/help",
        args: "",
        desc: "Show help and command reference",
    },
    SlashCommandDef {
        name: "/model",
        args: "[name]",
        desc: "Switch or view active LLM model",
    },
    SlashCommandDef {
        name: "/models",
        args: "[filter]",
        desc: "List catalog view of available models",
    },
    SlashCommandDef {
        name: "/provider",
        args: "[name]",
        desc: "Show or switch active LLM provider",
    },
    SlashCommandDef {
        name: "/plan",
        args: "",
        desc: "Switch to plan mode (read-only until applied)",
    },
    SlashCommandDef {
        name: "/build",
        args: "",
        desc: "Switch to build mode (tool execution)",
    },
    SlashCommandDef {
        name: "/todo",
        args: "",
        desc: "Show the plan Axiom is tracking",
    },
    SlashCommandDef {
        name: "/skills",
        args: "",
        desc: "List active and installed skills",
    },
    SlashCommandDef {
        name: "/workspace",
        args: "[path]",
        desc: "Show or change active workspace directory",
    },
    SlashCommandDef {
        name: "/status",
        args: "",
        desc: "Show version, install mode, and binary health",
    },
    SlashCommandDef {
        name: "/update",
        args: "",
        desc: "Check for and automatically install updates",
    },
    SlashCommandDef {
        name: "/variant",
        args: "[xhigh|high|medium|low]",
        desc: "Configure model effort/variant",
    },
    SlashCommandDef {
        name: "/thinking",
        args: "[on|off|auto]",
        desc: "Toggle reasoning/thinking mode",
    },
    SlashCommandDef {
        name: "/test",
        args: "[command]",
        desc: "Auto-detect and run workspace tests",
    },
    SlashCommandDef {
        name: "/permission",
        args: "[velocity|full|strict]",
        desc: "Switch permission mode",
    },
    SlashCommandDef {
        name: "/theme",
        args: "[axiom|blood|ash|high]",
        desc: "Switch visual color theme",
    },
    SlashCommandDef {
        name: "/clear",
        args: "",
        desc: "Clear conversation history",
    },
    SlashCommandDef {
        name: "/undo",
        args: "",
        desc: "Restore latest workspace checkpoint",
    },
    SlashCommandDef {
        name: "/checkpoints",
        args: "",
        desc: "List recovery snapshots",
    },
    SlashCommandDef {
        name: "/restore",
        args: "<id>",
        desc: "Restore an agent recovery snapshot",
    },
    SlashCommandDef {
        name: "/proof",
        args: "[on|off|status|latest]",
        desc: "Audit and execution provenance",
    },
    SlashCommandDef {
        name: "/history",
        args: "[id]",
        desc: "List past sessions or switch to one",
    },
    SlashCommandDef {
        name: "/resume",
        args: "<id>",
        desc: "Continue a previous conversation",
    },
    SlashCommandDef {
        name: "/show",
        args: "<output_id>",
        desc: "Display durable tool output",
    },
    SlashCommandDef {
        name: "/commands",
        args: "",
        desc: "Display interactive command palette",
    },
    SlashCommandDef {
        name: "/exit",
        args: "",
        desc: "Exit Axiom session",
    },
];
