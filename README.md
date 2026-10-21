# OpenFilePower dla Windows

Minimalistyczny menedżer plików AMOLED dla Windows 10/11 – siostrzana aplikacja [OpenFilePower](https://github.com/BlackSingularities/OpenFilePower) na Androida. Rust (Tauri 2) + Svelte 5.

## Funkcje

**Przeglądanie**
- Karty (przeciąganie, przywracanie zamkniętych, upuszczanie plików na kartę), podział na dwa panele.
- Lista, siatka i galeria z wirtualnym przewijaniem – płynnie także przy dziesiątkach tysięcy plików.
- Foldery ze zdjęciami same otwierają się jako galeria; widok i sortowanie zapamiętywane per folder.
- Okruszki z rozwijanymi podfolderami, edycja ścieżki z podpowiedziami, paleta szybkiego przejścia (Ctrl+K).
- Filtrowanie na żywo i wyszukiwanie w podfolderach (wieloznaczniki `*.jpg`), filtry typów.

**Miniatury i podgląd – bez generowania własnych miniatur**
- Miniatury i ikony z systemowej pamięci podręcznej Windows (`IShellItemImageFactory`), w tym RAW/HEIC przez kodeki WIC.
- Miniatury PDF z systemu; jeśli Windows ich nie ma – render pierwszej strony tylko w pamięci.
- Panel podglądu: zdjęcia, wideo, audio, PDF (wbudowany czytnik), tekst i kod, DOCX/XLSX/PPTX/ODT/EPUB, archiwa ZIP.
- Pełnoekranowa przeglądarka: zoom do kursora, przesuwanie, obrót, pokaz slajdów, pasek miniatur, metadane EXIF.

**Operacje na plikach**
- Kopiowanie/przenoszenie w tle z postępem, prędkością, czasem do końca i anulowaniem; atomowe zapisy.
- Konflikty: zastąp, zastąp starsze, zachowaj oba, pomiń. Cofanie (Ctrl+Z).
- Kosz systemowy, schowek zgodny z Eksploratorem, przeciąganie, zaznaczanie ramką, zmiana nazwy w miejscu.
- Rozbudowane menu kontekstowe + pełne menu Eksploratora (Shift + prawy przycisk).
- Właściwości: wszystkie metadane systemowe, rozmiar folderu, SHA-256, systemowe okno właściwości.

**Dyski**
- Dyski lokalne, wymienne i sieciowe z zajętością; automatyczne odświeżanie po podłączeniu pendrive’a.
- Mapowanie udziałów SMB/NAS (logowanie w oknie Windows), widoczne także rozłączone, zapamiętane dyski.

**Synchronizacja**
- Kopia lustrzana, dwukierunkowa (z konfliktami) lub kopia zapasowa – z dyskiem zewnętrznym albo sieciowym.
- Dysk rozpoznawany po numerze seryjnym woluminu – działa pod dowolną literą.
- Gdy cel jest podłączony – synchronizacja na bieżąco; gdy nie – informacja, kiedy był widziany i ile zmian czeka.
- Wersjonowanie (`.ofp-versions`), ochrona przed masowym usuwaniem, podgląd planu, historia, działanie z zasobnika i autostart.

## Budowanie

```bash
npm ci
npx tauri build
```

Każdy push na `main` buduje instalator i wersję przenośną w GitHub Actions i publikuje wydanie.

---

**Projekt i wykonanie: [redzikowski.com](https://redzikowski.com)**

<a href="https://buycoffee.to/adamredzikowski" target="_blank"><img src="https://buycoffee.to/static/img/share/share-button-dark--pl.png" style="width: 351px; height: 92px" width="351" height="92" alt="Postaw kawę dla adamredzikowski na buycoffee.to"></a>
