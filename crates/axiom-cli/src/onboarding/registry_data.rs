#[derive(Debug, Clone, Copy)]
pub(crate) struct EmbeddedRegistryFile {
    pub(crate) relative_path: &'static str,
    pub(crate) contents: &'static [u8],
}

pub(crate) static EMBEDDED_REGISTRY_FILES: &[EmbeddedRegistryFile] = &[
    EmbeddedRegistryFile {
        relative_path: "registry.json",
        contents: include_bytes!("../../../../fixtures/skill-registry/registry.json"),
    },
    EmbeddedRegistryFile {
        relative_path: "bundles/essential.windows.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/bundles/essential.windows.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "bundles/essential.linux.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/bundles/essential.linux.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "bundles/essential.macos.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/bundles/essential.macos.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/file.read/skill.toml",
        contents: include_bytes!("../../../../fixtures/skill-registry/skills/file.read/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/file.read/README.md",
        contents: include_bytes!("../../../../fixtures/skill-registry/skills/file.read/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/file.write/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/file.write/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/file.write/README.md",
        contents: include_bytes!("../../../../fixtures/skill-registry/skills/file.write/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/project.scan/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/project.scan/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/project.scan/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/project.scan/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/web.fetch/skill.toml",
        contents: include_bytes!("../../../../fixtures/skill-registry/skills/web.fetch/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/web.fetch/README.md",
        contents: include_bytes!("../../../../fixtures/skill-registry/skills/web.fetch/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.powershell.safe/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/shell.powershell.safe/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.powershell.safe/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/shell.powershell.safe/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.bash.safe/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/shell.bash.safe/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.bash.safe/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/shell.bash.safe/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.zsh.safe/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/shell.zsh.safe/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.zsh.safe/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/shell.zsh.safe/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/git.status/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/git.status/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/git.status/README.md",
        contents: include_bytes!("../../../../fixtures/skill-registry/skills/git.status/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/git.diff/skill.toml",
        contents: include_bytes!("../../../../fixtures/skill-registry/skills/git.diff/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/git.diff/README.md",
        contents: include_bytes!("../../../../fixtures/skill-registry/skills/git.diff/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/python.write/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/python.write/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/python.write/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/python.write/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/python.run/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/python.run/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/python.run/README.md",
        contents: include_bytes!("../../../../fixtures/skill-registry/skills/python.run/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/github.search/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/github.search/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/github.search/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/github.search/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/deep-research/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/deep-research/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/deep-research/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/deep-research/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/github-research/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/github-research/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/github-research/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/github-research/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/humanized-codes/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/humanized-codes/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/humanized-codes/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/humanized-codes/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/research-first/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/research-first/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/research-first/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/research-first/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/game-builder/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/game-builder/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/game-builder/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/game-builder/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/test.run/skill.toml",
        contents: include_bytes!("../../../../fixtures/skill-registry/skills/test.run/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/test.run/README.md",
        contents: include_bytes!("../../../../fixtures/skill-registry/skills/test.run/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/file.replace/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/file.replace/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/file.replace/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/file.replace/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/subagent.run/skill.toml",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/subagent.run/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/subagent.run/README.md",
        contents: include_bytes!(
            "../../../../fixtures/skill-registry/skills/subagent.run/README.md"
        ),
    },
];
