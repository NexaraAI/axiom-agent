use axiom_engine::{Permission, SideEffectClass};

use crate::protocol::ToolAnnotations;

/// Parses a configured side-effect class name.
pub fn parse_side_effect_class(name: &str) -> Option<SideEffectClass> {
    match name.trim().to_ascii_lowercase().as_str() {
        "filesystem_read" => Some(SideEffectClass::FilesystemRead),
        "filesystem_write" => Some(SideEffectClass::FilesystemWrite),
        "network" => Some(SideEffectClass::Network),
        "process" => Some(SideEffectClass::Process),
        "git" => Some(SideEffectClass::Git),
        _ => None,
    }
}

/// Parses a configured list of class names, reporting the first unknown entry.
pub fn parse_side_effect_classes(
    names: &[String],
) -> std::result::Result<Vec<SideEffectClass>, String> {
    names
        .iter()
        .map(|name| {
            parse_side_effect_class(name)
                .ok_or_else(|| format!("unknown side-effect class `{name}`"))
        })
        .collect()
}

/// Derives the side-effect classes for a remote tool from its annotations.
///
/// The protocol's defaults are deliberately pessimistic (not read-only,
/// destructive, open-world), so an un-annotated third-party tool is gated as a
/// process launch that writes and reaches the network. `Process` is always
/// present because calling an MCP tool means running someone else's program.
pub fn classes_for_annotations(annotations: Option<&ToolAnnotations>) -> Vec<SideEffectClass> {
    let read_only = annotations
        .and_then(|annotations| annotations.read_only_hint)
        .unwrap_or(false);
    let open_world = annotations
        .and_then(|annotations| annotations.open_world_hint)
        .unwrap_or(true);

    let mut classes = vec![SideEffectClass::Process];
    classes.push(if read_only {
        SideEffectClass::FilesystemRead
    } else {
        SideEffectClass::FilesystemWrite
    });
    if open_world {
        classes.push(SideEffectClass::Network);
    }
    classes.sort_unstable();
    classes.dedup();
    classes
}

/// Maps side-effect classes onto the permissions Axiom advertises for a tool.
pub fn permissions_for_classes(classes: &[SideEffectClass]) -> Vec<Permission> {
    let mut permissions = Vec::new();
    for class in classes {
        let permission = match class {
            SideEffectClass::FilesystemRead => Permission::FileSystemRead,
            SideEffectClass::FilesystemWrite => Permission::FileSystemWrite,
            SideEffectClass::Network => Permission::Network,
            SideEffectClass::Process | SideEffectClass::Git => Permission::ShellRun,
        };
        if !permissions.contains(&permission) {
            permissions.push(permission);
        }
    }
    permissions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unannotated_tools_gate_as_process_write_and_network() {
        let classes = classes_for_annotations(None);

        assert_eq!(
            classes,
            vec![
                SideEffectClass::FilesystemWrite,
                SideEffectClass::Network,
                SideEffectClass::Process,
            ]
        );
    }

    #[test]
    fn read_only_closed_world_tools_gate_as_a_local_read() {
        let annotations = ToolAnnotations {
            read_only_hint: Some(true),
            open_world_hint: Some(false),
            ..ToolAnnotations::default()
        };

        assert_eq!(
            classes_for_annotations(Some(&annotations)),
            vec![SideEffectClass::FilesystemRead, SideEffectClass::Process]
        );
    }

    #[test]
    fn read_only_open_world_tools_still_declare_the_network() {
        let annotations = ToolAnnotations {
            read_only_hint: Some(true),
            ..ToolAnnotations::default()
        };

        assert_eq!(
            classes_for_annotations(Some(&annotations)),
            vec![
                SideEffectClass::FilesystemRead,
                SideEffectClass::Network,
                SideEffectClass::Process,
            ]
        );
    }

    #[test]
    fn parses_configured_class_names() {
        assert_eq!(
            parse_side_effect_class("Network"),
            Some(SideEffectClass::Network)
        );
        assert_eq!(
            parse_side_effect_classes(&["filesystem_read".to_string(), "git".to_string()]),
            Ok(vec![SideEffectClass::FilesystemRead, SideEffectClass::Git])
        );
        assert!(parse_side_effect_classes(&["teleport".to_string()]).is_err());
    }

    #[test]
    fn maps_classes_onto_unique_permissions() {
        let permissions = permissions_for_classes(&[
            SideEffectClass::Process,
            SideEffectClass::Git,
            SideEffectClass::Network,
        ]);

        assert_eq!(permissions, vec![Permission::ShellRun, Permission::Network]);
    }
}
