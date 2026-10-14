//! nixe-assistant-asr: a warm streaming recognizer (NVIDIA Nemotron streaming 0.6B through
//! sherpa-onnx) behind the frame protocol in `lib.rs`.
//!
//!   nixe-assistant-asr --model DIR [--threads N] [--provider cpu|cuda]

use anyhow::{bail, Context, Result};
use assistant_core::asr::{Frame, Kind, Reply, DECODER, ENCODER, JOINER, LEAD_PAD_SAMPLES, SAMPLE_RATE, TAIL_PAD_SAMPLES, TOKENS};
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

struct Args {
    model: PathBuf,
    threads: i32,
    provider: String,
}

fn args() -> Result<Args> {
    let mut a = Args { model: PathBuf::new(), threads: 2, provider: "cpu".into() };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--model" => a.model = value()?.into(),
            "--threads" => a.threads = value()?.parse().context("--threads")?,
            "--provider" => a.provider = value()?,
            _ => bail!("unknown argument {flag}"),
        }
    }
    if a.model.as_os_str().is_empty() {
        bail!("usage: nixe-assistant-asr --model DIR [--threads N] [--provider cpu|cuda]");
    }
    Ok(a)
}

fn recognizer(a: &Args) -> Result<OnlineRecognizer> {
    let file = |name: &str| -> Result<Option<String>> {
        let p = a.model.join(name);
        anyhow::ensure!(p.is_file(), "{} is missing", p.display());
        Ok(Some(p.to_string_lossy().into_owned()))
    };
    let mut config = OnlineRecognizerConfig::default();
    config.model_config.transducer.encoder = file(ENCODER)?;
    config.model_config.transducer.decoder = file(DECODER)?;
    config.model_config.transducer.joiner = file(JOINER)?;
    config.model_config.tokens = file(TOKENS)?;
    config.model_config.num_threads = a.threads.max(1);
    config.model_config.provider = Some(a.provider.clone());
    config.decoding_method = Some("greedy_search".into());
    // The daemon's VAD decides where an utterance ends.
    config.enable_endpoint = false;
    OnlineRecognizer::create(&config).context("the recognizer could not be created")
}

struct Utterance {
    id: u32,
    stream: OnlineStream,
    samples: u64,
    last: String,
}

fn send(out: &mut impl Write, reply: &Reply) -> Result<()> {
    serde_json::to_writer(&mut *out, reply)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

fn ms(samples: u64) -> u64 {
    samples * 1000 / SAMPLE_RATE as u64
}

fn feed(rec: &OnlineRecognizer, u: &mut Utterance, samples: &[i16]) {
    let f: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
    u.stream.accept_waveform(SAMPLE_RATE as i32, &f);
    u.samples += samples.len() as u64;
    while rec.is_ready(&u.stream) {
        rec.decode(&u.stream);
    }
}

fn text(rec: &OnlineRecognizer, u: &Utterance) -> String {
    rec.get_result(&u.stream).map(|r| r.text.trim().to_string()).unwrap_or_default()
}

fn main() -> Result<()> {
    let out = std::io::stdout();
    let mut out = BufWriter::new(out.lock());
    let a = args()?;
    let started = Instant::now();
    let rec = match recognizer(&a) {
        Ok(rec) => rec,
        Err(e) => {
            send(&mut out, &Reply::Error { message: format!("{e:#}") })?;
            return Err(e);
        }
    };
    send(&mut out, &Reply::Ready { load_ms: started.elapsed().as_millis() as u64 })?;
    let stdin = std::io::stdin();
    let mut input = std::io::BufReader::new(stdin.lock());
    let mut current: Option<Utterance> = None;
    while let Some(frame) = Frame::read(&mut input)? {
        match frame.kind {
            Kind::Audio => {
                if current.as_ref().is_none_or(|u| u.id != frame.utterance) {
                    let mut u = Utterance { id: frame.utterance, stream: rec.create_stream(), samples: 0, last: String::new() };
                    feed(&rec, &mut u, &vec![0i16; LEAD_PAD_SAMPLES]);
                    // audio_ms counts the caller's audio, not the padding.
                    u.samples = 0;
                    current = Some(u);
                }
                let u = current.as_mut().unwrap();
                feed(&rec, u, &frame.samples);
                let now = text(&rec, u);
                if now != u.last {
                    send(&mut out, &Reply::Partial { u: u.id, text: now.clone(), audio_ms: ms(u.samples) })?;
                    u.last = now;
                }
            }
            Kind::Finish => {
                let t = Instant::now();
                let (text, audio_ms) = match current.take() {
                    Some(mut u) if u.id == frame.utterance => {
                        let spoken = u.samples;
                        feed(&rec, &mut u, &vec![0i16; TAIL_PAD_SAMPLES]);
                        u.stream.input_finished();
                        while rec.is_ready(&u.stream) {
                            rec.decode(&u.stream);
                        }
                        (text(&rec, &u), ms(spoken))
                    }
                    // Nothing was heard for it (or it was replaced): an empty final.
                    other => {
                        current = other;
                        (String::new(), 0)
                    }
                };
                send(&mut out, &Reply::Final { u: frame.utterance, text, audio_ms, decode_ms: t.elapsed().as_millis() as u64 })?;
            }
            Kind::Abort => {
                if current.as_ref().is_some_and(|u| u.id == frame.utterance) {
                    current = None;
                }
            }
        }
    }
    Ok(())
}
