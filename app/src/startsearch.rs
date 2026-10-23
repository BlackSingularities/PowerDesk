//! Wyszukiwanie wszystkiego w menu Start (poza aplikacjami): wynik kalkulatora i przeliczeń,
//! ustawienia Windows, pliki z szybkiego indeksu; bez zapytania – polecane aplikacje (najczęściej
//! uruchamiane) i ostatnio używane pliki.

use powerdesk_core::{calc, quickindex};

use crate::StartRow;

/// Co stoi w wierszu listy Startu.
#[derive(Clone, Debug)]
pub enum Row {
    Header,
    /// Numer na liście aplikacji.
    App(usize),
    /// Plik albo folder (także skrót .lnk z „Ostatnio używanych”).
    File(String),
    /// Strona Ustawień Windows (ms-settings:…).
    Setting(String),
    /// Wynik do skopiowania.
    Calc(String),
}

/// Strony Ustawień Windows z polskimi nazwami i słowami, po których się je znajdzie.
const SETTINGS: [(&str, &str, &str); 30] = [
    ("Ekran", "ms-settings:display", "rozdzielczość skalowanie monitor jasność"),
    ("Dźwięk", "ms-settings:sound", "głośniki mikrofon audio"),
    ("Powiadomienia", "ms-settings:notifications", "nie przeszkadzać"),
    ("Zasilanie i bateria", "ms-settings:powersleep", "uśpienie energia bateria"),
    ("Pamięć", "ms-settings:storagesense", "dysk miejsce czyszczenie"),
    ("Schowek", "ms-settings:clipboard", "historia"),
    ("Pulpit zdalny", "ms-settings:remotedesktop", "rdp"),
    ("Informacje o systemie", "ms-settings:about", "wersja nazwa komputera"),
    ("Bluetooth i urządzenia", "ms-settings:bluetooth", "parowanie"),
    ("Drukarki i skanery", "ms-settings:printers", "drukarka skaner"),
    ("Mysz", "ms-settings:mousetouchpad", "kursor przewijanie"),
    ("Touchpad", "ms-settings:devices-touchpad", "gładzik gesty"),
    ("Sieć i Internet", "ms-settings:network", "ethernet połączenie"),
    ("Wi-Fi", "ms-settings:network-wifi", "wifi sieć bezprzewodowa"),
    ("VPN", "ms-settings:network-vpn", "vpn"),
    ("Tryb samolotowy", "ms-settings:network-airplanemode", "samolot"),
    ("Tło", "ms-settings:personalization-background", "tapeta personalizacja"),
    ("Kolory", "ms-settings:colors", "motyw ciemny jasny akcent"),
    ("Motywy", "ms-settings:themes", "personalizacja"),
    ("Czcionki", "ms-settings:fonts", "fonty"),
    ("Aplikacje i funkcje", "ms-settings:appsfeatures", "odinstaluj programy"),
    ("Aplikacje domyślne", "ms-settings:defaultapps", "otwieraj za pomocą"),
    ("Aplikacje autostartu", "ms-settings:startupapps", "uruchamianie"),
    ("Konta", "ms-settings:yourinfo", "użytkownik konto"),
    ("Opcje logowania", "ms-settings:signinoptions", "hasło pin hello"),
    ("Data i godzina", "ms-settings:dateandtime", "zegar strefa czasowa"),
    ("Język i region", "ms-settings:regionlanguage", "klawiatura język"),
    ("Windows Update", "ms-settings:windowsupdate", "aktualizacje"),
    ("Zabezpieczenia Windows", "ms-settings:windowsdefender", "antywirus defender"),
    ("Prywatność i zabezpieczenia", "ms-settings:privacy", "uprawnienia kamera lokalizacja"),
];

fn header(name: &str) -> (StartRow, Row) {
    (StartRow { header: true, name: name.into(), ..Default::default() }, Row::Header)
}

/// Ikona pliku albo folderu z miniatur powłoki (ta sama co w liście plików).
fn file_row(name: &str, path: &str, detail: &str, px: u32) -> (StartRow, Row) {
    let img = crate::thumbs::get(path, px, true);
    (
        StartRow { header: false, name: name.into(), has_img: img.is_some(), img: img.unwrap_or_default(), hidden: false, detail: detail.into(), icon: crate::icons::icon("file").into() },
        Row::File(path.to_string()),
    )
}

/// Wiersze przed aplikacjami przy zapytaniu: wynik obliczenia.
pub fn before_apps(query: &str) -> Vec<(StartRow, Row)> {
    match calc::evaluate(query) {
        Some((text, value)) => vec![(
            StartRow { header: false, name: text.into(), detail: "Enter – kopiuj wynik".into(), icon: crate::icons::icon("hash").into(), ..Default::default() },
            Row::Calc(value),
        )],
        None => Vec::new(),
    }
}

/// Wiersze po aplikacjach przy zapytaniu: ustawienia Windows i pliki.
pub fn after_apps(query: &str, px: u32) -> Vec<(StartRow, Row)> {
    let mut out = Vec::new();
    let q = quickindex::fold(query.trim());
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return out;
    }
    let sets: Vec<_> = SETTINGS
        .iter()
        .filter(|(name, _, keys)| {
            let hay = quickindex::fold(&format!("{name} {keys}"));
            words.iter().all(|w| hay.contains(w))
        })
        .take(5)
        .collect();
    if !sets.is_empty() {
        out.push(header("Ustawienia"));
        for (name, uri, _) in sets {
            out.push((StartRow { header: false, name: (*name).into(), detail: "Ustawienia Windows".into(), icon: crate::icons::icon("settings").into(), ..Default::default() }, Row::Setting(uri.to_string())));
        }
    }
    let files = quickindex::search(query, 12);
    if !files.is_empty() {
        out.push(header("Pliki i foldery"));
        for f in files {
            let parent = std::path::Path::new(&f.path).parent().map(|p| p.display().to_string()).unwrap_or_default();
            out.push(file_row(&f.name, &f.path, &parent, px));
        }
    }
    out
}

/// Bez zapytania, nad listą liter: polecane aplikacje i ostatnio używane pliki.
pub fn top(apps: &[powerdesk_core::startmenu::App], launches: &std::collections::HashMap<String, u32>, px: u32) -> Vec<(StartRow, Row)> {
    let mut out = Vec::new();
    let mut fav: Vec<(usize, u32)> = apps.iter().enumerate().filter_map(|(i, a)| launches.get(&a.shell_path()).map(|&n| (i, n))).collect();
    fav.sort_by(|a, b| b.1.cmp(&a.1));
    if !fav.is_empty() {
        out.push(header("Polecane"));
        for (i, _) in fav.into_iter().take(5) {
            let a = &apps[i];
            let img = crate::thumbs::get(&a.shell_path(), px, true);
            out.push((StartRow { header: false, name: a.name.clone().into(), has_img: img.is_some(), img: img.unwrap_or_default(), ..Default::default() }, Row::App(i)));
        }
    }
    let recent = quickindex::recent(6);
    if !recent.is_empty() {
        out.push(header("Ostatnio używane"));
        for (name, lnk) in recent {
            out.push(file_row(&name, &lnk, "", px));
        }
    }
    out
}

/// Uruchomienie wiersza (poza aplikacją – tę uruchamia start.rs). Zwraca true, gdy zamknąć Start.
pub fn activate(row: &Row) -> bool {
    match row {
        Row::File(p) => {
            let _ = open::that_detached(p);
            true
        }
        Row::Setting(uri) => {
            let _ = open::that_detached(uri);
            true
        }
        Row::Calc(v) => {
            let _ = powerdesk_core::clip::set_text(v);
            true
        }
        _ => false,
    }
}
