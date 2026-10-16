use ratatui::{buffer::Buffer, style::Color};
pub const SIZES: &[(u16, u16)] = &[
    (120, 40),
    (100, 30),
    (80, 24),
    (60, 20),
    (30, 10),
    (20, 8),
    (1, 1),
    (120, 34),
];
pub fn text(buffer: &Buffer) -> String {
    buffer
        .content
        .chunks(buffer.area.width.max(1) as usize)
        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn color(color: Color, foreground: bool) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => {
            if foreground {
                "#ececf4".into()
            } else {
                "#171820".into()
            }
        }
    }
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
pub fn capture(app: &str, state: &str, buffer: &Buffer) {
    let Ok(dir) = std::env::var("AURORA_RENDER_DIR") else {
        return;
    };
    let (w, h) = (buffer.area.width, buffer.area.height);
    let path = std::path::Path::new(&dir);
    std::fs::create_dir_all(path).unwrap();
    std::fs::write(
        path.join(format!("{app}-{state}-{w}x{h}.txt")),
        text(buffer),
    )
    .unwrap();
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}"><rect width="100%" height="100%" fill="#171820"/><g font-family="DejaVu Sans Mono" font-size="14">"##,
        w as usize * 9,
        h as usize * 19
    );
    for y in 0..h {
        for x in 0..w {
            let c = &buffer[(x, y)];
            let (px, py) = (x as usize * 9, y as usize * 19);
            svg.push_str(&format!(
                r#"<rect x="{px}" y="{py}" width="9" height="19" fill="{}"/>"#,
                color(c.bg, false)
            ));
        }
    }
    for y in 0..h {
        for x in 0..w {
            let c = &buffer[(x, y)];
            if c.symbol().trim().is_empty() {
                continue;
            }
            svg.push_str(&format!(
                r#"<text x="{}" y="{}" fill="{}" font-weight="{}" font-style="{}" text-decoration="{}">{}</text>"#,
                x as usize * 9,
                y as usize * 19 + 14,
                color(c.fg, true),
                if c.modifier.contains(ratatui::style::Modifier::BOLD) { "bold" } else { "normal" },
                if c.modifier.contains(ratatui::style::Modifier::ITALIC) { "italic" } else { "normal" },
                if c.modifier.contains(ratatui::style::Modifier::UNDERLINED) { "underline" } else { "none" },
                escape(c.symbol())
            ));
        }
    }
    svg.push_str("</g></svg>");
    std::fs::write(path.join(format!("{app}-{state}-{w}x{h}.svg")), svg).unwrap();
}

pub fn verify_footer(buffer: &Buffer) {
    let (w, h) = (buffer.area.width, buffer.area.height);
    if w < 60 || h < 20 {
        return;
    }
    let bg = Color::Rgb(0x14, 0x15, 0x1d);
    let accent = Color::Rgb(0xb5, 0xa0, 0xff);
    let start_button = (4..11)
        .map(|x| buffer[(x, h - 3)].symbol())
        .collect::<String>()
        == " Start ";
    for y in h - 3..h - 1 {
        for x in 4..w - 4 {
            if y == h - 3 && x < 11 && start_button {
                assert_eq!(buffer[(x, y)].bg, accent);
                continue;
            }
            assert_eq!(
                buffer[(x, y)].bg,
                bg,
                "footer background at {x},{y} in {w}x{h}"
            );
        }
    }
    let mut open = false;
    let mut count = 0;
    for x in 4..w - 4 {
        let cell = &buffer[(x, h - 3)];
        if cell.symbol() == "(" {
            assert!(!open, "nested keycap");
            open = true;
            count += 1;
        }
        if open {
            assert_eq!(cell.fg, accent, "unstyled keycap at {x} in {w}x{h}");
        }
        if cell.symbol() == ")" {
            assert!(open, "unmatched closing keycap");
            open = false;
        }
    }
    assert!(!open, "clipped keycap in {w}x{h}");
    assert!(count >= 2, "missing shortcuts in {w}x{h}");
}

/// Interactive visual QA uses the same populated fixtures as the render audit.
/// No backend ticks or operation executors run in this harness.
pub fn live_preview<A>(
    app: &mut A,
    draw: impl Fn(&mut ratatui::Frame, &mut A),
    navigate: impl Fn(&mut A, crossterm::event::KeyEvent),
) {
    use crossterm::event::{self, Event, KeyCode, KeyModifiers};
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            ratatui::restore();
        }
    }
    let mut terminal = ratatui::init();
    let _restore = Restore;
    loop {
        let rendered = terminal.draw(|f| draw(f, app)).unwrap();
        crate::aurora::paint_wordmark(rendered.buffer).unwrap();
        if event::poll(std::time::Duration::from_millis(100)).unwrap() {
            if let Event::Key(key) = event::read().unwrap() {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }
                navigate(app, key);
            }
        }
    }
}
