//! Text on screen with `tesseract`: words with boxes, grouped into lines. Boxes are image
//! pixels of the shot they came from.

use crate::Rect;
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Line {
    pub text: String,
    pub bounds: Rect,
    /// Mean word confidence, 0–100.
    pub confidence: f32,
}

/// Words below this confidence are dropped (icons and textures read as junk).
const MIN_CONFIDENCE: f32 = 45.0;

/// Parse `tesseract … tsv` output into lines.
pub fn parse_tsv(tsv: &str) -> Vec<Line> {
    let mut lines: Vec<((i32, i32, i32, i32), Line, u32)> = Vec::new();
    for row in tsv.lines().skip(1) {
        let f: Vec<&str> = row.split('\t').collect();
        if f.len() < 12 || f[0] != "5" {
            continue;
        }
        let num = |i: usize| f[i].parse::<i32>().unwrap_or(0);
        let conf: f32 = f[10].parse().unwrap_or(-1.0);
        let word = f[11].trim();
        if word.is_empty() || conf < MIN_CONFIDENCE {
            continue;
        }
        let key = (num(1), num(2), num(3), num(4));
        let r = Rect { x: num(6), y: num(7), w: num(8), h: num(9) };
        match lines.iter_mut().find(|(k, ..)| *k == key) {
            Some((_, line, n)) => {
                line.text.push(' ');
                line.text.push_str(word);
                let (x0, y0) = (line.bounds.x.min(r.x), line.bounds.y.min(r.y));
                let (x1, y1) = ((line.bounds.x + line.bounds.w).max(r.x + r.w), (line.bounds.y + line.bounds.h).max(r.y + r.h));
                line.bounds = Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
                line.confidence = (line.confidence * *n as f32 + conf) / (*n + 1) as f32;
                *n += 1;
            }
            None => lines.push((key, Line { text: word.to_string(), bounds: r, confidence: conf }, 1)),
        }
    }
    lines.into_iter().map(|(_, l, _)| l).collect()
}

/// Read the text in a PNG.
pub async fn read(png: &[u8]) -> Result<Vec<Line>> {
    let mut child = tokio::process::Command::new("tesseract")
        // Sparse text: UIs are scattered labels, not paragraphs.
        .args(["stdin", "stdout", "--psm", "11", "-l", "eng", "tsv"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("tesseract isn't available")?;
    let mut stdin = child.stdin.take().context("tesseract stdin")?;
    let data = png.to_vec();
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&data).await;
    });
    let out = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output())
        .await
        .context("tesseract timed out")??;
    let _ = writer.await;
    ensure!(out.status.success(), "tesseract failed");
    Ok(parse_tsv(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_words_into_lines() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t800\t600\t-1\t\n\
5\t1\t1\t1\t1\t1\t10\t20\t40\t12\t95.1\tSave\n\
5\t1\t1\t1\t1\t2\t55\t21\t30\t12\t90.0\tfile\n\
5\t1\t2\t1\t1\t1\t300\t400\t20\t20\t12.0\t~#\n\
5\t1\t2\t1\t1\t2\t330\t400\t60\t14\t88.0\tCancel\n";
        let lines = parse_tsv(tsv);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Save file");
        assert_eq!(lines[0].bounds, Rect { x: 10, y: 20, w: 75, h: 13 });
        assert!((lines[0].confidence - 92.55).abs() < 0.01);
        // The low-confidence junk word is dropped, its line keeps the real word.
        assert_eq!(lines[1].text, "Cancel");
        assert_eq!(lines[1].bounds, Rect { x: 330, y: 400, w: 60, h: 14 });
    }
}
