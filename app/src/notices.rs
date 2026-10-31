//! Centrum powiadomień: dymki programów z zasobnika (w trybie powłoki nie ma ich kto pokazać),
//! przypomnienia, aktualizacje i błędy PowerDesk. Każde wyskakuje na chwilę nad paskiem
//! i zostaje w historii (okienko pod ikoną przy zegarze, licznik nieprzeczytanych).

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::time::Duration;

use chrono::Local;
use slint::ComponentHandle;

use crate::{NoticeRow, NoticeToastWindow};

#[derive(Clone)]
struct Notice {
    time: chrono::DateTime<Local>,
    source: String,
    title: String,
    text: String,
}

const MAX: usize = 100;

thread_local! {
    static LIST: RefCell<VecDeque<Notice>> = const { RefCell::new(VecDeque::new()) };
    static UNREAD: Cell<i32> = const { Cell::new(0) };
    static TOAST: RefCell<Option<NoticeToastWindow>> = const { RefCell::new(None) };
    static TOAST_TIMER: slint::Timer = slint::Timer::default();
}

pub fn unread() -> i32 {
    UNREAD.with(|u| u.get())
}

/// Nowe powiadomienie: do historii, licznik na pasku i dymek nad paskiem na 6 sekund.
pub fn add(source: &str, title: &str, text: &str) {
    let n = Notice { time: Local::now(), source: source.to_string(), title: title.to_string(), text: text.to_string() };
    LIST.with(|l| {
        let mut l = l.borrow_mut();
        l.push_front(n.clone());
        l.truncate(MAX);
    });
    UNREAD.with(|u| u.set(u.get() + 1));
    crate::taskbar::notices_changed();
    popup(&n);
}

/// Powiadomienie, które Windows już pokazał (toast innego programu): tylko historia i licznik, bez dymka.
pub fn add_quiet(source: &str, title: &str, text: &str) {
    let n = Notice { time: Local::now(), source: source.to_string(), title: title.to_string(), text: text.to_string() };
    LIST.with(|l| {
        let mut l = l.borrow_mut();
        l.push_front(n);
        l.truncate(MAX);
    });
    UNREAD.with(|u| u.set(u.get() + 1));
    crate::taskbar::notices_changed();
}

/// Wiersze do okienka historii (najnowsze pierwsze).
pub fn rows() -> Vec<NoticeRow> {
    let today = Local::now().date_naive();
    LIST.with(|l| {
        l.borrow()
            .iter()
            .map(|n| NoticeRow {
                source: n.source.clone().into(),
                title: n.title.clone().into(),
                text: n.text.clone().into(),
                time: if n.time.date_naive() == today { n.time.format("%H:%M").to_string() } else { n.time.format("%d.%m %H:%M").to_string() }.into(),
            })
            .collect()
    })
}

/// Okienko historii otwarte – wszystko przeczytane.
pub fn mark_read() {
    UNREAD.with(|u| u.set(0));
    crate::taskbar::notices_changed();
}

pub fn clear() {
    LIST.with(|l| l.borrow_mut().clear());
    mark_read();
}

/// Dymek nad paskiem, w prawym dolnym rogu monitora z kursorem; klik zamyka.
fn popup(n: &Notice) {
    close_popup();
    let Ok(w) = NoticeToastWindow::new() else { return };
    w.set_source(n.source.clone().into());
    w.set_title_text(n.title.clone().into());
    w.set_text(n.text.clone().into());
    w.on_dismiss(|| slint::Timer::single_shot(Duration::ZERO, close_popup));
    w.on_open_list(|| {
        slint::Timer::single_shot(Duration::ZERO, close_popup);
        crate::taskbar::open_notices_here();
    });
    crate::taskbar::place_toast(w.window(), 340.0, 96.0);
    let _ = w.show();
    let hwnd = crate::window_hwnd(w.window());
    powerdesk_core::desktop::hide_from_taskbar(hwnd);
    crate::taskbar::popup_look(hwnd);
    TOAST.with(|t| *t.borrow_mut() = Some(w));
    TOAST_TIMER.with(|t| t.start(slint::TimerMode::SingleShot, Duration::from_secs(6), close_popup));
}

fn close_popup() {
    if let Some(w) = TOAST.with(|t| t.borrow_mut().take()) {
        let _ = w.hide();
    }
}
