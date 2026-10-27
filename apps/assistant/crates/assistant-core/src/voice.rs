//! Voice and Aura choices are independent of persona prompts and aliases.

use crate::{paths, Persona};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceProfile {
    pub persona_id: String,
    pub provider: String,
    pub voice: String,
    /// Developer-supplied, rights-cleared clip for a cloned voice.
    #[serde(default)]
    pub reference_audio: Option<PathBuf>,
    #[serde(default = "default_tint")]
    pub aura_tint: String,
    #[serde(default)]
    pub aura_style: Option<String>,
}

fn default_tint() -> String {
    "accent".into()
}

impl VoiceProfile {
    pub fn path(id: &str) -> PathBuf {
        paths::voices_dir().join(format!("{id}.toml"))
    }

    pub fn from_persona(persona: &Persona) -> Self {
        let tint = match persona.id.as_str() {
            "nyx" | "vesper" => "danger",
            "echo" | "mira" | "cora" => "highlight",
            "cipher" | "hal" => "border",
            _ => "accent",
        };
        Self {
            persona_id: persona.id.clone(),
            provider: "kokoro".into(),
            voice: persona.voice.clone(),
            reference_audio: None,
            aura_tint: tint.into(),
            aura_style: None,
        }
    }

    pub fn load(id: &str) -> crate::Result<Option<Self>> {
        if id.is_empty() || Persona::slugify(id) != id {
            return Err(crate::Error::msg("invalid persona ID"));
        }
        let path = Self::path(id);
        if !path.exists() {
            return Ok(None);
        }
        let profile: Self = toml::from_str(&fs::read_to_string(path)?)?;
        profile.validate()?;
        if profile.persona_id != id {
            return Err(crate::Error::msg("voice profile ID mismatch"));
        }
        Ok(Some(profile))
    }

    pub fn ensure_for(persona: &Persona) -> crate::Result<Self> {
        if let Some(profile) = Self::load(&persona.id)? {
            return Ok(profile);
        }
        let profile = Self::from_persona(persona);
        profile.save()?;
        Ok(profile)
    }

    pub fn save(&self) -> crate::Result<()> {
        self.validate()?;
        fs::create_dir_all(paths::voices_dir())?;
        crate::config::atomic_write(&Self::path(&self.persona_id), toml::to_string_pretty(self)?.as_bytes())
    }

    pub fn validate(&self) -> crate::Result<()> {
        if self.persona_id.is_empty()
            || Persona::slugify(&self.persona_id) != self.persona_id
            || !["kokoro", "chatterbox"].contains(&self.provider.as_str())
            || self.voice.trim().is_empty()
            || (self.provider == "chatterbox"
                && !self.reference_audio.as_ref().is_some_and(|path| path.is_absolute() && path.extension().and_then(|ext| ext.to_str()) == Some("wav")))
            || !["accent", "highlight", "danger", "border"].contains(&self.aura_tint.as_str())
            || self.aura_style.as_ref().is_some_and(|s| !["full", "edge-flow"].contains(&s.as_str()))
        {
            return Err(crate::Error::msg("invalid voice profile"));
        }
        Ok(())
    }
}

/// Voices available in the installed Kokoro pack. No download or synthesis is done here.
pub fn installed_kokoro_voices() -> Vec<String> {
    let root = paths::cache_dir().join("loqui/models--onnx-community--Kokoro-82M-v1.0-ONNX/snapshots");
    let mut voices = Vec::new();
    if let Ok(snapshots) = fs::read_dir(root) {
        for snapshot in snapshots.flatten() {
            if let Ok(files) = fs::read_dir(snapshot.path().join("voices")) {
                for file in files.flatten() {
                    let path = file.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("bin") && path.is_file() {
                        if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                            voices.push(name.to_string());
                        }
                    }
                }
            }
        }
    }
    voices.sort();
    voices.dedup();
    voices
}
