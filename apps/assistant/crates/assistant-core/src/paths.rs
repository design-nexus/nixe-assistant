use std::path::PathBuf;

pub fn xdg_runtime() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| cache_dir().join("runtime"))
}

pub fn runtime_dir() -> PathBuf {
    xdg_runtime().join("omarchy-assistant")
}

/// Isolated Voxtype runtime so the assistant's Parakeet daemon does not share F9's lock.
pub fn assistant_voxtype_runtime() -> PathBuf {
    let p = xdg_runtime().join("assistant-voxtype");
    if p.exists() {
        p
    } else {
        let legacy = xdg_runtime().join("nyx-voxtype");
        if legacy.exists() {
            legacy
        } else {
            p
        }
    }
}

pub fn nyx_voxtype_runtime() -> PathBuf {
    assistant_voxtype_runtime()
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("assistant.sock")
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".").join(".config"))
        .join("omarchy-assistant")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn personas_dir() -> PathBuf {
    config_dir().join("personas")
}

pub fn apps_path() -> PathBuf {
    config_dir().join("apps.toml")
}

pub fn commands_path() -> PathBuf {
    config_dir().join("commands.toml")
}

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(".").join(".local/share"))
        .join("omarchy-assistant")
}

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".").join(".cache"))
        .join("omarchy-assistant")
}
