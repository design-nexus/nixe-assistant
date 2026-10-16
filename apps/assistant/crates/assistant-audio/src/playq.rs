//! The playback queue, apart from PipeWire so it can be tested.
//!
//! Audio arrives as segments tagged with the reply (generation) and phrase they belong to.
//! The output callback pulls samples through [`PlayQueue::pull`], which applies the ducking
//! gain with a short ramp (no clicks) and records how far playback got, so the daemon knows
//! what the listener actually heard when a reply is interrupted. It also measures gaps: the
//! queue running dry between two phrases of the same reply, which the listener hears as an
//! extra pause.

use std::collections::VecDeque;

/// The playback rate.
const RATE: usize = 24_000;
/// Gain changes ramp over about 10 ms at the 24 kHz playback rate.
const RAMP_SAMPLES: f32 = 240.0;

struct Segment {
    generation: u64,
    phrase: u32,
    pcm: Vec<f32>,
    pos: usize,
}

/// How far playback got: the last sample sent to the speaker belonged to this phrase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayedTo {
    pub generation: u64,
    pub phrase: u32,
    /// Samples of that phrase played so far.
    pub samples: usize,
    /// The phrase's length.
    pub total: usize,
}

pub struct PlayQueue {
    segments: VecDeque<Segment>,
    queued: usize,
    gain: f32,
    target: f32,
    played: PlayedTo,
    paused: bool,
    /// The phrase that ended when the queue last ran dry, and the silent samples since.
    dry_after: Option<(u64, u32)>,
    dry: usize,
    /// Mid-reply gaps: generation and length in samples.
    gaps: Vec<(u64, usize)>,
}

impl Default for PlayQueue {
    fn default() -> Self {
        Self {
            segments: VecDeque::new(),
            queued: 0,
            gain: 1.0,
            target: 1.0,
            played: PlayedTo::default(),
            paused: false,
            dry_after: None,
            dry: 0,
            gaps: Vec::new(),
        }
    }
}

impl PlayQueue {
    pub fn push(&mut self, generation: u64, phrase: u32, pcm: Vec<f32>) {
        if pcm.is_empty() {
            return;
        }
        self.queued += pcm.len();
        self.segments.push_back(Segment { generation, phrase, pcm, pos: 0 });
    }

    /// Samples still to play.
    pub fn queued(&self) -> usize {
        self.queued
    }

    /// Drop everything queued.
    pub fn clear(&mut self) {
        self.segments.clear();
        self.queued = 0;
        self.dry_after = None;
    }

    /// Drop the audio of `generation` and older replies; newer audio keeps playing.
    pub fn cancel_through(&mut self, generation: u64) {
        self.segments.retain(|s| s.generation > generation);
        self.queued = self.segments.iter().map(|s| s.pcm.len() - s.pos).sum();
        self.gaps.retain(|(g, _)| *g > generation);
        if self.dry_after.is_some_and(|(g, _)| g <= generation) {
            self.dry_after = None;
        }
    }

    /// The gain to ramp to (1.0 normal, lower while ducked).
    pub fn set_gain(&mut self, target: f32) {
        self.target = target.clamp(0.0, 1.0);
    }

    /// Hold playback in place: silence goes out and nothing queued is used up.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn played(&self) -> PlayedTo {
        self.played
    }

    /// The gaps heard so far in `generation`'s reply, in ms; older replies' are dropped.
    pub fn take_gaps(&mut self, generation: u64) -> Vec<u32> {
        let gaps = self.gaps.iter().filter(|(g, _)| *g == generation).map(|(_, n)| (n * 1000 / RATE) as u32).collect();
        self.gaps.retain(|(g, _)| *g > generation);
        gaps
    }

    /// Fill `out` with the next samples, silence once the queue runs dry.
    pub fn pull(&mut self, out: &mut [f32]) {
        let step = 1.0 / RAMP_SAMPLES;
        if self.paused {
            out.fill(0.0);
            return;
        }
        for slot in out.iter_mut() {
            let Some(seg) = self.segments.front_mut() else {
                *slot = 0.0;
                if self.dry_after.is_some() {
                    self.dry += 1;
                }
                continue;
            };
            if let Some((generation, phrase)) = self.dry_after.take() {
                if seg.generation == generation && seg.phrase > phrase && self.dry > 0 {
                    self.gaps.push((generation, self.dry));
                }
            }
            self.dry = 0;
            let s = seg.pcm[seg.pos];
            seg.pos += 1;
            self.queued -= 1;
            self.played = PlayedTo { generation: seg.generation, phrase: seg.phrase, samples: seg.pos, total: seg.pcm.len() };
            if seg.pos == seg.pcm.len() {
                let ended = (seg.generation, seg.phrase);
                self.segments.pop_front();
                if self.segments.is_empty() {
                    self.dry_after = Some(ended);
                }
            }
            self.gain += (self.target - self.gain).clamp(-step, step);
            *slot = s * self.gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_in_order_and_tracks_progress() {
        let mut q = PlayQueue::default();
        q.push(1, 0, vec![0.5; 3]);
        q.push(1, 1, vec![0.25; 2]);
        assert_eq!(q.queued(), 5);
        let mut out = [0.0; 4];
        q.pull(&mut out);
        assert_eq!(out, [0.5, 0.5, 0.5, 0.25]);
        assert_eq!(q.played(), PlayedTo { generation: 1, phrase: 1, samples: 1, total: 2 });
        let mut out = [9.0; 3];
        q.pull(&mut out);
        assert_eq!(out, [0.25, 0.0, 0.0], "silence once dry");
        assert_eq!(q.queued(), 0);
        assert_eq!(q.played(), PlayedTo { generation: 1, phrase: 1, samples: 2, total: 2 });
    }

    #[test]
    fn cancelling_a_generation_keeps_newer_audio() {
        let mut q = PlayQueue::default();
        q.push(1, 0, vec![0.1; 4]);
        q.push(2, 0, vec![0.2; 2]);
        q.push(1, 1, vec![0.1; 4]);
        let mut out = [0.0; 1];
        q.pull(&mut out);
        q.cancel_through(1);
        assert_eq!(q.queued(), 2);
        let mut out = [0.0; 3];
        q.pull(&mut out);
        assert_eq!(out, [0.2, 0.2, 0.0]);
        // The record of what was heard survives the cancel.
        assert_eq!(q.played().generation, 2);
    }

    #[test]
    fn pause_holds_the_place() {
        let mut q = PlayQueue::default();
        q.push(1, 0, vec![0.5; 4]);
        let mut out = [0.0; 2];
        q.pull(&mut out);
        q.set_paused(true);
        let mut out = [9.0; 3];
        q.pull(&mut out);
        assert_eq!(out, [0.0; 3]);
        assert_eq!(q.queued(), 2, "nothing used up while paused");
        q.set_paused(false);
        let mut out = [0.0; 3];
        q.pull(&mut out);
        assert_eq!(out, [0.5, 0.5, 0.0]);
    }

    #[test]
    fn gain_ramps_without_jumping() {
        let mut q = PlayQueue::default();
        q.push(1, 0, vec![1.0; 1000]);
        q.set_gain(0.25);
        let mut out = vec![0.0; 1000];
        q.pull(&mut out);
        // No step between neighbours larger than the ramp, and it settles at the target.
        assert!(out.windows(2).all(|w| (w[0] - w[1]).abs() <= 1.0 / RAMP_SAMPLES + 1e-6));
        assert!((out[999] - 0.25).abs() < 1e-6);
        assert!(out[0] > 0.99, "starts from full volume");
    }

    #[test]
    fn gaps_between_phrases_of_a_reply() {
        let mut q = PlayQueue::default();
        q.push(1, 0, vec![0.5; 10]);
        q.push(1, 1, vec![0.5; 10]);
        let mut out = vec![0.0; 25];
        q.pull(&mut out);
        assert!(q.take_gaps(1).is_empty(), "contiguous phrases");
        // The next phrase arrives 2400 samples (100 ms) after the queue ran dry.
        q.push(2, 0, vec![0.5; 10]);
        let mut out = vec![0.0; 10];
        q.pull(&mut out);
        let mut out = vec![0.0; 2400];
        q.pull(&mut out);
        q.push(2, 1, vec![0.5; 10]);
        let mut out = vec![0.0; 10];
        q.pull(&mut out);
        assert_eq!(q.take_gaps(2), vec![100]);
        assert!(q.take_gaps(2).is_empty(), "taken once");
        // Silence after the last phrase, or before the next reply, isn't a gap.
        let mut out = vec![0.0; 4800];
        q.pull(&mut out);
        q.push(3, 0, vec![0.5; 10]);
        q.pull(&mut out);
        assert!(q.take_gaps(2).is_empty() && q.take_gaps(3).is_empty());
    }

    #[test]
    fn cancelled_replies_and_pauses_leave_no_gaps() {
        let mut q = PlayQueue::default();
        q.push(1, 0, vec![0.5; 10]);
        let mut out = vec![0.0; 500];
        q.pull(&mut out);
        q.push(1, 1, vec![0.5; 10]);
        q.pull(&mut out);
        q.cancel_through(1);
        assert!(q.take_gaps(1).is_empty(), "dropped with its reply");
        // A pause holds the queue; it isn't the queue running dry.
        q.push(2, 0, vec![0.5; 4]);
        q.push(2, 1, vec![0.5; 4]);
        let mut out = vec![0.0; 2];
        q.pull(&mut out);
        q.set_paused(true);
        let mut out = vec![0.0; 2400];
        q.pull(&mut out);
        q.set_paused(false);
        let mut out = vec![0.0; 10];
        q.pull(&mut out);
        assert!(q.take_gaps(2).is_empty());
    }
}
