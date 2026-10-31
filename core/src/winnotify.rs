//! Powiadomienia innych programów (toasty Windows) do historii PowerDesk. Dymki nadal pokazuje
//! Windows (Eksplorator w tle) – tu tylko zapis: UserNotificationListener, odpytywany co kilka sekund.
//! Bez zgody użytkownika (Ustawienia → Prywatność → Powiadomienia) albo gdy system odmówi
//! programowi bez tożsamości pakietu – nic się nie dzieje.

use std::collections::HashSet;
use std::time::Duration;

use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};

const POLL: Duration = Duration::from_secs(3);

/// Nadawca, tytuł i treść powiadomienia.
fn describe(n: &UserNotification) -> Option<(String, String, String)> {
    let app = n.AppInfo().ok().and_then(|a| a.DisplayInfo().ok()).and_then(|d| d.DisplayName().ok()).map(|s| s.to_string()).unwrap_or_default();
    let binding = n.Notification().ok()?.Visual().ok()?.GetBinding(&KnownNotificationBindings::ToastGeneric().ok()?).ok()?;
    let texts = binding.GetTextElements().ok()?;
    let lines: Vec<String> = (0..texts.Size().unwrap_or(0)).filter_map(|i| texts.GetAt(i).ok()).filter_map(|t| t.Text().ok()).map(|s| s.to_string()).collect();
    let title = lines.first().cloned().unwrap_or_default();
    let text = lines.iter().skip(1).cloned().collect::<Vec<_>>().join("\n");
    (!title.is_empty() || !text.is_empty()).then_some((app, title, text))
}

/// Startuje nasłuch w wątku tła; `on_new(nadawca, tytuł, treść)` dla każdego nowego powiadomienia.
/// Powiadomienia sprzed startu tylko zapamiętujemy (bez zalewu historii).
/// Drugie wywołanie (przebudowa pasków po zmianie monitorów) – nic.
pub fn start(on_new: impl Fn(String, String, String) + Send + 'static) {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let Ok(listener) = UserNotificationListener::Current() else {
            crate::log::info("powiadomienia Windows: nasłuch niedostępny");
            return;
        };
        let status = listener.RequestAccessAsync().and_then(|op| op.get());
        if status.ok() != Some(UserNotificationListenerAccessStatus::Allowed) {
            crate::log::info("powiadomienia Windows: brak zgody na odczyt");
            return;
        }
        let mut seen: HashSet<u32> = HashSet::new();
        let mut first = true;
        loop {
            if let Ok(list) = listener.GetNotificationsAsync(NotificationKinds::Toast).and_then(|op| op.get()) {
                let mut now = HashSet::new();
                for i in 0..list.Size().unwrap_or(0) {
                    let Ok(n) = list.GetAt(i) else { continue };
                    let Ok(id) = n.Id() else { continue };
                    now.insert(id);
                    if !first && !seen.contains(&id) {
                        if let Some((app, title, text)) = describe(&n) {
                            on_new(app, title, text);
                        }
                    }
                }
                // Usunięte z centrum akcji – zapominamy (identyfikatory mogą wrócić).
                seen = now;
                first = false;
            }
            std::thread::sleep(POLL);
        }
    });
}
