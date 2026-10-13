//! Persistent Assistant Personas with phonetic aliases and prebuilt profiles.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Persona {
    pub id: String,
    pub name: String,
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default = "default_prefixes")]
    pub prefixes: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

fn default_voice() -> String {
    "af_heart".into()
}

fn default_prefixes() -> Vec<String> {
    vec![
        "hey".into(),
        "ok".into(),
        "okay".into(),
        "hi".into(),
        "yo".into(),
        "hello".into(),
        "a".into(),
        "ay".into(),
        "aye".into(),
    ]
}

impl Persona {
    /// List of prebuilt personas: Nyx, Iris, Echo, Cipher, Vesper, Jarvis, Mira, Hal, Cora.
    pub fn prebuilts() -> Vec<Self> {
        vec![
            Self {
                id: "nyx".into(),
                name: "Nyx".into(),
                voice: "af_heart".into(),
                prompt: "You are Nyx, a fast local voice assistant on Omarchy. Answer questions out loud in short spoken sentences. No markdown, no bullet lists, no code fences. Never claim you changed the computer.".into(),
                prefixes: vec!["hey".into(), "ok".into(), "okay".into(), "hi".into(), "yo".into(), "hello".into(), "a".into(), "ay".into()],
                aliases: vec![
                    "nyx".into(),
                    "nix".into(),
                    "nicks".into(),
                    "nick".into(),
                    "niks".into(),
                    "nics".into(),
                    "nyks".into(),
                    "next".into(),
                    "six".into(),
                    "neks".into(),
                    "knicks".into(),
                    "knix".into(),
                    "a nyx".into(),
                    "ay nyx".into(),
                    "yo nyx".into(),
                    "hey nyx".into(),
                ],
            },
            Self {
                id: "iris".into(),
                name: "Iris".into(),
                voice: "af_bella".into(),
                prompt: "You are Iris, an insightful and elegant spoken AI companion. Answer concisely and conversationally in natural spoken sentences. No markdown, no code blocks.".into(),
                prefixes: vec!["hey".into(), "ok".into(), "okay".into(), "hi".into(), "yo".into(), "hello".into(), "a".into(), "ay".into()],
                aliases: vec![
                    "iris".into(),
                    "eyris".into(),
                    "irish".into(),
                    "eyeless".into(),
                    "irus".into(),
                    "ayris".into(),
                    "aris".into(),
                    "ire is".into(),
                    "eye ris".into(),
                    "ires".into(),
                    "iriss".into(),
                    "hi ris".into(),
                    "hi iris".into(),
                    "a iris".into(),
                    "ay iris".into(),
                    "hey iris".into(),
                    "yiris".into(),
                ],
            },
            Self {
                id: "echo".into(),
                name: "Echo".into(),
                voice: "af_heart".into(),
                prompt: "You are Echo, an agile, snappy voice assistant. Provide clear, direct, spoken answers in one or two sentences. No markdown.".into(),
                prefixes: vec!["hey".into(), "ok".into(), "okay".into(), "hi".into(), "yo".into(), "hello".into(), "a".into(), "ay".into()],
                aliases: vec![
                    "echo".into(),
                    "eco".into(),
                    "ecko".into(),
                    "ekko".into(),
                    "eko".into(),
                    "gecko".into(),
                    "hecko".into(),
                    "eggo".into(),
                    "hecho".into(),
                    "heco".into(),
                    "acco".into(),
                    "acko".into(),
                    "a echo".into(),
                    "ay echo".into(),
                    "yo echo".into(),
                    "hey echo".into(),
                    "ok echo".into(),
                    "okay echo".into(),
                    "deco".into(),
                    "deko".into(),
                ],
            },
            Self {
                id: "cipher".into(),
                name: "Cipher".into(),
                voice: "am_adam".into(),
                prompt: "You are Cipher, a sharp, technical spoken intelligence. Answer precisely and thoughtfully without unnecessary fluff. No markdown.".into(),
                prefixes: vec!["hey".into(), "ok".into(), "okay".into(), "hi".into(), "yo".into(), "hello".into(), "a".into(), "ay".into()],
                aliases: vec![
                    "cipher".into(),
                    "cypher".into(),
                    "sipher".into(),
                    "sypher".into(),
                    "cyfa".into(),
                    "syfa".into(),
                    "site for".into(),
                    "sight for".into(),
                    "safer".into(),
                    "cypha".into(),
                    "seifer".into(),
                    "sifer".into(),
                    "sci fi".into(),
                    "a cipher".into(),
                    "ay cipher".into(),
                ],
            },
            Self {
                id: "vesper".into(),
                name: "Vesper".into(),
                voice: "bf_emma".into(),
                prompt: "You are Vesper, a calm, articulate spoken assistant. Provide courteous, succinct answers suited for listening. No markdown.".into(),
                prefixes: vec!["hey".into(), "ok".into(), "okay".into(), "hi".into(), "yo".into(), "hello".into(), "a".into(), "ay".into()],
                aliases: vec![
                    "vesper".into(),
                    "vespa".into(),
                    "vespar".into(),
                    "whisper".into(),
                    "vespr".into(),
                    "jasper".into(),
                    "fesper".into(),
                    "vespur".into(),
                    "vest per".into(),
                    "west per".into(),
                    "a vesper".into(),
                    "ay vesper".into(),
                ],
            },
            Self {
                id: "jarvis".into(),
                name: "Jarvis".into(),
                voice: "bm_george".into(),
                prompt: "You are Jarvis, a brilliant, composed spoken AI assistant. Keep replies concise, refined, and natural. No markdown, no bullet lists.".into(),
                prefixes: vec!["hey".into(), "ok".into(), "okay".into(), "hi".into(), "yo".into(), "hello".into(), "a".into(), "ay".into()],
                aliases: vec![
                    "jarvis".into(),
                    "jarves".into(),
                    "jarvus".into(),
                    "harvest".into(),
                    "jar visit".into(),
                    "service".into(),
                    "jar vis".into(),
                    "jar bliss".into(),
                    "charvis".into(),
                    "chavis".into(),
                    "javis".into(),
                    "a jarvis".into(),
                    "ay jarvis".into(),
                ],
            },
            Self {
                id: "mira".into(),
                name: "Mira".into(),
                voice: "af_sarah".into(),
                prompt: "You are Mira, a friendly, perceptive voice assistant. Keep answers natural, brief, and helpful. Spoken tone only.".into(),
                prefixes: vec!["hey".into(), "ok".into(), "okay".into(), "hi".into(), "yo".into(), "hello".into(), "a".into(), "ay".into()],
                aliases: vec![
                    "mira".into(),
                    "meera".into(),
                    "myra".into(),
                    "mirror".into(),
                    "meerah".into(),
                    "mera".into(),
                    "mirah".into(),
                    "meara".into(),
                    "nearer".into(),
                    "mida".into(),
                    "a mira".into(),
                    "ay mira".into(),
                ],
            },
            Self {
                id: "hal".into(),
                name: "Hal".into(),
                voice: "bm_lewis".into(),
                prompt: "You are Hal, a calm, deliberate voice assistant. Keep responses measured, brief, and strictly spoken. No markdown.".into(),
                prefixes: vec!["hey".into(), "ok".into(), "okay".into(), "hi".into(), "yo".into(), "hello".into(), "a".into(), "ay".into()],
                aliases: vec![
                    "hal".into(),
                    "how".into(),
                    "hell".into(),
                    "pal".into(),
                    "al".into(),
                    "howie".into(),
                    "hound".into(),
                    "hall".into(),
                    "hull".into(),
                    "help".into(),
                    "howl".into(),
                    "a hal".into(),
                    "ay hal".into(),
                ],
            },
            Self {
                id: "cora".into(),
                name: "Cora".into(),
                voice: "bf_isabella".into(),
                prompt: "You are Cora, a warm, lively spoken voice assistant. Keep answers concise, direct, and conversational. Spoken sentences only.".into(),
                prefixes: vec!["hey".into(), "ok".into(), "okay".into(), "hi".into(), "yo".into(), "hello".into(), "a".into(), "ay".into()],
                aliases: vec![
                    "cora".into(),
                    "kora".into(),
                    "coral".into(),
                    "koral".into(),
                    "chora".into(),
                    "corah".into(),
                    "core uh".into(),
                    "korah".into(),
                    "core a".into(),
                    "quran".into(),
                    "flora".into(),
                    "aura".into(),
                    "a cora".into(),
                    "ay cora".into(),
                ],
            },
        ]
    }

    /// Checks if a persona ID belongs to one of the built-in system personas.
    pub fn is_prebuilt(id: &str) -> bool {
        matches!(
            id.to_lowercase().as_str(),
            "nyx" | "iris" | "echo" | "cipher" | "vesper" | "jarvis" | "mira" | "hal" | "cora"
        )
    }

    /// Converts a display name into a sanitized filesystem ID slug.
    pub fn slugify(name: &str) -> String {
        let mut slug = String::new();
        for c in name.chars() {
            if c.is_alphanumeric() {
                slug.push(c.to_ascii_lowercase());
            } else if (c == ' ' || c == '-' || c == '_') && !slug.ends_with('-') && !slug.is_empty()
            {
                slug.push('-');
            }
        }
        let trimmed = slug.trim_matches('-').to_string();
        if trimmed.is_empty() {
            "assistant".into()
        } else {
            trimmed
        }
    }

    /// Automatically generates speech-to-text phonetic aliases and common mishearings for any name.
    pub fn generate_aliases(name: &str) -> Vec<String> {
        let lower = name.trim().to_lowercase();
        if lower.is_empty() {
            return Vec::new();
        }

        let mut variants = Vec::new();

        // Check if matching a prebuilt persona first to borrow its hand-curated list
        for prebuilt in Self::prebuilts() {
            if prebuilt.id == lower || prebuilt.name.to_lowercase() == lower {
                for a in prebuilt.aliases {
                    if !variants.contains(&a) {
                        variants.push(a);
                    }
                }
                break;
            }
        }

        if !variants.contains(&lower) {
            variants.push(lower.clone());
        }

        // Phonetic expansions
        let mut expansions = Vec::new();
        expansions.push(lower.clone());

        // c <-> k
        if lower.contains('c') {
            expansions.push(lower.replace('c', "k"));
        }
        if lower.contains('k') {
            expansions.push(lower.replace('k', "c"));
        }
        // ph <-> f
        if lower.contains("ph") {
            expansions.push(lower.replace("ph", "f"));
        }
        if lower.contains('f') {
            expansions.push(lower.replace('f', "ph"));
        }
        // s <-> z / cy / c
        if lower.contains('s') {
            expansions.push(lower.replace('s', "z"));
            expansions.push(lower.replace('s', "c"));
        }
        if lower.contains('z') {
            expansions.push(lower.replace('z', "s"));
        }
        // sh <-> ch
        if lower.contains("sh") {
            expansions.push(lower.replace("sh", "ch"));
        }
        if lower.contains("ch") {
            expansions.push(lower.replace("ch", "sh"));
        }
        // v <-> b / w
        if lower.contains('v') {
            expansions.push(lower.replace('v', "b"));
            expansions.push(lower.replace('v', "w"));
        }
        if lower.contains('w') {
            expansions.push(lower.replace('w', "v"));
        }
        // th <-> t / f
        if lower.contains("th") {
            expansions.push(lower.replace("th", "t"));
            expansions.push(lower.replace("th", "f"));
        }
        // qu <-> kw
        if lower.contains("qu") {
            expansions.push(lower.replace("qu", "kw"));
        }
        // x -> ks, cks, cs, z
        if lower.contains('x') {
            expansions.push(lower.replace('x', "ks"));
            expansions.push(lower.replace('x', "cks"));
            expansions.push(lower.replace('x', "cs"));
            expansions.push(lower.replace('x', "z"));
        }
        // y <-> i, ee
        if lower.contains('y') {
            expansions.push(lower.replace('y', "i"));
            expansions.push(lower.replace('y', "ee"));
        }
        if lower.contains('i') {
            expansions.push(lower.replace('i', "y"));
            expansions.push(lower.replace('i', "ee"));
        }
        // double vowels
        if lower.contains("ee") {
            expansions.push(lower.replace("ee", "i"));
            expansions.push(lower.replace("ee", "ea"));
        }
        if lower.contains("ea") {
            expansions.push(lower.replace("ea", "ee"));
        }
        if lower.contains("oo") {
            expansions.push(lower.replace("oo", "u"));
        }
        if lower.contains("ou") {
            expansions.push(lower.replace("ou", "ow"));
        }
        if lower.contains("ai") {
            expansions.push(lower.replace("ai", "ay"));
            expansions.push(lower.replace("ai", "ey"));
        }

        // Head changes (word starts)
        let vowels = ['a', 'e', 'i', 'o', 'u'];
        if lower.starts_with(&vowels[..]) {
            expansions.push(format!("h{lower}"));
            expansions.push(format!("y{lower}"));
        } else if lower.starts_with('h') && lower.len() > 1 {
            expansions.push(lower[1..].to_string());
        } else if lower.starts_with("wh") && lower.len() > 2 {
            expansions.push(format!("w{}", &lower[2..]));
        }

        // Suffix adjustments (common speech-to-text endings)
        for base in &expansions {
            variants.push(base.clone());

            if base.ends_with("er") && base.len() > 2 {
                let stem = &base[..base.len() - 2];
                variants.push(format!("{stem}ar"));
                variants.push(format!("{stem}a"));
                variants.push(format!("{stem}ah"));
                variants.push(format!("{stem}or"));
                variants.push(format!("{stem}ur"));
            } else if base.ends_with('a') && base.len() > 1 {
                let stem = &base[..base.len() - 1];
                variants.push(format!("{base}h"));
                variants.push(format!("{stem}uh"));
            } else if base.ends_with("is") && base.len() > 2 {
                let stem = &base[..base.len() - 2];
                variants.push(format!("{stem}es"));
                variants.push(format!("{stem}us"));
                variants.push(format!("{stem}ish"));
                variants.push(format!("{stem}ys"));
                variants.push(format!("{stem}ice"));
            } else if base.ends_with("us") && base.len() > 2 {
                let stem = &base[..base.len() - 2];
                variants.push(format!("{stem}as"));
                variants.push(format!("{stem}is"));
                variants.push(format!("{stem}os"));
            } else if base.ends_with('o') && base.len() > 1 {
                variants.push(format!("{base}h"));
                variants.push(format!("{base}w"));
                variants.push(format!("{base}e"));
            } else if base.ends_with('y') && base.len() > 1 {
                let stem = &base[..base.len() - 1];
                variants.push(format!("{stem}ee"));
                variants.push(format!("{stem}ey"));
                variants.push(format!("{stem}i"));
            }
        }

        // Add compound prefix forms (common acoustic head slurs like "a <name>", "ay <name>")
        let current_variants = variants.clone();
        for v in current_variants {
            if !v.contains(' ') {
                variants.push(format!("a {v}"));
                variants.push(format!("ay {v}"));
            }
        }

        // Filter and deduplicate
        let mut result = Vec::new();
        for v in variants {
            let clean = v.trim().to_lowercase();
            if clean.len() >= 2 && !result.contains(&clean) {
                result.push(clean);
            }
        }

        if !result.contains(&lower) {
            result.insert(0, lower);
        }

        result
    }

    /// Constructs a new persona with automatic alias generation and defaults.
    pub fn new_with_generated(name: &str) -> Self {
        let trimmed_name = name.trim();
        let display_name = if trimmed_name.is_empty() {
            "Assistant".to_string()
        } else {
            let mut chars = trimmed_name.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => "Assistant".to_string(),
            }
        };

        let id = Self::slugify(&display_name);
        let aliases = Self::generate_aliases(&display_name);
        let prompt = format!(
            "You are {display_name}, a fast spoken voice assistant on Omarchy. \
             Answer questions out loud in short, natural spoken sentences. \
             No markdown, no bullet lists, no code fences."
        );

        Self {
            id,
            name: display_name,
            voice: "af_heart".into(),
            prompt,
            prefixes: default_prefixes(),
            aliases,
        }
    }

    /// Ensures the personas directory exists and all prebuilt personas are written to disk
    /// if not already present.
    pub fn ensure_prebuilts() -> crate::Result<()> {
        let dir = paths::personas_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }

        for prebuilt in Self::prebuilts() {
            let path = dir.join(format!("{}.toml", prebuilt.id));
            if !path.exists() {
                prebuilt.save()?;
            }
        }

        Ok(())
    }

    /// Loads all saved personas from disk (`~/.config/nixe-assistant/personas/*.toml`).
    /// Seeds prebuilts if they do not exist yet.
    pub fn load_all() -> crate::Result<Vec<Self>> {
        Self::ensure_prebuilts()?;
        let dir = paths::personas_dir();
        let mut personas = Vec::new();

        if let Ok(rd) = fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    let p = Self::load_from_file(&path)?;
                    if personas
                        .iter()
                        .any(|old: &Self| old.id == p.id || old.name.eq_ignore_ascii_case(&p.name))
                    {
                        return Err(crate::Error::msg("duplicate persona ID/name"));
                    }
                    personas.push(p);
                }
            }
        }

        // Sort alphabetically by name
        personas.sort_by_key(|a| a.name.to_lowercase());

        if personas.is_empty() {
            personas = Self::prebuilts();
        }

        Ok(personas)
    }

    /// Loads a persona from a specific TOML file.
    pub fn load_from_file(path: &Path) -> crate::Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut p: Persona = toml::from_str(&text)?;
        if p.id.is_empty() {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                p.id = stem.to_string();
            }
        }
        Ok(p)
    }

    /// Saves this persona to `~/.config/nixe-assistant/personas/<id>.toml`.
    pub fn save(&self) -> crate::Result<()> {
        let dir = paths::personas_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        if self.id.is_empty() || Self::slugify(&self.id) != self.id {
            return Err(crate::Error::msg("invalid persona ID"));
        }
        let path = dir.join(format!("{}.toml", self.id));
        let mut doc = if path.exists() {
            let raw = fs::read_to_string(&path)?;
            let _: Persona = toml::from_str(&raw)?;
            raw.parse::<toml_edit::DocumentMut>()
                .map_err(|e| crate::Error::msg(e.to_string()))?
        } else {
            toml_edit::DocumentMut::new()
        };
        let updated = toml::to_string_pretty(self)?
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| crate::Error::msg(e.to_string()))?;
        crate::config::merge_table(doc.as_table_mut(), updated.as_table());
        let text = doc.to_string();
        crate::config::atomic_write(&path, text.as_bytes())?;
        Ok(())
    }

    /// Deletes this persona from disk if it is not a protected prebuilt.
    pub fn delete(&self) -> crate::Result<bool> {
        if Self::is_prebuilt(&self.id) {
            return Ok(false);
        }
        let path = paths::personas_dir().join(format!("{}.toml", self.id));
        if path.exists() {
            fs::remove_file(path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Updates Omarchy plugin manifest files with the active assistant persona's name and aliases.
pub fn sync_plugin_manifest(persona: &Persona) -> crate::Result<()> {
    let dir = dirs::config_dir()
        .ok_or_else(|| crate::Error::msg("config directory unavailable"))?
        .join("omarchy/plugins");
    let targets: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("manifest.json"))
        .filter(|p| p.is_file())
        .collect();

    let mut aliases = vec![
        persona.id.clone(),
        persona.name.to_lowercase(),
        "assistant".into(),
        "voice".into(),
        "ai".into(),
        "microphone".into(),
    ];
    for a in &persona.aliases {
        if !aliases.contains(a) && !a.contains(' ') {
            aliases.push(a.clone());
        }
    }

    for path in targets {
        if !path.exists() {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content) {
                if !json["id"].as_str().is_some_and(|id| {
                    id.ends_with(".nixe-assistant") || id == "omarchy.assistant"
                }) {
                    continue;
                }
                let name = &persona.name;
                json["name"] = serde_json::json!(name);
                json["description"] = serde_json::json!(format!(
                    "Live status, quick mute/unmute, speech testing, and settings for {name}."
                ));
                if let Some(bar_widget) = json.get_mut("barWidget") {
                    bar_widget["displayName"] = serde_json::json!(name);
                    bar_widget["aliases"] = serde_json::json!(aliases);
                }
                if let Ok(pretty) = serde_json::to_string_pretty(&json) {
                    crate::config::atomic_write(&path, pretty.as_bytes())?;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prebuilts_contain_all_requested_personas() {
        let prebuilts = Persona::prebuilts();
        let ids: Vec<&str> = prebuilts.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"nyx"));
        assert!(ids.contains(&"iris"));
        assert!(ids.contains(&"echo"));
        assert!(ids.contains(&"cipher"));
        assert!(ids.contains(&"vesper"));
        assert!(ids.contains(&"jarvis"));
        assert!(ids.contains(&"mira"));
        assert!(ids.contains(&"hal"));
        assert!(ids.contains(&"cora"));
    }

    #[test]
    fn alias_generator_creates_reasonable_phonetics() {
        let aliases = Persona::generate_aliases("Zephyr");
        assert!(aliases.contains(&"zephyr".to_string()));
        assert!(aliases
            .iter()
            .any(|a| a.contains("zef") || a.contains("sef")));

        let jarvis_aliases = Persona::generate_aliases("Jarvis");
        assert!(jarvis_aliases.contains(&"jarvis".to_string()));
        assert!(jarvis_aliases.contains(&"harvest".to_string()));
    }

    #[test]
    fn slugify_produces_clean_ids() {
        assert_eq!(Persona::slugify("My Voice Bot"), "my-voice-bot");
        assert_eq!(Persona::slugify("Cora"), "cora");
        assert_eq!(Persona::slugify("Agent 007"), "agent-007");
    }
}
