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
| 1 | Menedżer plików – przeglądanie (pełny Toolbar, widoki, sortowanie, filtrowanie, wyszukiwanie) | **w toku** |
| 2 | Operacje na plikach, menu kontekstowe, okna dialogowe, schowek, przeciąganie | – |
| 3 | Podgląd i przeglądarka (zdjęcia, wideo, dokumenty) | – |
| 4 | Ekrany specjalne: Ten komputer, Ustawienia, Synchronizacja, Analiza miejsca, Terminal | – |
| 5 | Pulpit PowerDesk na wszystkich monitorach (tryb testowy i zastępujący) | – |
| 6 | Pasek zadań: AppBar na każdym monitorze, lista okien, zegar, Start | – |
| 7 | Zasobnik (ikony tray), menu Start, miniatury okien | – |
| 8 | Instalator i deinstalator, migracja z OpenFilePower, wydanie | – |

## Lista parytetu (z obecnej wersji)

### Okno i nawigacja
- [x] Okno bez ramki, przeciąganie, minimalizacja/maksymalizacja/zamknięcie
- [x] Karty: nowa, zamknij, przełączanie, zamknięcie środkowym przyciskiem
- [ ] Karty: przypinanie, przeciąganie, przywracanie zamkniętych (Ctrl+Shift+T), menu karty, upuszczanie plików na kartę
- [ ] Podział na dwa panele (Ctrl+\\)
- [x] Wstecz / dalej / w górę / odśwież, Alt+strzałki, Backspace, F5
- [x] Okruszki z rozwijanymi podfolderami, edycja ścieżki (Ctrl+L, Alt+D)
- [ ] Podpowiedzi przy edycji ścieżki
- [ ] Paleta szybkiego przejścia (Ctrl+K) i poleceń (Ctrl+Shift+P)
- [x] Klawiatura: strzałki po układzie, Shift/Ctrl, Home/End, Enter, Ctrl+A, Ctrl+1/2/3, Ctrl+H, Ctrl+Tab, Ctrl+W
- [ ] Boczne przyciski myszy, F6 między obszarami, wpisywanie nazwy
- [ ] Mica / Acrylic, akcent systemowy i własny, krycie okienek

### Widoki plików
- [x] Lista (podstawowe kolumny), ikony systemowe
- [x] Siatka i galeria z wirtualnym przewijaniem (układ liczony w Rust, rysowane tylko widoczne)
- [x] Miniatury z pamięci podręcznej Windows (w tym RAW/HEIC)
- [ ] Miniatury PDF renderowane, gdy Windows ich nie ma
- [x] Sortowanie (menu i nagłówki kolumn), foldery na górze, sortowanie naturalne
- [x] Kolumny konfigurowalne (menu nagłówka i przycisk), szerokości (uchwyt, dwuklik przywraca), przenoszenie przeciąganiem
- [x] Sortowanie po wymiarach, dacie wykonania i czasie trwania
- [x] Grupowanie (nazwa, daty, typ, rodzaj, rozmiar) z nagłówkami: zwijanie, zaznaczanie grupy
- [x] Widok i sortowanie zapamiętywane per folder, auto-galeria, wielkość miniatur (suwak, Ctrl+kółko)
- [x] Filtrowanie na żywo (z `*` i `?`), wyszukiwanie w podfolderach (Enter)
- [x] Filtry kategorii (Wszystko, Media, Zdjęcia, Wideo, Dokumenty, Audio, Archiwa) – tylko obecne w folderze
- [x] Składnia zapytań (typ:, ext:, rozmiar:, data:, utworzony:) w filtrze i wyszukiwaniu
- [ ] Fasety (format, kadr, data…), zapisane wyszukiwania, podpowiedzi zapytań
- [x] Zaznaczanie ramką (z tła, na liście poza nazwą, w siatce/galerii z Altem; Ctrl/Shift rozszerza; samoczynne przewijanie)
- [x] Zmiana nazwy w miejscu (F2, menu, nowy folder/plik; Enter/Esc/Tab), zaznaczenie zapamiętane per folder
- [x] Rozmiary folderów w tle (2 wątki, bez dysków sieciowych, unieważniane po zmianach), metadane mediów w kolumnach

### Panel boczny
- [x] Szybki dostęp (znane foldery), dyski z zajętością
- [ ] Ulubione, biblioteki, zapisane wyszukiwania, lokalizacje sieciowe, synchronizacja, zwijanie sekcji, zmiana szerokości

### Operacje
- [ ] Kopiowanie/przenoszenie w tle z postępem, konflikty, cofanie/ponawianie
- [ ] Kosz, schowek zgodny z Eksploratorem, przeciąganie do/z innych programów
- [ ] Menu kontekstowe własne + pełne menu Eksploratora (Shift)
- [ ] Nowy folder/plik, masowa zmiana nazwy, archiwa (ZIP/7z: przeglądanie, wypakowanie, tworzenie)
- [ ] Właściwości: metadane systemowe, rozmiar folderu, SHA-256, systemowe okno właściwości
- [ ] Półka (Shelf), toasty, okna dialogowe

### Podgląd
- [ ] Panel podglądu: zdjęcia, wideo, audio, PDF, tekst/kod z podświetlaniem, DOCX/XLSX/PPTX/ODT/EPUB, ZIP, metadane, mapa GPS
- [ ] Pełnoekranowa przeglądarka: zoom do kursora, obrót, pokaz slajdów, pasek miniatur, EXIF, dotyk, osobne okno

### Ekrany specjalne
- [ ] Ten komputer (Home): dyski, foldery, ostatnie pliki, informacje o systemie
- [ ] Ustawienia (wszystkie obecne opcje)
- [ ] Synchronizacja: profile, podgląd planu, historia, praca z zasobnika
- [ ] Analiza miejsca i porządki
- [ ] Wbudowany terminal

### Dyski i sieć
- [ ] Wysuwanie, czyszczenie dysku, mapowanie SMB, FTP/SFTP, WebDAV

### Integracja z systemem
- [ ] Zasobnik, autostart, domyślny menedżer plików, menu kontekstowe Eksploratora, aktualizacje
- [ ] Jedna instancja, otwieranie ścieżki z wiersza poleceń

### Nowe: pulpit i pasek
- [ ] Pulpit na każdym monitorze, tapety Windows, ikony z Pulpitu użytkownika i publicznego
- [ ] **Tryb testowy**: nakładka zamiast podmiany pulpitu, szybkie wyłączenie (Esc, zasobnik, `--desktop-test`)
- [ ] Tryb zastępujący: wpięcie w WorkerW, ukrycie ikon systemowych, przywracanie po wyjściu i awarii
- [ ] Reakcja na zmiany monitorów, DPI, tapety i restart Eksploratora
- [ ] Pasek zadań (AppBar) na każdym monitorze, systemowy pasek ukryty; Alt+Tab, Win+Tab, Win+D zostają systemowe
- [ ] Lista okien, grupowanie, miniatury, menu przycisków (nowe okno / przypnij / zamknij)
- [ ] Zegar, głośność, sieć, bateria; zasobnik (przechwytywanie ikon)
- [ ] Menu Start (najpierw jak w Windows)

### Instalator i deinstalator
- [ ] Instalacja per użytkownik i per maszyna, skróty, autostart
- [ ] Migracja: odinstalowanie OpenFilePower i przeniesienie ustawień
- [ ] Deinstalacja: zamknięcie aplikacji, przywrócenie pulpitu i paska Windows, usunięcie wpisów rejestru
  (autostart, menu kontekstowe, domyślny menedżer), opcjonalnie usunięcie ustawień
