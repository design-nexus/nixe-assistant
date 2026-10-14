//! The streaming recognizer end to end: the worker binary, the model and the client.
//! Skipped when the model isn't installed (`nixe-assistant models fetch asr`).
use assistant_agent::asr::AsrWorker;
use assistant_core::Config;

#[tokio::test]
async fn hears_speech_and_ignores_silence() {
    let worker = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/nixe-assistant-asr");
    let model = assistant_core::paths::models_dir().join(assistant_core::asr::MODEL_NAME);
    let speech = std::env::var("ASR_TEST_WAV").ok();
    if !worker.is_file() || !model.is_dir() {
        eprintln!("skipped: build -p assistant-asr --release and fetch the model");
        return;
    }
    // The client finds the worker next to the running binary; tests point it at the build.
    let mut cfg = Config::default();
    cfg.stt.model = model.display().to_string();
    let w = match AsrWorker::start_with(&cfg, &worker).await {
        Ok(w) => w,
        Err(e) => panic!("worker didn't start: {e:#}"),
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    let u = w.begin().await.unwrap();
    w.feed(u, &vec![0i16; 16_000]);
    assert_eq!(w.finish(u, &cancel).await.unwrap(), "", "silence has no words");
    if let Some(path) = speech {
        let mut r = hound::WavReader::open(path).unwrap();
        let pcm: Vec<i16> = r.samples::<i16>().map(Result::unwrap).collect();
        let mut partials = w.subscribe();
        let u = w.begin().await.unwrap();
        for chunk in pcm.chunks(512) {
            w.feed(u, chunk);
        }
        let text = w.finish(u, &cancel).await.unwrap();
        assert!(!text.is_empty());
        let mut saw_partial = false;
        while let Ok(r) = partials.try_recv() {
            saw_partial |= matches!(r, assistant_core::asr::Reply::Partial { .. });
        }
        assert!(saw_partial, "partials while it listened");
    }
}
