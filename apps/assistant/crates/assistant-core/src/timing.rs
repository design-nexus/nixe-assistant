use crate::event::TimingStage;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct TurnTiming {
    origin: Instant,
    pub vad_end: Option<Instant>,
    pub stt_done: Option<Instant>,
    pub route_done: Option<Instant>,
    pub llm_first: Option<Instant>,
    pub tts_begin: Option<Instant>,
    pub audio_play: Option<Instant>,
}

impl TurnTiming {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            vad_end: None,
            stt_done: None,
            route_done: None,
            llm_first: None,
            tts_begin: None,
            audio_play: None,
        }
    }

    pub fn mark_vad_end(&mut self) {
        self.vad_end = Some(Instant::now());
        self.origin = self.vad_end.unwrap();
    }

    fn ms(&self, t: Option<Instant>) -> u64 {
        t.map(|x| x.saturating_duration_since(self.origin).as_millis() as u64)
            .unwrap_or(0)
    }

    pub fn stages(&self) -> Vec<TimingStage> {
        vec![
            TimingStage {
                name: "VAD end".into(),
                ms: 0,
            },
            TimingStage {
                name: "STT complete".into(),
                ms: self.ms(self.stt_done),
            },
            TimingStage {
                name: "routing".into(),
                ms: self.ms(self.route_done),
            },
            TimingStage {
                name: "LLM first token".into(),
                ms: self.ms(self.llm_first),
            },
            TimingStage {
                name: "TTS begins".into(),
                ms: self.ms(self.tts_begin),
            },
            TimingStage {
                name: "audio playback".into(),
                ms: self.ms(self.audio_play),
            },
        ]
    }

    pub fn format_trace(&self) -> String {
        let stages = self.stages();
        let total = stages.last().map(|s| s.ms).unwrap_or(0);
        let mut lines = String::from("Nyx timing\n");
        for s in &stages {
            lines.push_str(&format!("  {:<18} {:>5} ms\n", s.name, s.ms));
        }
        lines.push_str(&format!("  {:<18} {:>5} ms\n", "TOTAL", total));
        lines
    }
}

impl Default for TurnTiming {
    fn default() -> Self {
        Self::new()
    }
}
