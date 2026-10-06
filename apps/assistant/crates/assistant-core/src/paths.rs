use std::path::PathBuf;

pub fn xdg_runtime() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn runtime_dir() -> PathBuf {
    xdg_runtime().join("omarchy-assistant")
}

/// Isolated Voxtype runtime so Nyx's Parakeet daemon does not share F9's lock.
pub fn nyx_voxtype_runtime() -> PathBuf {
    xdg_runtime().join("nyx-voxtype")
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

pub fn apps_path() -> PathBuf {
    config_dir().join("apps.toml")
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
