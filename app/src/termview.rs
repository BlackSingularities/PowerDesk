//! Wbudowany terminal (Terminal.svelte): panel pod listą plików, PowerShell przez ConPTY
//! (`powerdesk_core::terminal`), ekran emulowany przez `vt100` i rysowany w Slint komórka po komórce.
//! Sesja żyje po ukryciu panelu; terminal może iść za folderem otwartym w aplikacji.

use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

use powerdesk_core::events::AppHandle as Core;
use powerdesk_core::{clip, terminal};
use slint::{Color, ComponentHandle, ModelRc, VecModel};
use vt100::Color as VtColor;

use crate::{Shared, TermLine, TermRun, TermState, Theme};

struct St {
    parser: vt100::Parser,
    id: Option<u64>,
    cwd: String,
    cols: u16,
    rows: u16,
    /// Zaznaczenie w komórkach widocznego ekranu: (wiersz, kolumna) początku i końca.
    sel: Option<((u16, u16), (u16, u16))>,
    dirty: bool,
}

impl Default for St {
    fn default() -> Self {
        St { parser: vt100::Parser::new(24, 80, 5000), id: None, cwd: String::new(), cols: 0, rows: 0, sel: None, dirty: false }
    }
}

thread_local! {
    static ST: RefCell<St> = RefCell::new(St::default());
    static TIMER: slint::Timer = slint::Timer::default();
}

fn with_ui(s: &Shared, f: impl FnOnce(&TermState)) {
    if let Some(ui) = s.borrow().ui.upgrade() {
        f(&ui.global::<TermState>());
    }
}

fn local(p: &str) -> bool {
    !p.is_empty() && !p.starts_with("ofp:") && !crate::archives::browsing(p) && Path::new(p).is_dir()
}

fn active_path(s: &Shared) -> String {
    crate::tab_mut(s, |t| t.path.clone())
}

pub fn toggle(s: &Shared) {
    let open = {
        let mut b = s.borrow_mut();
        b.prefs.terminal_open = !b.prefs.terminal_open;
        b.prefs.save();
        b.prefs.terminal_open
    };
    show(s, open);
}

/// Stan panelu z ustawień (przy starcie).
pub fn restore(s: &Shared) {
    let (open, h) = {
        let b = s.borrow();
        (b.prefs.terminal_open, b.prefs.terminal_height)
    };
    with_ui(s, |g| g.set_height(h));
    show(s, open);
}

fn show(s: &Shared, open: bool) {
    with_ui(s, |g| {
        g.set_open(open);
        if open {
            g.set_focus_req(g.get_focus_req() + 1);
        }
    });
}

fn start(s: &Shared) {
    let path = active_path(s);
    let cwd = if local(&path) { path } else { String::new() };
    let (cols, rows, old) = ST.with(|st| {
        let mut st = st.borrow_mut();
        let old = st.id.take();
        st.parser = vt100::Parser::new(st.rows.max(2), st.cols.max(10), 5000);
        st.sel = None;
        st.cwd = cwd.clone();
        (st.cols.max(10), st.rows.max(2), old)
    });
    if let Some(id) = old {
        terminal::kill(id);
    }
    match terminal::start(Core, &cwd, cols, rows) {
        Ok(id) => ST.with(|st| st.borrow_mut().id = Some(id)),
        Err(e) => crate::transfers::toast(s, powerdesk_core::trf!("Nie udało się uruchomić terminala: {e}"), crate::transfers::Kind::Error, None),
    }
    with_ui(s, |g| g.set_cwd(if cwd.is_empty() { crate::tr("Terminal").into() } else { cwd.into() }));
    render(s);
}

/// Zdarzenie „term-data” – porcja wyjścia powłoki.
pub fn on_data(s: &Shared, p: serde_json::Value) {
    let id = p["id"].as_u64();
    let data = p["data"].as_str().unwrap_or_default();
    let mine = ST.with(|st| {
        let mut st = st.borrow_mut();
        if st.id != id {
            return false;
        }
        st.parser.process(data.as_bytes());
        st.dirty = true;
        true
    });
    if mine {
        schedule(s);
    }
}

pub fn on_exit(s: &Shared, p: serde_json::Value) {
    let mine = ST.with(|st| {
        let mut st = st.borrow_mut();
        if st.id.is_none() || st.id != p.as_u64() {
            return false;
        }
        st.id = None;
        st.parser.process(crate::tr("\r\n\x1b[2m[proces zakończony – Enter uruchomi nowy]\x1b[0m\r\n").as_bytes());
        true
    });
    if mine {
        render(s);
    }
}

/// Wyjście przychodzi kawałkami – rysujemy najwyżej co klatkę.
fn schedule(s: &Shared) {
    TIMER.with(|t| {
        if t.running() {
            return;
        }
        let s = s.clone();
        t.start(slint::TimerMode::SingleShot, Duration::from_millis(16), move || {
            if ST.with(|st| std::mem::take(&mut st.borrow_mut().dirty)) {
                render(&s);
            }
        });
    });
}

/// Paleta Campbell (Windows Terminal), sześcian 6×6×6 i odcienie szarości.
fn idx_color(i: u8) -> Color {
    const BASE: [u32; 16] = [0x0c0c0c, 0xc50f1f, 0x13a10e, 0xc19c00, 0x0037da, 0x881798, 0x3a96dd, 0xcccccc, 0x767676, 0xe74856, 0x16c60c, 0xf9f1a5, 0x3b78ff, 0xb4009e, 0x61d6d6, 0xf2f2f2];
    let hex = |h: u32| Color::from_rgb_u8((h >> 16) as u8, (h >> 8) as u8, h as u8);
    match i {
        0..=15 => hex(BASE[i as usize]),
        16..=231 => {
            let v = i - 16;
            let c = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
            Color::from_rgb_u8(c(v / 36), c(v / 6 % 6), c(v % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            Color::from_rgb_u8(g, g, g)
        }
    }
}

fn resolve(c: VtColor, default: Color) -> Color {
    match c {
        VtColor::Default => default,
        VtColor::Idx(i) => idx_color(i),
        VtColor::Rgb(r, g, b) => Color::from_rgb_u8(r, g, b),
    }
}

fn render(s: &Shared) {
    let Some(ui) = s.borrow().ui.upgrade() else { return };
    let light = ui.global::<Theme>().get_light();
    let (dfg, dbg) = if light { (Color::from_rgb_u8(0x11, 0x11, 0x11), Color::from_rgb_u8(0xf6, 0xf6, 0xf7)) } else { (Color::from_rgb_u8(0xff, 0xff, 0xff), Color::from_rgb_u8(0, 0, 0)) };
    ST.with(|st| {
        let st = st.borrow();
        let scr = st.parser.screen();
        let (rows, cols) = scr.size();
        let mut lines = Vec::with_capacity(rows as usize);
        for r in 0..rows {
            let mut runs: Vec<TermRun> = Vec::new();
            for c in 0..cols {
                let Some(cell) = scr.cell(r, c) else { continue };
                if cell.is_wide_continuation() {
                    continue;
                }
                let (mut fg, mut bg) = (resolve(cell.fgcolor(), dfg), resolve(cell.bgcolor(), dbg));
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let bg = if bg == dbg { Color::default() } else { bg };
                let text = if cell.has_contents() { cell.contents() } else { " ".to_string() };
                let w = if cell.is_wide() { 2 } else { 1 };
                match runs.last_mut() {
                    Some(last) if last.fg == fg && last.bg == bg && last.bold == cell.bold() && last.underline == cell.underline() && !cell.is_wide() && last.col + last.len == c as i32 => {
                        last.text = format!("{}{text}", last.text).into();
                        last.len += 1;
                    }
                    _ => runs.push(TermRun { col: c as i32, len: w, text: text.into(), fg, bg, bold: cell.bold(), underline: cell.underline() }),
                }
            }
            // Spacje na domyślnym tle na końcu wiersza nic nie rysują.
            runs.retain(|r| r.bg != Color::default() || !r.text.trim().is_empty() || r.underline);
            lines.push(TermLine { runs: ModelRc::new(VecModel::from(runs)) });
        }
        let (cr, cc) = scr.cursor_position();
        let sel = st.sel.map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        let g = ui.global::<TermState>();
        g.set_lines(ModelRc::new(VecModel::from(lines)));
        g.set_cursor_row(cr as i32);
        g.set_cursor_col(cc as i32);
        g.set_cursor_on(!scr.hide_cursor() && scr.scrollback() == 0 && st.id.is_some());
        g.set_scrolled(scr.scrollback() > 0);
        match sel {
            Some(((r0, c0), (r1, c1))) => {
                g.set_sel_on(true);
                g.set_sel_r0(r0 as i32);
                g.set_sel_c0(c0 as i32);
                g.set_sel_r1(r1 as i32);
                g.set_sel_c1(c1 as i32);
            }
            None => g.set_sel_on(false),
        }
    });
}

fn write(s: &Shared, data: &str) {
    let id = ST.with(|st| {
        let mut st = st.borrow_mut();
        // Pisanie wraca na dół historii i kasuje zaznaczenie.
        let back = st.parser.screen().scrollback() > 0 || st.sel.is_some();
        st.parser.set_scrollback(0);
        st.sel = None;
        (st.id, back)
    });
    if id.1 {
        render(s);
    }
    if let Some(id) = id.0 {
        let _ = terminal::write(id, data);
    }
}

fn selection_text() -> Option<String> {
    ST.with(|st| {
        let st = st.borrow();
        let (a, b) = st.sel?;
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let t = st.parser.screen().contents_between(a.0, a.1, b.0, b.1 + 1);
        (!t.is_empty()).then_some(t)
    })
}

fn copy(s: &Shared) -> bool {
    let Some(t) = selection_text() else { return false };
    let _ = clip::set_text(&t);
    ST.with(|st| st.borrow_mut().sel = None);
    render(s);
    true
}

fn paste(s: &Shared) {
    let text = clip::get_text().replace("\r\n", "\r").replace('\n', "\r");
    if text.is_empty() {
        return;
    }
    let bracketed = ST.with(|st| st.borrow().parser.screen().bracketed_paste());
    if bracketed {
        write(s, &format!("\x1b[200~{text}\x1b[201~"));
    } else {
        write(s, &text);
    }
}

/// Klawisz w terminalu; `false` – niech obsłuży go reszta okna.
fn key(s: &Shared, k: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
    if ctrl && k == "`" {
        toggle(s);
        return true;
    }
    let lower = k.to_lowercase();
    if ctrl && (lower == "c" && (shift || selection_text().is_some())) {
        copy(s);
        return true;
    }
    if ctrl && lower == "v" {
        paste(s);
        return true;
    }
    if ctrl && k == "\t" {
        return false;
    }
    let (running, app_cursor) = ST.with(|st| {
        let st = st.borrow();
        (st.id.is_some(), st.parser.screen().application_cursor())
    });
    if !running {
        if k == "\n" || k == "\r" {
            start(s);
        }
        return true;
    }
    let arrow = |c: char| if app_cursor { format!("\x1bO{c}") } else { format!("\x1b[{c}") };
    let seq: String = match k {
        "\n" | "\r" => "\r".into(),
        "\u{8}" => "\x7f".into(),
        "\t" => if shift { "\x1b[Z".into() } else { "\t".into() },
        "\u{1b}" => "\x1b".into(),
        "\u{7f}" => "\x1b[3~".into(),
        "\u{F700}" => arrow('A'),
        "\u{F701}" => arrow('B'),
        "\u{F703}" => arrow('C'),
        "\u{F702}" => arrow('D'),
        "\u{F729}" => "\x1b[H".into(),
        "\u{F72B}" => "\x1b[F".into(),
        "\u{F727}" => "\x1b[2~".into(),
        "\u{F72C}" => {
            if shift {
                scroll(s, rows_now() as i32);
                return true;
            }
            "\x1b[5~".into()
        }
        "\u{F72D}" => {
            if shift {
                scroll(s, -(rows_now() as i32));
                return true;
            }
            "\x1b[6~".into()
        }
        _ => {
            let mut ch = k.chars();
            let c = match (ch.next(), ch.next()) {
                (Some(c), None) => c,
                // Kilka znaków naraz (np. z IME) idzie do powłoki bez zmian.
                (Some(_), Some(_)) => {
                    write(s, k);
                    return true;
                }
                _ => return false,
            };
            if ('\u{F704}'..='\u{F8FF}').contains(&c) {
                // F1–F12 i inne klawisze specjalne – zostają dla okna.
                return false;
            }
            if ctrl && c.is_ascii_alphabetic() {
                ((c.to_ascii_lowercase() as u8 - b'a' + 1) as char).to_string()
            } else if alt {
                format!("\x1b{c}")
            } else {
                c.to_string()
            }
        }
    };
    write(s, &seq);
    true
}

fn rows_now() -> u16 {
    ST.with(|st| st.borrow().rows)
}

/// Przewija historię: dodatnie – w górę.
fn scroll(s: &Shared, n: i32) {
    ST.with(|st| {
        let mut st = st.borrow_mut();
        let cur = st.parser.screen().scrollback() as i32;
        st.parser.set_scrollback((cur + n).max(0) as usize);
    });
    render(s);
}

fn resized(s: &Shared, cols: i32, rows: i32) {
    let (cols, rows) = (cols.clamp(10, 1000) as u16, rows.clamp(2, 500) as u16);
    let (id, changed, fresh) = ST.with(|st| {
        let mut st = st.borrow_mut();
        let changed = (st.cols, st.rows) != (cols, rows);
        let fresh = st.cols == 0;
        st.cols = cols;
        st.rows = rows;
        if changed {
            st.parser.set_size(rows, cols);
        }
        (st.id, changed, fresh)
    });
    if fresh {
        start(s);
        return;
    }
    if changed {
        if let Some(id) = id {
            terminal::resize(id, cols, rows);
        }
        render(s);
    }
}

/// Terminal idzie za folderem otwartym w aplikacji (gdy włączone „Za folderem”).
pub fn follow(s: &Shared) {
    let on = s.borrow().ui.upgrade().is_some_and(|u| {
        let g = u.global::<TermState>();
        g.get_follow()
    });
    if !on {
        return;
    }
    let path = active_path(s);
    let id = ST.with(|st| {
        let st = st.borrow();
        st.id.filter(|_| !st.cwd.eq_ignore_ascii_case(&path))
    });
    let Some(id) = id else { return };
    if !local(&path) {
        return;
    }
    if terminal::cd(id, &path).is_ok() {
        ST.with(|st| st.borrow_mut().cwd = path.clone());
        with_ui(s, |g| g.set_cwd(path.into()));
    }
}

pub fn bind(ui: &crate::MainWindow, st: &Shared) {
    let g = ui.global::<TermState>();
    let s = st.clone();
    g.on_toggle(move || toggle(&s));
    let s = st.clone();
    g.on_key(move |k, ctrl, shift, alt| key(&s, k.as_str(), ctrl, shift, alt));
    let s = st.clone();
    g.on_resized(move |c, r| resized(&s, c, r));
    let s = st.clone();
    g.on_restart(move || start(&s));
    let s = st.clone();
    g.on_scroll(move |n| scroll(&s, n));
    let s = st.clone();
    g.on_toggle_follow(move || {
        with_ui(&s, |g| g.set_follow(!g.get_follow()));
        follow(&s);
    });
    let s = st.clone();
    g.on_sel_clear(move || {
        ST.with(|st| st.borrow_mut().sel = None);
        render(&s);
    });
    let s = st.clone();
    g.on_sel_move(move |r0, c0, r1, c1| {
        let max = ST.with(|st| {
            let st = st.borrow();
            (st.rows.saturating_sub(1) as i32, st.cols.saturating_sub(1) as i32)
        });
        let p = |r: i32, c: i32| (r.clamp(0, max.0) as u16, c.clamp(0, max.1) as u16);
        ST.with(|st| st.borrow_mut().sel = Some((p(r0, c0), p(r1, c1))));
        render(&s);
    });
    let s = st.clone();
    // Prawy przycisk jak w konsoli Windows: z zaznaczeniem kopiuje, bez – wkleja.
    g.on_context(move || {
        if !copy(&s) {
            paste(&s);
        }
    });
    let s = st.clone();
    g.on_height_changed(move |h| {
        let mut b = s.borrow_mut();
        b.prefs.terminal_height = h;
        b.prefs.save();
    });
}
