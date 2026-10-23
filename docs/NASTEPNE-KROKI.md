# PowerDesk – co dalej (stan na 23.10.2026)

Plik dla następnej sesji. Pełny plan i lista parytetu: [PLAN-POWERDESK.md](PLAN-POWERDESK.md).

## Zasady pracy (od użytkownika)

- Bez `border-left` i zamienników (pionowa belka po lewej) – wyróżnienia tłem, obwódką, kropką, górną linią.
- `cargo build` tylko na prośbę albo przed commitem; **nie uruchamiać i nie testować programu bez wyraźnej prośby**
  (uruchomiony PowerDesk podmienia pulpit i pasek użytkownika). Do sprawdzania kodu: `cargo check -p powerdesk`.
- Bez sterowania ekranem (computer use), chyba że użytkownik poprosi.
- Każda kontrolka z podpowiedzią (TipArea), panele wysuwane nad treścią.
- PowerDesk zastępuje Eksplorator zawsze – bez przełączników „włącz/wyłącz”.
- Okienka (menu, głośność, sieć, kalendarz, Start, schowek…) mają wyglądać tak samo jak menu kontekstowe.

## Stan

Gałąź `powerdesk-pulpit` scalona do `main`. Ostatnie commity: podmiana pulpitu i paska na stałe, zasobnik
(sieć/Wi-Fi z hasłem, głośność z mikserem i wyborem urządzenia, bateria, kalendarz), grupowanie i przypinanie,
podglądy okien, menu Start w układzie Windows 10, historia schowka (Win+V), przezroczystość paska.

Moduły: `core/src/shellswap.rs` (podmiana i przywracanie, strażnik `--shell-guard`), `core/src/startmenu.rs`
(aplikacje, zasilanie, hak klawisza Win i Win+V), `core/src/sysbar.rs` (audio, sieć, bateria), `core/src/wifi.rs`,
`core/src/cliphist.rs`, `app/src/taskbar.rs`, `app/src/start.rs`, `app/src/desktop.rs`, `app/src/cliphistory.rs`.

## Do sprawdzenia przy najbliższym uruchomieniu (wszystko niesprawdzone na ekranie)

1. Pasek nie miga; zmaksymalizowane okno kończy się równo nad paskiem (bez szpary) na każdym monitorze –
   pasek Windows jest teraz tylko ukryty, Windows sam rezerwuje 48 px. Uwaga: przy skali ekranu ≠ 100%
   wysokość paska Windows i PowerDesk może się różnić – wtedy dopasować `HEIGHT` w `taskbar.rs` do niej.
2. Pulpit pokazuje ikony (okna pulpitu w warstwie tapety `WorkerW`, Windows 11 24H2) i przyjmuje kliknięcia.
   Jeśli kliknięcia nie dochodzą – sprawdzić, czy coś nad warstwą (DefView) nadal je łapie.
3. Przezroczystość paska (Ustawienia → Pasek zadań): przy 0% widać tapetę, nie czarne tło.
4. Okienka w stylu menu (zaokrąglenie, obwódka), Start bez obwódki.
5. Win+V – historia schowka; klik wkleja w poprzednie okno; samo Win otwiera Start PowerDesk, Win+E/D/Tab systemowe.
6. Wybór urządzenia odtwarzającego (IPolicyConfig), łączenie z nową siecią Wi-Fi z hasłem.
7. Start: zmiana rozmiaru krawędziami, widok duże ikony, ukrywanie aplikacji, menu zasilania.
8. Strażnik: twarde zabicie głównego procesu przywraca pasek i ikony Windows (sprawdzone wcześniej, ale po
   zmianach w `shellswap.rs` – sprawdzić ponownie).

## Decyzja do podjęcia z użytkownikiem: tryb powłoki

Użytkownik chce, żeby w wersji produkcyjnej pasek Windows i Eksplorator były **wyłączone**, a nie ukryte.
Propozycja (czeka na „tak”):
- PowerDesk jako powłoka: `HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon\Shell` = ścieżka do
  `powerdesk.exe` (bez administratora, tylko bieżący użytkownik). Eksplorator nie startuje po zalogowaniu.
- Pulpit: zwykłe okna na dnie (bez Progman), tapeta jak dziś.
- Zasobnik: własne okno klasy `Shell_TrayWnd` – PowerDesk dostaje ikony innych programów (`Shell_NotifyIcon`),
  czego teraz nie da się zrobić.
- Własne skróty: Win+E, Win+R, Win+D, Win+L, Win+Tab (Alt+Tab działa bez Eksploratora); sprawdzić powiadomienia.
- Bezpieczeństwo: strażnik restartuje PowerDesk po awarii, po kilku awariach z rzędu uruchamia `explorer.exe`.
- Deinstalacja i `--remove-integration` przywracają `Shell` = `explorer.exe`.

## Kolejne zadania z planu

- Menu Start: foldery grup jak w Windows 10, przeciąganie i rozmiary kafelków.
- Pasek: zmiana kolejności przypiętych przeciąganiem.
- Reakcja na zmiany monitorów, DPI i tapety (teraz tylko restart Eksploratora odbudowuje pulpit).
- Etap 8: instalator i deinstalator (przywrócenie pulpitu, paska i powłoki), migracja z OpenFilePower.
