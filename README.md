# PowerDesk

Pulpit, pasek zadań, menu Start i menedżer plików dla Windows 10/11 w jednym programie – natywnie
w Rust + Slint, bez przeglądarki w środku. Zastępuje Eksplorator Windows (także jako powłoka po
zalogowaniu) i wcześniejszą aplikację OpenFilePower.

## Najważniejsze

- **Menedżer plików:** karty i dwa panele, lista / siatka / galeria, podgląd (zdjęcia, wideo, PDF, dokumenty,
  kod, SVG, czcionki, modele 3D, Markdown), operacje w tle z cofaniem, archiwa, porównywanie folderów,
  synchronizacja, analiza miejsca, terminal.
- **Pulpit:** ikony, tapety (z Windows, pokaz slajdów, zdjęcie dnia), widżety, strefy okien (Shift przy przeciąganiu).
- **Pasek zadań:** okna z podglądami, przypięte programy, zasobnik (sieć, głośność, bateria, ikony programów),
  centrum powiadomień, kalendarz, pulpity wirtualne (opcjonalnie).
- **Menu Start:** aplikacje, wyszukiwanie plików i ustawień, kalkulator z jednostkami, kafelki w grupach
  i folderach, widżety w kafelkach.
- **Widżety:** zegar, zegary świata, kalendarz, pogoda, co teraz gra, zdjęcia, przypomnienia, zadania,
  odliczanie, system, dyski, sieć, schowek, kursy walut, notatka.
- **Nakładka (Alt+R):** FPS i czas klatki dowolnego programu (zdarzenia systemowe Windows, bez wstrzykiwania),
  procesor, karta graficzna, pamięć; tryb gry i limit klatek NVIDIA.
- **Bezpieczeństwo:** strażnik przywraca pulpit i pasek Windows po awarii; Shift przy logowaniu – sesja
  z Eksploratorem; dziennik i „Zgłoś problem”.

## Instalacja

Pobierz `PowerDesk-<wersja>.exe` z [Wydań](../../releases) i uruchom – przy pierwszym uruchomieniu
zaproponuje instalację dla bieżącego użytkownika (bez uprawnień administratora). Aktualizuje się sam.
Odinstalowanie: Ustawienia Windows → Aplikacje albo Ustawienia PowerDesk → System – pulpit, pasek
i Eksplorator Windows wracają na miejsce.

## Budowanie

```
cargo build --release -p powerdesk
```

Struktura: `core/` – backend bez interfejsu (pliki, integracja z Windows), `app/` – interfejs Slint
(`app/ui/*.slint`), `docs/` – plan i notatki (`PLAN-POWERDESK.md`, `NASTEPNE-KROKI.md`).

## Licencja

Zobacz [LICENSE](LICENSE).
