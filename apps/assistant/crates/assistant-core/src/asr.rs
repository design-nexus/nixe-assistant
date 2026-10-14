//! The streaming recognizer's wire protocol, shared by the worker (`nixe-assistant-asr`)
//! and the daemon's client.
//!
//! The recognizer runs in its own process: sherpa-onnx brings its own ONNX Runtime, which
//! must not meet the one Kokoro and Silero load, and a crash there must not take the
//! daemon down.
//!
//! stdin carries binary frames: `kind: u8`, `utterance: u32`, `samples: u32`, then that many
//! little-endian i16 samples at 16 kHz mono. stdout carries one JSON [`Reply`] per line.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

pub const SAMPLE_RATE: u32 = 16_000;
/// Silence fed after the last word before the final result. The model decodes in 560 ms
/// chunks with right context, so a short word at the very end ("stop") is lost without it.
pub const TAIL_PAD_SAMPLES: usize = 16_000;
/// Silence fed before an utterance's first audio: without some lead-in the first word of a
/// take that starts right at speech ("what is…") can be dropped.
pub const LEAD_PAD_SAMPLES: usize = 4_000;
/// Model files inside the model directory.
pub const ENCODER: &str = "encoder.int8.onnx";
pub const DECODER: &str = "decoder.int8.onnx";
pub const JOINER: &str = "joiner.int8.onnx";
pub const TOKENS: &str = "tokens.txt";
/// The model `models fetch asr` installs, and where it comes from.
pub const MODEL_NAME: &str = "sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25";
pub const MODEL_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25.tar.bz2";
/// Frames larger than this are refused (60 s of audio), so a bad length can't exhaust memory.
const MAX_SAMPLES: u32 = SAMPLE_RATE * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Audio for the utterance. A new utterance id starts a fresh stream.
    Audio = 0,
    /// The utterance ended: pad, decode the rest and reply with its final text.
    Finish = 1,
    /// Drop the utterance without a final.
    Abort = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: Kind,
    pub utterance: u32,
    pub samples: Vec<i16>,
}

impl Frame {
    pub fn audio(utterance: u32, samples: &[i16]) -> Self {
        Self { kind: Kind::Audio, utterance, samples: samples.to_vec() }
    }
    pub fn finish(utterance: u32) -> Self {
        Self { kind: Kind::Finish, utterance, samples: Vec::new() }
    }
    pub fn abort(utterance: u32) -> Self {
        Self { kind: Kind::Abort, utterance, samples: Vec::new() }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(9 + self.samples.len() * 2);
        out.push(self.kind as u8);
        out.extend_from_slice(&self.utterance.to_le_bytes());
        out.extend_from_slice(&(self.samples.len() as u32).to_le_bytes());
        for s in &self.samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(&self.encode())?;
        w.flush()
    }

    /// The next frame, or `None` at a clean end of input.
    pub fn read(r: &mut impl Read) -> io::Result<Option<Self>> {
        let mut head = [0u8; 9];
        match r.read_exact(&mut head) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let kind = match head[0] {
            0 => Kind::Audio,
            1 => Kind::Finish,
            2 => Kind::Abort,
            k => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown frame kind {k}"))),
        };
        let utterance = u32::from_le_bytes(head[1..5].try_into().unwrap());
        let n = u32::from_le_bytes(head[5..9].try_into().unwrap());
        if n > MAX_SAMPLES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("frame of {n} samples")));
        }
        let mut raw = vec![0u8; n as usize * 2];
        r.read_exact(&mut raw)?;
        let samples = raw.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        Ok(Some(Self { kind, utterance, samples }))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reply {
    /// The model is loaded; frames may follow.
    Ready { load_ms: u64 },
    /// The words so far; sent whenever they change.
    Partial { u: u32, text: String, audio_ms: u64 },
    /// The whole utterance, after `Finish`.
    Final { u: u32, text: String, audio_ms: u64, decode_ms: u64 },
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let frames = [Frame::audio(7, &[0, 1, -1, i16::MAX, i16::MIN]), Frame::finish(7), Frame::abort(8), Frame::audio(9, &[])];
        let mut buf = Vec::new();
        for f in &frames {
            f.write(&mut buf).unwrap();
        }
        let mut r = &buf[..];
        for f in &frames {
            assert_eq!(Frame::read(&mut r).unwrap().as_ref(), Some(f));
        }
        assert_eq!(Frame::read(&mut r).unwrap(), None);
    }

    #[test]
    fn bad_frames_are_refused() {
        assert!(Frame::read(&mut &[9u8, 0, 0, 0, 0, 0, 0, 0, 0][..]).is_err());
        let mut huge = vec![0u8];
        huge.extend_from_slice(&1u32.to_le_bytes());
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(Frame::read(&mut &huge[..]).is_err());
        // A frame cut short is an error, not a clean end.
        let mut cut = Frame::audio(1, &[1, 2, 3]).encode();
        cut.truncate(cut.len() - 1);
        assert!(Frame::read(&mut &cut[..]).is_err());
    }

    #[test]
    fn replies_are_tagged_json() {
        let r = Reply::Partial { u: 3, text: "stop".into(), audio_ms: 640 };
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, r#"{"kind":"partial","u":3,"text":"stop","audio_ms":640}"#);
        assert_eq!(serde_json::from_str::<Reply>(&s).unwrap(), r);
    }
}
