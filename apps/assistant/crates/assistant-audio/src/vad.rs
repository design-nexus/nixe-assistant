//! Streaming VAD with hysteresis. Silero (ONNX) or energy/RMS, same API.

use crate::silero::{Silero, SileroState};
use assistant_core::config::VadCfg;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadDecision {
    Silence,
    SpeechStart,
    Speech,
    SpeechEnd,
}

pub trait VadGate {
    fn reset(&mut self);
    fn push(&mut self, samples: &[i16]) -> VadDecision;
    fn heard_speech(&self) -> bool;
}

pub enum StreamingVad {
    Energy(EnergyVad),
    Silero(SileroGate),
}

impl StreamingVad {
    pub fn new(
        vad: &VadCfg,
        silero: Option<Arc<Silero>>,
        hangover_ms: u64,
        start_confirm: u32,
        energy_threshold: f32,
    ) -> Self {
        if vad.wants_silero() {
            if let Some(model) = silero {
                return Self::Silero(SileroGate::new(
                    model,
                    vad.threshold,
                    hangover_ms,
                    32,
                    start_confirm,
                ));
            }
        }
        Self::Energy(EnergyVad::with_confirm(
            energy_threshold,
            hangover_ms,
            32,
            start_confirm,
        ))
    }
}

impl VadGate for StreamingVad {
    fn reset(&mut self) {
        match self {
            Self::Energy(v) => v.reset(),
            Self::Silero(v) => v.reset(),
        }
    }

    fn push(&mut self, samples: &[i16]) -> VadDecision {
        match self {
            Self::Energy(v) => v.push(samples),
            Self::Silero(v) => v.push(samples),
        }
    }

    fn heard_speech(&self) -> bool {
        match self {
            Self::Energy(v) => v.heard_speech(),
            Self::Silero(v) => v.heard_speech(),
        }
    }
}

pub struct EnergyVad {
    threshold: f32,
    hangover_frames: u32,
    start_confirm: u32,
    voiced_run: u32,
    silence_run: u32,
    in_speech: bool,
    ever_speech: bool,
}

impl EnergyVad {
    pub fn new(threshold: f32, hangover_ms: u64, frame_ms: u64) -> Self {
        Self::with_confirm(threshold, hangover_ms, frame_ms, 1)
    }

    pub fn with_confirm(
        threshold: f32,
        hangover_ms: u64,
        frame_ms: u64,
        start_confirm: u32,
    ) -> Self {
        let hangover_frames = (hangover_ms / frame_ms.max(1)).max(1) as u32;
        Self {
            threshold,
            hangover_frames,
            start_confirm: start_confirm.max(1),
            voiced_run: 0,
            silence_run: 0,
            in_speech: false,
            ever_speech: false,
        }
    }

    pub fn reset(&mut self) {
        self.voiced_run = 0;
        self.silence_run = 0;
        self.in_speech = false;
        self.ever_speech = false;
    }

    pub fn push(&mut self, samples: &[i16]) -> VadDecision {
        if samples.is_empty() {
            return if self.in_speech {
                VadDecision::Speech
            } else {
                VadDecision::Silence
            };
        }
        let mut acc = 0.0f32;
        for s in samples {
            let x = *s as f32 / 32768.0;
            acc += x * x;
        }
        let rms = (acc / samples.len() as f32).sqrt();
        let voiced = rms >= self.threshold;

        if voiced {
            self.silence_run = 0;
            self.voiced_run = self.voiced_run.saturating_add(1);
            if !self.in_speech {
                if self.voiced_run < self.start_confirm {
                    return VadDecision::Silence;
                }
                self.in_speech = true;
                self.ever_speech = true;
                return VadDecision::SpeechStart;
            }
            return VadDecision::Speech;
        }
        self.voiced_run = 0;

        if self.in_speech {
            self.silence_run += 1;
            if self.silence_run >= self.hangover_frames {
                self.in_speech = false;
                return VadDecision::SpeechEnd;
            }
            return VadDecision::Speech;
        }
        VadDecision::Silence
    }

    pub fn heard_speech(&self) -> bool {
        self.ever_speech
    }
}

impl VadGate for EnergyVad {
    fn reset(&mut self) {
        EnergyVad::reset(self);
    }

    fn push(&mut self, samples: &[i16]) -> VadDecision {
        EnergyVad::push(self, samples)
    }

    fn heard_speech(&self) -> bool {
        EnergyVad::heard_speech(self)
    }
}

pub struct SileroGate {
    model: Arc<Silero>,
    state: SileroState,
    pending: Vec<i16>,
    threshold: f32,
    hangover_frames: u32,
    start_confirm: u32,
    voiced_run: u32,
    silence_run: u32,
    in_speech: bool,
    ever_speech: bool,
}

impl SileroGate {
    pub fn new(
        model: Arc<Silero>,
        threshold: f32,
        hangover_ms: u64,
        frame_ms: u64,
        start_confirm: u32,
    ) -> Self {
        let hangover_frames = (hangover_ms / frame_ms.max(1)).max(1) as u32;
        let state = model.new_state();
        Self {
            model,
            state,
            pending: Vec::new(),
            threshold,
            hangover_frames,
            start_confirm: start_confirm.max(1),
            voiced_run: 0,
            silence_run: 0,
            in_speech: false,
            ever_speech: false,
        }
    }

    fn decide(&mut self, voiced: bool) -> VadDecision {
        if voiced {
            self.silence_run = 0;
            self.voiced_run = self.voiced_run.saturating_add(1);
            if !self.in_speech {
                if self.voiced_run < self.start_confirm {
                    return VadDecision::Silence;
                }
                self.in_speech = true;
                self.ever_speech = true;
                return VadDecision::SpeechStart;
            }
            return VadDecision::Speech;
        }
        self.voiced_run = 0;
        if self.in_speech {
            self.silence_run += 1;
            if self.silence_run >= self.hangover_frames {
                self.in_speech = false;
                return VadDecision::SpeechEnd;
            }
            return VadDecision::Speech;
        }
        VadDecision::Silence
    }
}

impl VadGate for SileroGate {
    fn reset(&mut self) {
        self.state.reset();
        self.pending.clear();
        self.voiced_run = 0;
        self.silence_run = 0;
        self.in_speech = false;
        self.ever_speech = false;
    }

    fn push(&mut self, samples: &[i16]) -> VadDecision {
        if samples.is_empty() && self.pending.is_empty() {
            return if self.in_speech {
                VadDecision::Speech
            } else {
                VadDecision::Silence
            };
        }
        self.pending.extend_from_slice(samples);
        let chunk = self.model.chunk_len();
        let mut last = if self.in_speech {
            VadDecision::Speech
        } else {
            VadDecision::Silence
        };
        while self.pending.len() >= chunk {
            let frame: Vec<i16> = self.pending.drain(..chunk).collect();
            let prob = match self.model.infer(&frame, &mut self.state) {
                Ok(p) => p,
                Err(err) => {
                    tracing::warn!(%err, "silero infer");
                    0.0
                }
            };
            last = self.decide(prob >= self.threshold);
        }
        last
    }

    fn heard_speech(&self) -> bool {
        self.ever_speech
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hangover_ends_speech() {
        let mut vad = EnergyVad::new(0.05, 64, 32);
        let loud = vec![10_000i16; 512];
        let quiet = vec![0i16; 512];
        assert!(matches!(vad.push(&loud), VadDecision::SpeechStart));
        assert!(matches!(vad.push(&quiet), VadDecision::Speech));
        assert!(matches!(vad.push(&quiet), VadDecision::SpeechEnd));
        assert!(vad.heard_speech());
    }
}
