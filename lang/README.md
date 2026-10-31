# Tłumaczenia PowerDesk

Tekstem źródłowym jest polski. Każdy inny język to jeden plik gettext:

```
lang/<kod>/LC_MESSAGES/powerdesk.po
```

Ten sam plik czyta Slint (teksty `@tr("…")` w `app/ui/*.slint`) i program w Ruście (`tr("…")`, `trf!("…")` – `core/src/lang.rs`). Budowa sama znajduje wszystkie foldery w `lang/`, a język pojawia się w Ustawieniach → Wygląd → Język.

## Nowy język

1. Skopiuj `lang/powerdesk.pot` do `lang/<kod>/LC_MESSAGES/powerdesk.po` (kod jak w Windows: `de`, `fr`, `uk`, `pt-BR`…).
2. W nagłówku wpisz `Language: <kod>` i `X-Language-Name: <nazwa języka w tym języku>` (np. `Deutsch`) – to nazwa na liście w Ustawieniach.
3. Uzupełnij `msgstr` (np. w Poedit). Puste `msgstr` – zostaje tekst polski.
4. Wypchnij – GitHub Actions zbuduje program z nowym językiem.

## Zasady

- Miejsca `{nazwa}` i `{}` zostaw bez zmian; kolejność można zmienić (`{}` liczą się po kolei – w tłumaczeniu można pisać `{0}`, `{1}`).
- Teksty zaczynające się lub kończące spacją, `·`, `–` albo `\n` to kawałki łączone z innymi – zachowaj te znaki.
- Nie tłumacz poleceń (PowerShell, `Set-Location`), nazw formatów schowka, czcionek ani `AUTH:` – zostaw `msgstr` puste.

## Nowe teksty w kodzie

W `.slint` pisz `@tr("Tekst")`, w Ruście `crate::tr("Tekst")` (w `core` – `crate::lang::tr`) albo `powerdesk_core::trf!("Tekst {x}")` zamiast `format!`, a potem dopisz `msgid` do `powerdesk.pot` i plików `.po`.
