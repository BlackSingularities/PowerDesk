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

## Tryb powłoki (decyzja: tak – w kodzie, nieuruchamiany)

- `core/src/shellmode.rs`: wpis `HKCU\…\Winlogon\Shell` = `"powerdesk.exe" --shell`, odnawiany w
  `integration::ensure` przy każdym starcie (bez przełącznika), zdejmowany w `--remove-integration`
  (wtedy też start `explorer.exe`, jeśli go nie ma). `detect()` przy starcie: brak `Shell_TrayWnd` = powłoka.
- Pulpit: zwykłe okna na dnie (podklasa okna wymusza `HWND_BOTTOM`), bez Progman. Monitory: zapasowo
  `EnumDisplayMonitors` + tapeta z `HKCU\Control Panel\Desktop`, gdy IDesktopWallpaper nie działa.
- Pasek: obszar roboczy monitorów ustawiany sami (`SPI_SETWORKAREA`), zerowany przy zamknięciu.
- Zasobnik: `core/src/trayhost.rs` – własne okno `Shell_TrayWnd`, ikony programów z WM_COPYDATA
  (NIM_ADD/MODIFY/DELETE/SETVERSION), klik lewy/dwuklik/prawy odsyłany komunikatem zwrotnym; na pasku
  przed ikoną sieci (`TrayApp` w `taskbar.slint`). Dymki (NIF_INFO) – jako powiadomienie PowerDesk
  (w głównym oknie). `Shell_NotifyIconGetRect` – miejsce ostatniego kliku albo zasobnika przy zegarze.
  AppBar: tylko ABM_GETSTATE (układ APPBARMSGDATA 64-bitowy, przesunięcie 56 – niesprawdzone).
- Skróty: Win+E (okno PowerDesk), Win+R (RunFileDlg), Win+D (własne minimalizowanie i przywracanie);
  Win+L i Alt+Tab działają bez Eksploratora. Win+Tab – własny widok zadań (`app/src/taskview.rs`:
  karty wszystkich okien z podglądami DWM na monitorze z kursorem, klik przełącza, środkowy/krzyżyk
  zamyka, Esc i klik w tło wracają).
- AppBary innych programów (SHAppBarMessage przez nasz `Shell_TrayWnd`): NEW/REMOVE/QUERYPOS/SETPOS/
  GETSTATE/GETTASKBARPOS; obszar roboczy = monitor − nasz pasek − AppBary (`shellmode::set_bar`,
  `appbar_*`), wynik przez SHLockShared. Dwa układy danych (80 B x64, 52 B) – niesprawdzone na żywo.
- Pierwsze okno pulpitu rejestrowane przez `SetShellWindow` (user32, nieudokumentowane).
- Menu paska i Startu jako powłoka: „Uruchom ponownie PowerDesk” (strażnik restartuje; liczy się do
  limitu) i „Przełącz na Eksplorator Windows” (wpis zdjęty, strażnik uruchamia `explorer.exe`).
- Strażnik: po końcu procesu w trybie powłoki – przy wylogowaniu nic; restart PowerDesk; 3 końce w 2 min
  (także „Zakończ PowerDesk”) albo zdjęty wpis – wpis usunięty i start `explorer.exe`.

Do sprawdzenia przy pierwszym logowaniu z powłoką: czy Winlogon honoruje wpis HKCU (Windows 11 24H2),
ikony zasobnika (struktura NOTIFYICONDATA 32-bitowa), menu ikon programów po prawym kliku, obszar
roboczy przy kilku monitorach, czy pulpit nie wychodzi nad okna po kliknięciu. Awaryjnie: Ctrl+Alt+Del →
Menedżer zadań → Uruchom `powerdesk.exe --remove-integration` albo `explorer.exe`.

## Zmiany ekranu (w kodzie, nieuruchamiane)

`desktop::screen_signature()` (monitory, DPI, tapeta z rejestru i data pliku TranscodedWallpaper) co 2 s
w pilnowaniu pulpitu; zmiana – pulpit i pasek budowane od nowa.

## Aktualizacje (w kodzie, nieuruchamiane)

- Repozytorium jest **prywatne** (od 23.10.2026). Wydania czyta `core/src/update.rs` z kluczem:
  `%LOCALAPPDATA%\PowerDesk\update-key.txt` > zmienna `POWERDESK_UPDATE_TOKEN` > wbudowany przy kompilacji.
- CI buduje `PowerDesk-<wersja>.exe` z sekretem `POWERDESK_UPDATE_TOKEN` i `POWERDESK_VERSION`; plik trafia
  do wydania i `SHA256SUMS.txt`. **Do zrobienia przez użytkownika:** fine-grained token (tylko to repo,
  Contents: Read-only) i `gh secret set POWERDESK_UPDATE_TOKEN`. Token wbudowany w .exe da się wyciągnąć –
  dlatego tylko odczyt i tylko to repozytorium.
- `app/src/updates.rs`: sprawdzanie minutę po starcie i co 6 h, dymek w zasobniku + powiadomienie
  „Zaktualizuj”; ręcznie: prawy przycisk na Start → „Sprawdź aktualizacje”. Bez zgodnej sumy SHA-256
  nie podmieniamy. Skrypt `apply.cmd` czeka na koniec procesu, zostawia `powerdesk.exe.old`, podmienia
  i uruchamia; w trybie powłoki uruchamia strażnik (znacznik `updating.txt`, bez liczenia awarii).

## Też w kodzie, nieklikane (23.10.2026)

- Pasek: przypięte przeciągane w nowe miejsce (przygasają, puszczenie wybiera pozycję; `taskbar::reorder`).
- Zasobnik powłoki: ponad 6 ikon – strzałka rozwija resztę (`tray-max`, `tray-open`).
- Panel boczny, prawy przycisk na dysku: Wysuń (czasownik „eject”), Odłącz dysk sieciowy, Oczyszczanie dysku
  (`cleanmgr /dX`), Mapuj dysk sieciowy… (`WNetConnectionDialog`).

- Start: kafelki w grupach (`prefs.start_groups`, stary `start_pins` przenoszony), siatka 6 pól po 52 px
  (`app/src/starttiles.rs`, testy), nazwa grupy wpisywana w nagłówku, folder – upuszczenie na środek
  kafelka (klik otwiera, „Rozgrupuj folder”), upuszczenie obok – miejsce przed kafelkiem, pod grupami – nowa
  grupa; „Zmień rozmiar” w menu kafelka.

- Ustawienia (nieklikane): Przeglądanie – „Jedno kliknięcie otwiera” (`SettingsState.single-click` czyta
  fileview.slint), włączanie sekcji panelu bocznego (`prefs.side_hidden`); Wygląd – „Krycie tła okna” przy
  Mica/Acrylic (`Theme.window-alpha`, 0 = domyślne); System – „Krzyżyk chowa okno do zasobnika”
  (`close_to_tray`, domyślnie tak) i „Okno folderów przy starcie z Windows” (`autostart_window`, też przy
  `--shell`). Autostart sam w sobie zostaje zawsze włączony (integracja bez przełączników).

## Widżety (nowe, w kodzie, nieklikane)

- `app/ui/widgets.slint` (`WidgetBody`, globalny `WidgetData`) i `app/src/widgets.rs` (dane co 1 s,
  dyski co 30 s). Rodzaje: Zegar, Kalendarz, System (procesor, pamięć, historia), Dyski, Notatka – każdy
  dopasowuje układ do rozmiaru (mały kafelek – najważniejsze, duży – więcej).
- Start: prawy przycisk na pustym miejscu kafelków → „Dodaj widżet”; widżet to kafelek (`StartPin.widget`),
  rozmiary jak kafelki, nie wchodzi do folderów. Klik: notatka – pisanie, kalendarz – Kalendarz Windows,
  system – Menedżer zadań.
- Pulpit: menu tła → „Dodaj widżet” w miejscu kliknięcia; przeciąganie (notatka – za górny pasek), róg
  – rozmiar, siatka 8 px, prawy przycisk – „Usuń widżet”; zapis w `prefs.desktop_widgets` (monitor, x, y, w, h).
- Druga partia (`app/src/widgetfeeds.rs`, ustawienia wspólne dla rodzaju, zmiana z menu widżetu):
  Pogoda (Open-Meteo bez klucza, miejscowość – domyślnie Warszawa, co 30 min, prognoza 4 dni, ikony
  rysowane kształtami), Zegary świata (`chrono-tz`, polskie nazwy miast albo strefy IANA), Co teraz gra
  (`core/src/nowplaying.rs` – sterowanie multimediami Windows, przyciski), Zdjęcia (folder – domyślnie
  Obrazy, co 15 s, zmniejszane w tle), Przypomnienia („14:30 tekst”, „24.12 18:00 tekst”, „za 15 min
  tekst”; dymek i powiadomienie, potem znika). Na pulpicie notatkę i multimedia chwyta się za górny pasek.
- Przeglądarka: szczypanie dwoma palcami i gestem gładzika (`pinch` w main.rs → `ViewerState.pinch-*`).

## Kolejne zadania z planu

- Etap 8: instalator i deinstalator (przywrócenie pulpitu, paska i powłoki), migracja z OpenFilePower.
