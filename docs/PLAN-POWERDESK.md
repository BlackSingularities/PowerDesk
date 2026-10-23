# PowerDesk – plan przepisania na natywny interfejs (Rust + Slint)

Cel: jedna aplikacja **PowerDesk** (dawniej OpenFilePower) bez WebView, wyglądająca 1:1 jak obecna wersja,
z menedżerem plików, własnym pulpitem i własnym paskiem zadań na wszystkich monitorach.

## Architektura

```
Cargo.toml          workspace
core/               powerdesk-core – backend bez UI (przeniesiony bez zmian logiki z src-tauri)
  src/events.rs       kanał zdarzeń: moduły wołają app.emit(...), UI decyduje, dokąd trafia
app/                powerdesk – natywny interfejs Slint (docelowa aplikacja)
  ui/theme.slint      tokeny 1:1 z src/app.css (ciemny/jasny, promienie, czcionki)
  ui/icons.slint      generowane z src/lib/icons.ts (build.rs) – ta sama biblioteka ikon
  src/fmt.rs          formatowanie 1:1 z src/lib/util.ts
src-tauri/ + src/   obecna wersja Tauri/Svelte – działa i jest wydawana do czasu parytetu
```

Zasady:
- Backend (`core`) nie zna UI. Nowe funkcje systemowe (pulpit, pasek, okna, zasobnik) też trafiają do `core`.
- Wygląd przenosimy komponent po komponencie z plików `.svelte`, z tymi samymi wymiarami i tokenami.
- Wersja Tauri zostaje do osiągnięcia parytetu; potem `src/` i `src-tauri/` są usuwane.
- Renderer: lokalnie FemtoVG (toolchain GNU nie ma binariów Skia); docelowo Skia w buildach MSVC – najbliższy przeglądarce wygląd czcionek.

## Etapy

| Etap | Zakres | Stan |
|---|---|---|
| 0 | Fundament: workspace, `core`, kanał zdarzeń, okno Slint, motyw, ikony, pasek tytułu, panel boczny, lista plików, karty, nawigacja, ikony systemowe | gotowe |
| 1 | Menedżer plików – przeglądanie (pełny Toolbar, widoki, sortowanie, filtrowanie, wyszukiwanie) | gotowe |
| 2 | Operacje na plikach, menu kontekstowe, okna dialogowe, schowek, przeciąganie | gotowe |
| 3 | Podgląd i przeglądarka (zdjęcia, wideo, dokumenty) | gotowe (szczypanie – nieklikane) |
| 4 | Ekrany specjalne: Ten komputer, Ustawienia, Synchronizacja, Analiza miejsca, Terminal | gotowe (nowe ustawienia – nieklikane) |
| 5 | Pulpit PowerDesk na wszystkich monitorach (zastępuje pulpit Windows) | **w toku** |
| 6 | Pasek zadań: AppBar na każdym monitorze, lista okien, zegar, Start | **w toku** |
| 7 | Zasobnik (ikony tray), menu Start, miniatury okien | – |
| 8 | Instalator i deinstalator, migracja z OpenFilePower, wydanie | w kodzie, nieuruchamiane |

## Lista parytetu (z obecnej wersji)

### Okno i nawigacja
- [x] Okno bez ramki, przeciąganie, minimalizacja/maksymalizacja/zamknięcie
- [x] Karty: nowa, zamknij, przełączanie, zamknięcie środkowym przyciskiem
- [x] Karty: przypinanie (wracają po uruchomieniu), przeciąganie, przywracanie zamkniętych (Ctrl+Shift+T), menu karty, upuszczanie plików na kartę
- [x] Podział na dwa panele (Ctrl+\\): dwie karty obok siebie, uchwyt szerokości, F6 przenosi wskazanie; pasek adresu i polecenia są jedne – dla wskazanego panelu
- [x] Wstecz / dalej / w górę / odśwież, Alt+strzałki, Backspace, F5
- [x] Okruszki z rozwijanymi podfolderami, edycja ścieżki (Ctrl+L, Alt+D)
- [x] Podpowiedzi przy edycji ścieżki: podfoldery pasujące do wpisanego początku (do 12), strzałki, Tab uzupełnia, Enter i klik przechodzą
- [x] Paleta poleceń (Ctrl+Shift+P): czynności programu z grupami i skrótami, szukanie bez polskich znaków
- [x] Paleta szybkiego przejścia (Ctrl+K): podfoldery, szybki dostęp, dyski, historia, wpisana ścieżka
- [x] Klawiatura: strzałki po układzie, Shift/Ctrl, Home/End, Enter, Ctrl+A, Ctrl+1/2/3, Ctrl+H, Ctrl+Tab, Ctrl+W
- [x] Boczne przyciski myszy (wstecz/dalej w całym oknie), F6/Shift+F6 między panelami i paskiem adresu, wpisywanie nazwy (skok do pasującego elementu)
- [x] F6 także przez panel boczny (strzałki, Enter) i podgląd (strony PDF, Enter)
- [x] Mica / Acrylic (DWM, półprzezroczyste tło i panele), akcent systemowy i własny (#rrggbb), krycie okienek 60–100% (bez rozmycia – Slint go nie ma)

### Widoki plików
- [x] Lista (podstawowe kolumny), ikony systemowe
- [x] Siatka i galeria z wirtualnym przewijaniem (układ liczony w Rust, rysowane tylko widoczne)
- [x] Miniatury z pamięci podręcznej Windows (w tym RAW/HEIC)
- [x] Miniatury PDF renderowane (pierwsza strona, Windows.Data.Pdf), gdy Windows ich nie ma
- [x] Sortowanie (menu i nagłówki kolumn), foldery na górze, sortowanie naturalne
- [x] Kolumny konfigurowalne (menu nagłówka i przycisk), szerokości (uchwyt, dwuklik przywraca), przenoszenie przeciąganiem
- [x] Sortowanie po wymiarach, dacie wykonania i czasie trwania
- [x] Grupowanie (nazwa, daty, typ, rodzaj, rozmiar) z nagłówkami: zwijanie, zaznaczanie grupy
- [x] Widok i sortowanie zapamiętywane per folder, auto-galeria, wielkość miniatur (suwak, Ctrl+kółko)
- [x] Filtrowanie na żywo (z `*` i `?`), wyszukiwanie w podfolderach (Enter)
- [x] Filtry kategorii (Wszystko, Media, Zdjęcia, Wideo, Dokumenty, Audio, Archiwa) – tylko obecne w folderze
- [x] Składnia zapytań (typ:, ext:, rozmiar:, data:, utworzony:) w filtrze i wyszukiwaniu
- [x] Fasety (format, kadr, rozdzielczość, długość, jakość, rodzaj, rozmiar, data), podpowiedzi zapytań, zapisane wyszukiwania w panelu bocznym
- [x] Zmiana nazwy zapisanego wyszukiwania
- [x] Zaznaczanie ramką (z tła, na liście poza nazwą, w siatce/galerii z Altem; Ctrl/Shift rozszerza; samoczynne przewijanie)
- [x] Zmiana nazwy w miejscu (F2, menu, nowy folder/plik; Enter/Esc/Tab), zaznaczenie zapamiętane per folder
- [x] Rozmiary folderów w tle (2 wątki, bez dysków sieciowych, unieważniane po zmianach), metadane mediów w kolumnach

### Panel boczny
- [x] Szybki dostęp (znane foldery), dyski z zajętością
- [x] Przypięte foldery (menu folderu i tła, menu panelu), biblioteki (otwiera pierwszy folder), dyski sieciowe w sekcji Sieć, synchronizacje ze stanem, zwijanie sekcji, zmiana szerokości przeciąganiem
- [x] Panel boczny: widok całej biblioteki naraz, dodawanie lokalizacji sieciowych, przeciąganie na „Przypięte”

### Operacje
- [x] Kopiowanie/przenoszenie w tle: pasek transferów w oknie (postęp, prędkość, czas, wstrzymanie, anulowanie, błędy) zamiast okienek Windows; konflikty w oknie aplikacji; cofanie/ponawianie (Ctrl+Z/Ctrl+Y, „Cofnij” w powiadomieniu)
- [x] Kosz, schowek zgodny z Eksploratorem
- [x] Przeciąganie (OLE jak w Eksploratorze): na foldery, tło listy, panel boczny, okruszki i karty; do i z innych programów; Ctrl kopiuj, Shift przenieś
- [x] Menu kontekstowe własne + pełne menu Eksploratora (Shift)
- [x] Nowy folder/plik: menu „Nowy” (folder z datą, z zaznaczonych, dokumenty, pliki programisty), cofanie, przeniesienie zaznaczonych do nowego folderu
- [x] Masowa zmiana nazwy (wzorzec, numeracja, znajdź i zamień, podgląd, cofanie)
- [x] Archiwa ZIP/7z/RAR: przeglądanie jak folderu, otwieranie plików ze środka, wypakowanie (tutaj, obok, przez kopiowanie i wklejanie), tworzenie ZIP
- [x] „Wypakuj do…” z wyborem folderu
- [x] Przeciąganie z archiwum do innych programów (wypakowanie w tle do TEMP, start, gdy przycisk nadal wciśnięty)
- [x] Właściwości: metadane systemowe w sekcjach, rozmiar folderu, SHA-256, systemowe okno właściwości
- [x] Właściwości: mapa GPS (kafelki OpenStreetMap) i graficzne skale ekspozycji
- [x] Powiadomienia (toasty z akcją), okna potwierdzenia, trwałe usuwanie (Shift+Del)
- [x] Półka (Shelf): dodawanie (Ctrl+Shift+S, menu, przeciąganie), kopiowanie i przenoszenie do otwartego folderu, przeciąganie z półki, zapamiętywanie

### Podgląd
- [x] Panel podglądu: obraz z powłoki (zdjęcia, RAW, klatka wideo, strona PDF, dokumenty), tekst i kod, spis archiwum, metadane, folder i dysk, zmiana szerokości, Alt+P
- [x] Panel podglądu: tekst z DOCX/XLSX/PPTX/ODT/EPUB (arkusz wiersz po wierszu), mapa GPS, podświetlanie składni
- [x] Odtwarzanie wideo i audio (Media Foundation) w panelu podglądu i w przeglądarce: pasek z czasem, przewijaniem, głośnością i wyciszeniem; spacja, M
- [x] Panel podglądu: przewijanie stron PDF (Windows.Data.Pdf; przyciski i kółko nad stroną)
- [x] Przeglądarka zdjęć w osobnej karcie tego samego okna (pasek tytułu z kartami i przyciskami okna zostaje, minimalizacja działa; pełny ekran tylko przyciskiem albo F11): zoom do kursora, przesuwanie, obrót, pokaz slajdów, pasek miniatur, EXIF
- [x] Przeglądarka: przesuwanie palcem (w bok – następne i poprzednie, w dół – zamknięcie)
- [x] Przeglądarka: szczypanie – dwa palce (zdarzenia dotyku winit, własne liczenie) i gest gładzika; nieklikane

### Ekrany specjalne
- [x] Ten komputer (Home): powitanie z wyszukiwaniem, dyski, stan komputera (procesor z historią, RAM, czas pracy), rozmiary folderów, ostatnie pliki z filtrem i menu
- [x] Ten komputer: zmiana kolejności i zwijanie sekcji, ulubione, aktywność transferów i synchronizacji, wykres zmian z 14 dni
- [x] Ustawienia (karta, Ctrl+,): motyw i akcent, start (folder domowy / Ten komputer / wybrany folder), domyślny widok, przeglądanie, podgląd, pytanie przed Koszem, skróty, o aplikacji
- [x] Ustawienia: jedno kliknięcie, sekcje panelu bocznego, krycie tła okna (Mica/Acrylic), zamykanie do zasobnika, okno przy starcie z Windows – w kodzie, nieklikane; aktualizacje – w menu Start; integracja – sekcja System
- [x] Synchronizacja (karta, paleta): profile ze stanem i postępem, edytor, podgląd planu, potwierdzanie masowych usunięć, historia; silnik startuje po wejściu na ekran i czeka, gdy działa stary OpenFilePower
- [x] Synchronizacja: praca z zasobnika i od startu systemu, wejście z panelu bocznego
- [x] Zajętość i porządki (karta `ofp:space|folder`, menu tła, paleta, Ten komputer): mapa kafelkowa z wchodzeniem w głąb, lista największych, menu kafelka z Koszem; porządki – puste foldery, śmieci, TEMP, zaznaczanie grup
- [x] Wbudowany terminal (Ctrl+`, pasek narzędzi, paleta): PowerShell przez ConPTY, ekran `vt100` (kolory, kursor, historia kółkiem i Shift+PgUp/PgDn), zaznaczanie i kopiowanie (Ctrl+C przy zaznaczeniu, Ctrl+Shift+C, prawy przycisk), wklejanie, „Za folderem”, wysokość i widoczność zapamiętane

### Dyski i sieć
- [ ] Wysuwanie, czyszczenie dysku, mapowanie SMB, FTP/SFTP, WebDAV (w kodzie, nieklikane: menu dysku w panelu bocznym – Wysuń, Odłącz dysk sieciowy, Oczyszczanie dysku, Mapuj dysk sieciowy… (okno Windows); brak: FTP/SFTP, WebDAV)

### Integracja z systemem
- [x] Zasobnik (ikona, menu, zamykanie do zasobnika, powrót po restarcie Eksploratora), autostart (`--tray`), Win+E
- [x] Zastąpienie Eksploratora zawsze włączone (bez przełączników): foldery i dyski, Win+E, „Otwórz w PowerDesk”, autostart – odnawiane przy starcie, stan i „Napraw” w Ustawieniach → System; cofnięcie `--remove-integration`
- [x] Ustawienia jako ustawienia powłoki: Wygląd, Nowe okno, Widok folderów, Przeglądanie, Podgląd, Pulpit, Pasek zadań, System, Skróty, O aplikacji
- [ ] Aktualizacje (w kodzie, nieuruchamiane: prywatne wydania GitHub z kluczem, dymek i „Zaktualizuj”, suma SHA-256, podmiana .exe; czeka na sekret POWERDESK_UPDATE_TOKEN)
- [x] Jedna instancja (potok nazwany), otwieranie ścieżki z wiersza poleceń w nowej karcie, okno na wierzch

### Nowe: pulpit i pasek
- [x] Pulpit na każdym monitorze, tapety Windows, ikony z Pulpitu użytkownika i publicznego: menu w stylu PowerDesk, zaznaczanie z Ctrl, Shift i ramką, przeciąganie ikon z zapamiętaniem miejsc, „Uporządkuj ikony”, zmiana nazwy w miejscu, nowy folder i dokument, kopiuj/wytnij/wklej, Kosz
- [x] Tryb testowy usunięty – pulpit i pasek PowerDesk zawsze zastępują Windows
- [x] Tryb zastępujący: pulpit jako dziecko `Progman` (Win+D go nie chowa), ikony Windows i `WorkerW` ukryte; pasek Windows ukryty i na autoukrywaniu, paski PowerDesk jako AppBar, pod oknem pełnoekranowym; przywracanie przy wyjściu, przez strażnika po awarii (`--shell-guard`, sprawdzone) i w `--remove-integration`
- [ ] Tryb powłoki (Winlogon\Shell, bez Eksploratora): w kodzie, nieuruchamiany – pulpit na dnie, obszar roboczy, własny `Shell_TrayWnd` z ikonami programów, Win+E/R/D, Win+Tab (własny widok zadań), AppBary innych programów, SetShellWindow, strażnik z powrotem do Eksploratora (szczegóły: NASTEPNE-KROKI.md)
- [ ] Reakcja na zmiany monitorów, DPI, tapety i restart Eksploratora (w kodzie, nieuruchamiane: podpis ekranu co 2 s, przebudowa pulpitu i paska)
- [x] Pasek zadań (AppBar) na każdym monitorze, systemowy pasek ukryty; Alt+Tab, Win+Tab i Win+D zostają systemowe; „Pokaż pulpit” na końcu, menu pustego miejsca (Menedżer zadań, ustawienia, zasilanie), krycie 0–100% i pełna przezroczystość na pulpicie (osobno na monitorze), menu Start pod prawym przyciskiem (ustawienia, Menedżer zadań, „Zakończ PowerDesk”)
- [ ] Lista okien, grupowanie, miniatury, menu przycisków (nowe okno / przypnij / zamknij) (w kodzie: podglądy okien DWM po najechaniu – karta z tytułem i zamknięciem, klik przełącza, środkowy zamyka; grupowanie po programie z licznikiem, klik przełącza okna grupy, menu z listą okien, „Zminimalizuj wszystkie”, „Zamknij wszystkie okna”, przypinanie programów – przyciski z samą ikoną na początku paska, uruchamianie, zapamiętane w ustawieniach; nieklikane; przeciąganie przypiętych – zmiana kolejności, nieklikane)
- [ ] Zegar, głośność, sieć, bateria; zasobnik (przechwytywanie ikon) (w kodzie, nieuruchamiane: ikony sieci, głośności i baterii przy zegarze, okienko głośności z suwakiem i wyciszeniem, kółko i środkowy przycisk na ikonie, kalendarz po kliknięciu w zegar, mikser – głośność i wyciszanie każdej aplikacji, wybór urządzenia odtwarzającego, okienko sieci – lista Wi-Fi, łączenie i rozłączanie, hasło dla nowej sieci (profil WPA2/WPA3-Personal); sprawdzone: pasek, okienko głośności, suwak; brak: ikony aplikacji w tle – wymagają własnego `Shell_TrayWnd`, więc dopiero w trybie zastępującym)
- [ ] Menu Start (najpierw jak w Windows) (w kodzie, nieklikane: układ Windows 10 – konto, Dokumenty, Obrazy, Ustawienia, zasilanie; lista z folderu „Aplikacje” z literami, szukanie bez polskich znaków, Enter uruchamia pierwszą; kafelki przypiętych; menu aplikacji – jako administrator, przypnij do Startu i paska; samo Win otwiera Start PowerDesk, skróty z Win zostają systemowe; zasilanie i sesja – zablokuj, przełącz użytkownika, wyloguj, uśpij, hibernuj, zamknij, uruchom ponownie; ukrywanie aplikacji z „Pokaż ukryte”; widok lista albo duże ikony; zmiana rozmiaru przeciąganiem górnej i prawej krawędzi; grupy kafelków z nazwami, foldery (kafelek na środek innego), przeciąganie z podglądem, rozmiary mały/średni/szeroki/duży – w kodzie, nieklikane)

### Nowe: widżety
- [ ] Widżety w kafelkach Startu i na pulpicie (w kodzie, nieklikane): zegar, kalendarz, pogoda, zegary świata, co teraz gra, zdjęcia, przypomnienia, system, dyski, notatka

### Nowe: nakładka z wydajnością
- [ ] Nakładka Alt+R: FPS (ETW), czas klatki z wykresem, procesor, karta (obciążenie, temperatura NVIDIA), RAM, VRAM – w kodzie; karta sprawdzona, FPS czeka na uprawnienia

### Instalator i deinstalator
- [ ] Instalacja per użytkownik i per maszyna, skróty, autostart (w kodzie: `--install [--all-users]`, propozycja przy pierwszym uruchomieniu, Ustawienia → System)
- [ ] Migracja: odinstalowanie OpenFilePower i przeniesienie ustawień (w kodzie: migrate.rs, cichy deinstalator OpenFilePower po pytaniu)
- [ ] Deinstalacja (w kodzie: `--uninstall [--quiet]`, wpis w „Aplikacje i funkcje”): zamknięcie aplikacji, przywrócenie pulpitu i paska Windows, usunięcie wpisów rejestru
  (autostart, menu kontekstowe, domyślny menedżer), opcjonalnie usunięcie ustawień
