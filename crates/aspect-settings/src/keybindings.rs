use aspect_core::{Keybinding, KeybindingProfile};

#[must_use]
pub fn default_keybinding_profile() -> KeybindingProfile {
    KeybindingProfile {
        id: "default".to_string(),
        name: "Default".to_string(),
        bindings: vec![
            binding("workbench.action.showCommands", "Ctrl+Shift+P", None),
            binding("workbench.action.quickOpen", "Ctrl+P", None),
            binding("workbench.action.files.newUntitledFile", "Ctrl+N", None),
            binding("workbench.action.openSettings", "Ctrl+,", None),
            binding("workbench.action.openFolder", "Ctrl+O", None),
            binding(
                "workbench.action.toggleSidebar",
                "Ctrl+B",
                Some("workspace"),
            ),
            binding("workbench.view.explorer", "Ctrl+Shift+E", Some("workspace")),
            binding("workbench.view.search", "Ctrl+Shift+F", Some("workspace")),
            binding("workbench.view.scm", "Ctrl+Shift+G", Some("workspace")),
            binding("workbench.view.debug", "Ctrl+Shift+D", Some("workspace")),
            binding(
                "workbench.view.extensions",
                "Ctrl+Shift+X",
                Some("workspace"),
            ),
            binding("workbench.action.chat.toggle", "Ctrl+L", Some("workspace")),
            binding(
                "workbench.action.terminal.toggleTerminal",
                "Ctrl+`",
                Some("workspace"),
            ),
            binding("editor.action.toggleWordWrap", "Alt+Z", Some("editor")),
            binding(
                "editor.action.toggleMinimap",
                "Ctrl+M Ctrl+M",
                Some("editor"),
            ),
            binding("editor.action.fontZoomIn", "Ctrl+=", Some("editor")),
            binding("editor.action.fontZoomIn", "Ctrl+Shift+=", Some("editor")),
            binding("editor.action.fontZoomOut", "Ctrl+-", Some("editor")),
            binding("editor.action.fontZoomReset", "Ctrl+0", Some("editor")),
            binding("workbench.action.files.save", "Ctrl+S", Some("editor")),
            binding(
                "workbench.action.files.saveAs",
                "Ctrl+Shift+S",
                Some("editor"),
            ),
            binding(
                "workbench.action.files.saveAll",
                "Ctrl+K Ctrl+S",
                Some("dirtyEditors"),
            ),
            binding(
                "workbench.action.closeActiveEditor",
                "Ctrl+W",
                Some("editor"),
            ),
            binding(
                "workbench.action.splitEditorRight",
                "Ctrl+\\",
                Some("editor"),
            ),
            binding(
                "workbench.action.nextEditor",
                "Ctrl+PageDown",
                Some("editor"),
            ),
            binding(
                "workbench.action.previousEditor",
                "Ctrl+PageUp",
                Some("editor"),
            ),
        ],
    }
}

pub fn normalize_keybinding_profile(profile: KeybindingProfile) -> KeybindingProfile {
    let mut bindings = Vec::new();
    for binding in profile.bindings {
        let command = binding.command.trim();
        let key = normalize_key_sequence(&binding.key);
        if command.is_empty() || key.is_empty() {
            continue;
        }
        bindings.push(Keybinding {
            command: command.to_string(),
            key,
            when: binding.when.and_then(|value| {
                let value = value.trim();
                (!value.is_empty()).then(|| value.to_string())
            }),
        });
    }

    if bindings.is_empty() {
        return default_keybinding_profile();
    }

    if profile.id.trim().is_empty() || profile.id.trim() == "default" {
        for default_binding in default_keybinding_profile().bindings {
            if !bindings.iter().any(|binding| {
                binding.command == default_binding.command && binding.key == default_binding.key
            }) {
                bindings.push(default_binding);
            }
        }
    }

    KeybindingProfile {
        id: if profile.id.trim().is_empty() {
            "custom".to_string()
        } else {
            profile.id.trim().to_string()
        },
        name: if profile.name.trim().is_empty() {
            "Custom".to_string()
        } else {
            profile.name.trim().to_string()
        },
        bindings,
    }
}

fn normalize_key_sequence(value: &str) -> String {
    value
        .split_whitespace()
        .filter_map(normalize_key_chord)
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_key_chord(value: &str) -> Option<String> {
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut meta = false;
    let mut key = None;

    for part in value.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" | "win" | "super" => meta = true,
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" | "option" => alt = true,
            _ => key = Some(normalize_key_name(part)),
        }
    }

    let key = key?;
    let mut parts = Vec::with_capacity(5);
    if ctrl {
        parts.push("Ctrl".to_string());
    }
    if shift {
        parts.push("Shift".to_string());
    }
    if alt {
        parts.push("Alt".to_string());
    }
    if meta {
        parts.push("Meta".to_string());
    }
    parts.push(key);
    Some(parts.join("+"))
}

fn normalize_key_name(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "esc" => "Escape".to_string(),
        "space" => "Space".to_string(),
        "pgup" => "PageUp".to_string(),
        "pgdn" => "PageDown".to_string(),
        "left" => "ArrowLeft".to_string(),
        "right" => "ArrowRight".to_string(),
        "up" => "ArrowUp".to_string(),
        "down" => "ArrowDown".to_string(),
        key if key.len() == 1 => key.to_ascii_uppercase(),
        _ => {
            let mut chars = value.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        }
    }
}

fn binding(command: &str, key: &str, when: Option<&str>) -> Keybinding {
    Keybinding {
        command: command.to_string(),
        key: normalize_key_sequence(key),
        when: when.map(str::to_string),
    }
}
