//! Energy VAD with hysteresis. Silero can replace this behind the same API.

pub enum VadDecision {
    Silence,
    SpeechStart,
    Speech,
    SpeechEnd,
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
