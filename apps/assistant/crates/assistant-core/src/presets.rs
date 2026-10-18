//! Hardware presets: one choice that sets the model, its context, where speech runs, how
//! long the models stay loaded and how far a task may go, for the computer at hand.
//! Applying one writes ordinary settings (`config.toml`, `power.json`); changing any of them
//! afterwards makes the configuration "custom".

use crate::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Lite,
    Balanced,
    Advanced,
}

/// What a preset sets.
#[derive(Debug, Clone, PartialEq)]
pub struct Values {
    /// A curated model's file name (see `assistant-models`).
    pub model_file: &'static str,
    pub model_name: &'static str,
    pub ctx: u32,
    pub n_gpu_layers: i32,
    pub tts_device: &'static str,
    pub vision_idle_secs: u64,
    pub agent_max_calls: usize,
    /// Unload the models after this long idle (0: never); `power.json`.
    pub idle_seconds: u64,
    /// On battery, unload after this long instead (0: no change); `power.json`.
    pub battery_idle_seconds: u64,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Lite, Preset::Balanced, Preset::Advanced];

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "lite" => Some(Self::Lite),
            "balanced" => Some(Self::Balanced),
            "advanced" => Some(Self::Advanced),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Lite => "lite",
            Self::Balanced => "balanced",
            Self::Advanced => "advanced",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Lite => "Small model and short memory, speech on the processor, unloads soon: for laptops, small graphics cards and battery.",
            Self::Balanced => "The default model with room for a long conversation, speech on the graphics card: for most computers.",
            Self::Advanced => "The default model with the longest memory, kept loaded, and tasks may take more steps: for large graphics cards.",
        }
    }

    /// The settings, for a computer with a graphics card (`gpu`) or without.
    pub fn values(self, gpu: bool) -> Values {
        let gpu_layers = if gpu { -1 } else { 0 };
        let tts = if gpu { "cuda" } else { "cpu" };
        match self {
            Self::Lite => Values {
                model_file: "gemma-4-E2B-it-Q4_K_M.gguf",
                model_name: "Gemma 4 E2B",
                ctx: 8192,
                n_gpu_layers: gpu_layers,
                tts_device: "cpu",
                vision_idle_secs: 60,
                agent_max_calls: 4,
                idle_seconds: 120,
                battery_idle_seconds: 60,
            },
            Self::Balanced => Values {
                model_file: "gemma-4-E4B-it-Q4_K_M.gguf",
                model_name: "Gemma 4 E4B",
                ctx: 16384,
                n_gpu_layers: gpu_layers,
                tts_device: tts,
                vision_idle_secs: 120,
                agent_max_calls: 4,
                idle_seconds: 300,
                battery_idle_seconds: 0,
            },
            Self::Advanced => Values {
                model_file: "gemma-4-E4B-it-Q4_K_M.gguf",
                model_name: "Gemma 4 E4B",
                ctx: 32768,
                n_gpu_layers: gpu_layers,
                tts_device: tts,
                vision_idle_secs: 600,
                agent_max_calls: 8,
                idle_seconds: 0,
                battery_idle_seconds: 0,
            },
        }
    }
}

/// The preset that fits a computer with `vram_gb` of graphics memory (none: no graphics
/// card found), and why. Thresholds: the default model and its context want about 8 GB with
/// speech beside it; the 32k context and a vision model loaded together want about 14 GB.
pub fn recommend(vram_gb: Option<f32>) -> (Preset, String) {
    match vram_gb {
        None => (Preset::Lite, "no NVIDIA graphics card found".into()),
        Some(v) if v < 8.0 => (Preset::Lite, format!("{v:.0} GB of graphics memory")),
        Some(v) if v < 14.0 => (Preset::Balanced, format!("{v:.0} GB of graphics memory")),
        Some(v) => (Preset::Advanced, format!("{v:.0} GB of graphics memory")),
    }
}

/// The preset the settings match, if any. `idle` is `power.json`'s (idle, battery idle).
pub fn matching(cfg: &Config, idle: (u64, u64), gpu: bool) -> Option<Preset> {
    Preset::ALL.into_iter().find(|p| {
        let v = p.values(gpu);
        let model = std::path::Path::new(&cfg.llm.model).file_name().is_some_and(|n| n == v.model_file);
        model
            && cfg.llm.ctx == v.ctx
            && cfg.llm.n_gpu_layers == v.n_gpu_layers
            && cfg.tts.device == v.tts_device
            && cfg.vision.idle_secs == v.vision_idle_secs
            && cfg.agent.max_calls == v.agent_max_calls
            && idle == (v.idle_seconds, v.battery_idle_seconds)
    })
}

/// The `config.toml` keys a preset sets, with their values, given where its model is on this
/// computer (`None`: not downloaded, so the model stays as it is).
pub fn config_keys(v: &Values, model_path: Option<&std::path::Path>) -> Vec<(&'static str, serde_json::Value)> {
    use serde_json::json;
    let mut keys = vec![
        ("llm.ctx", json!(v.ctx)),
        ("llm.n_gpu_layers", json!(v.n_gpu_layers)),
        ("tts.device", json!(v.tts_device)),
        ("vision.idle_secs", json!(v.vision_idle_secs)),
        ("agent.max_calls", json!(v.agent_max_calls)),
    ];
    if let Some(path) = model_path {
        keys.insert(0, ("llm.model", json!(path.display().to_string())));
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommends_by_graphics_memory() {
        assert_eq!(recommend(None).0, Preset::Lite);
        assert_eq!(recommend(Some(6.0)).0, Preset::Lite);
        assert_eq!(recommend(Some(8.0)).0, Preset::Balanced);
        assert_eq!(recommend(Some(12.0)).0, Preset::Balanced);
        assert_eq!(recommend(Some(15.9)).0, Preset::Advanced);
        assert_eq!(recommend(Some(15.9)).1, "16 GB of graphics memory");
    }

    #[test]
    fn presets_scale_up() {
        let (l, b, a) = (Preset::Lite.values(true), Preset::Balanced.values(true), Preset::Advanced.values(true));
        assert!(l.ctx < b.ctx && b.ctx < a.ctx);
        assert_eq!(a.idle_seconds, 0, "advanced stays loaded");
        assert_eq!(l.tts_device, "cpu");
        let no_gpu = Preset::Balanced.values(false);
        assert_eq!((no_gpu.n_gpu_layers, no_gpu.tts_device), (0, "cpu"), "without a graphics card everything runs on the processor");
        for p in Preset::ALL {
            assert_eq!(Preset::parse(p.name()), Some(p));
        }
        assert_eq!(Preset::parse("Turbo"), None);
    }

    #[test]
    fn matching_finds_the_applied_preset_or_none() {
        let mut cfg = Config::default();
        let v = Preset::Balanced.values(true);
        cfg.llm.model = format!("/models/{}", v.model_file);
        for (key, value) in config_keys(&v, None) {
            cfg = cfg.with_key(key, value).unwrap();
        }
        assert_eq!(matching(&cfg, (v.idle_seconds, v.battery_idle_seconds), true), Some(Preset::Balanced));
        assert_eq!(matching(&cfg, (60, 0), true), None, "idle changed: custom");
        cfg.llm.ctx = 12000;
        assert_eq!(matching(&cfg, (v.idle_seconds, v.battery_idle_seconds), true), None, "context changed: custom");
    }

    #[test]
    fn the_model_is_set_only_when_downloaded() {
        let v = Preset::Lite.values(true);
        assert!(!config_keys(&v, None).iter().any(|(k, _)| *k == "llm.model"));
        let keys = config_keys(&v, Some(std::path::Path::new("/m/gemma-4-E2B-it-Q4_K_M.gguf")));
        assert_eq!(keys[0], ("llm.model", serde_json::json!("/m/gemma-4-E2B-it-Q4_K_M.gguf")));
    }
}
