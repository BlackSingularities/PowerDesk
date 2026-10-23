//! Podświetlanie składni w panelu podglądu: prosty lekser, bez zewnętrznych bibliotek.
//! Nie rozumie języków do końca – rozpoznaje komentarze, napisy, liczby, słowa kluczowe,
//! nazwy typów i znaczniki, czyli tyle, ile widać na pierwszy rzut oka w podglądzie.
//! Wynik to wiersze z kawałkami tekstu i numerem koloru (`Tok`), rysowane w preview.slint.

/// Rodzaj kawałka tekstu; liczby muszą się zgadzać z kolorami `syn-*` w theme.slint.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Plain = 0,
    Comment = 1,
    Str = 2,
    Num = 3,
    Key = 4,
    Type = 5,
    Punct = 6,
    Tag = 7,
    Attr = 8,
}

/// Jeden kawałek wiersza: (rodzaj, tekst).
pub type Piece = (Tok, String);

const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop",
    "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "macro_rules",
];
const CLIKE: &[&str] = &[
    "abstract", "and", "as", "async", "await", "base", "bool", "break", "byte", "case", "catch", "char", "class", "const", "constructor", "continue",
    "debugger", "default", "defer", "delete", "do", "double", "else", "enum", "export", "extends", "extern", "false", "final", "finally", "float", "fn",
    "for", "from", "func", "function", "get", "go", "goto", "if", "implements", "import", "in", "instanceof", "int", "interface", "internal", "is", "let",
    "lock", "long", "namespace", "new", "null", "nullptr", "of", "operator", "or", "override", "package", "private", "protected", "public", "readonly",
    "record", "ref", "return", "sealed", "set", "short", "sizeof", "static", "struct", "super", "switch", "template", "this", "throw", "throws", "true",
    "try", "typedef", "typeof", "union", "unsigned", "using", "val", "var", "virtual", "void", "volatile", "when", "where", "while", "with", "yield",
];
const PY: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "False", "finally", "for", "from",
    "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "self", "True", "try", "while", "with",
    "yield",
];
const SH: &[&str] = &[
    "case", "do", "done", "elif", "else", "esac", "exit", "export", "fi", "for", "function", "if", "in", "local", "param", "return", "set", "then", "until",
    "while", "echo", "cd", "rem", "goto", "call", "foreach", "begin", "end", "try", "catch", "finally",
];
const SQL: &[&str] = &[
    "alter", "and", "as", "asc", "begin", "between", "by", "case", "cast", "commit", "create", "delete", "desc", "distinct", "drop", "else", "end",
    "exists", "from", "group", "having", "in", "index", "inner", "insert", "into", "is", "join", "left", "like", "limit", "not", "null", "on", "or",
    "order", "outer", "primary", "references", "right", "rollback", "select", "set", "table", "then", "union", "unique", "update", "values", "view",
    "when", "where", "with",
];
const CSS: &[&str] = &["and", "font-face", "import", "keyframes", "media", "not", "only", "supports", "from", "to", "important"];

/// Jak podświetlać dany typ pliku.
struct Lang {
    line: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    words: &'static [&'static str],
    /// Znaczniki jak w HTML/XML.
    markup: bool,
    /// „klucz = wartość” i „klucz: wartość” na początku wiersza (ini, toml, yaml, json).
    keys: bool,
    /// Słowa kluczowe bez względu na wielkość liter (SQL, pliki wsadowe).
    ci: bool,
}

const C_COMMENT: Option<(&str, &str)> = Some(("/*", "*/"));

fn lang(ext: &str) -> Option<Lang> {
    let l = match ext {
        "rs" => Lang { line: &["//"], block: C_COMMENT, words: RUST, markup: false, keys: false, ci: false },
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "java" | "c" | "h" | "cpp" | "hpp" | "cs" | "go" | "kt" | "kts" | "swift" | "dart" | "php" | "scss"
        | "gradle" | "lua" => Lang { line: &["//", "--"], block: C_COMMENT, words: CLIKE, markup: false, keys: false, ci: false },
        "py" | "rb" => Lang { line: &["#"], block: None, words: PY, markup: false, keys: false, ci: false },
        "sh" | "ps1" | "bat" | "cmd" => Lang { line: &["#", "::", "rem "], block: None, words: SH, markup: false, keys: false, ci: true },
        "sql" => Lang { line: &["--"], block: C_COMMENT, words: SQL, markup: false, keys: false, ci: true },
        "css" => Lang { line: &[], block: C_COMMENT, words: CSS, markup: false, keys: false, ci: false },
        "html" | "htm" | "xml" | "svg" | "vue" | "svelte" | "xaml" | "xhtml" => Lang { line: &[], block: None, words: &[], markup: true, keys: false, ci: false },
        "json" => Lang { line: &["//"], block: C_COMMENT, words: &["true", "false", "null"], markup: false, keys: true, ci: false },
        "toml" | "ini" | "cfg" | "conf" | "properties" | "yml" | "yaml" | "reg" | "env" => {
            Lang { line: &["#", ";"], block: None, words: &["true", "false", "null", "yes", "no"], markup: false, keys: true, ci: true }
        }
        _ => return None,
    };
    Some(l)
}

/// Czy dla tego rozszerzenia mamy podświetlanie.
pub fn supported(ext: &str) -> bool {
    lang(ext).is_some()
}

/// Dzieli tekst na wiersze z pokolorowanymi kawałkami. `max_lines` ucina długie pliki,
/// `max_cols` – długie wiersze (w panelu i tak nie byłoby ich widać).
pub fn highlight(text: &str, ext: &str, max_lines: usize, max_cols: usize) -> Vec<Vec<Piece>> {
    let Some(l) = lang(ext) else { return Vec::new() };
    let src: Vec<char> = text.chars().collect();
    let mut lines: Vec<Vec<Piece>> = Vec::new();
    let mut cur: Vec<Piece> = Vec::new();
    let mut i = 0;
    // Stan znaczników: czy jesteśmy w środku `<tag …>`.
    let mut in_tag = false;
    while i < src.len() {
        if lines.len() >= max_lines {
            break;
        }
        let c = src[i];
        if c == '\n' {
            lines.push(std::mem::take(&mut cur));
            i += 1;
            continue;
        }
        if c == '\r' {
            i += 1;
            continue;
        }
        // Komentarz blokowy (może iść przez wiele wierszy).
        if let Some((open, close)) = l.block {
            if starts(&src, i, open) {
                let end = find(&src, i + open.len(), close).map(|e| e + close.len()).unwrap_or(src.len());
                i = emit(&src, i, end, Tok::Comment, &mut cur, &mut lines, max_cols);
                continue;
            }
        }
        if l.markup && starts(&src, i, "<!--") {
            let end = find(&src, i + 4, "-->").map(|e| e + 3).unwrap_or(src.len());
            i = emit(&src, i, end, Tok::Comment, &mut cur, &mut lines, max_cols);
            continue;
        }
        // Komentarz do końca wiersza.
        if l.line.iter().any(|o| starts_ci(&src, i, o)) {
            let end = src[i..].iter().position(|c| *c == '\n').map(|p| i + p).unwrap_or(src.len());
            i = emit(&src, i, end, Tok::Comment, &mut cur, &mut lines, max_cols);
            continue;
        }
        // Napis – do zamknięcia tym samym znakiem, z `\` jako ucieczką.
        if c == '"' || c == '\'' || (c == '`' && !l.markup) {
            let mut j = i + 1;
            while j < src.len() {
                if src[j] == '\\' {
                    j += 2;
                    continue;
                }
                if src[j] == c {
                    j += 1;
                    break;
                }
                // Apostrof w zwykłym tekście (np. „don't”) nie otwiera napisu na pół pliku.
                if src[j] == '\n' && c != '`' {
                    break;
                }
                j += 1;
            }
            i = emit(&src, i, j.min(src.len()), Tok::Str, &mut cur, &mut lines, max_cols);
            continue;
        }
        if l.markup {
            if c == '<' {
                in_tag = true;
                let mut j = i + 1;
                while j < src.len() && (src[j].is_alphanumeric() || "/_:-!?".contains(src[j])) {
                    j += 1;
                }
                i = emit(&src, i, j, Tok::Tag, &mut cur, &mut lines, max_cols);
                continue;
            }
            if c == '>' {
                in_tag = false;
                i = emit(&src, i, i + 1, Tok::Tag, &mut cur, &mut lines, max_cols);
                continue;
            }
            if in_tag && (c.is_alphabetic() || c == '_') {
                let mut j = i;
                while j < src.len() && (src[j].is_alphanumeric() || "_:-.".contains(src[j])) {
                    j += 1;
                }
                i = emit(&src, i, j, Tok::Attr, &mut cur, &mut lines, max_cols);
                continue;
            }
        }
        if c.is_ascii_digit() {
            let mut j = i;
            while j < src.len() && (src[j].is_ascii_alphanumeric() || src[j] == '.' || src[j] == '_') {
                j += 1;
            }
            i = emit(&src, i, j, Tok::Num, &mut cur, &mut lines, max_cols);
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '$' || c == '@' || c == '#' {
            let mut j = i;
            while j < src.len() && (src[j].is_alphanumeric() || "_$-".contains(src[j])) {
                j += 1;
            }
            let word: String = src[i..j].iter().collect();
            let keyword = l.words.iter().any(|w| if l.ci { w.eq_ignore_ascii_case(&word) } else { *w == word });
            let kind = if keyword {
                Tok::Key
            } else if l.keys && key_at_line_start(&cur) && value_follows(&src, j) {
                Tok::Attr
            } else if word.starts_with(|c: char| c.is_uppercase()) || src.get(j).is_some_and(|c| *c == '(') {
                Tok::Type
            } else {
                Tok::Plain
            };
            i = emit(&src, i, j, kind, &mut cur, &mut lines, max_cols);
            continue;
        }
        let kind = if c.is_whitespace() { Tok::Plain } else { Tok::Punct };
        i = emit(&src, i, i + 1, kind, &mut cur, &mut lines, max_cols);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Przed tym słowem w wierszu nie było jeszcze nic poza odstępami – czyli to klucz.
fn key_at_line_start(cur: &[Piece]) -> bool {
    cur.iter().all(|(k, t)| *k == Tok::Plain && t.trim().is_empty())
}

/// Po słowie (i odstępach) stoi `=` albo `:` – wtedy to nazwa, nie wartość.
fn value_follows(src: &[char], from: usize) -> bool {
    src[from..].iter().find(|c| !c.is_whitespace() || **c == '\n').is_some_and(|c| *c == '=' || *c == ':')
}

fn starts(src: &[char], at: usize, pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, c)| src.get(at + k) == Some(&c))
}

fn starts_ci(src: &[char], at: usize, pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, c)| src.get(at + k).is_some_and(|s| s.eq_ignore_ascii_case(&c)))
}

fn find(src: &[char], from: usize, pat: &str) -> Option<usize> {
    (from..src.len()).find(|i| starts(src, *i, pat))
}

/// Dokłada kawałek `src[from..to]` do wiersza, dzieląc go na znakach nowego wiersza.
fn emit(src: &[char], from: usize, to: usize, kind: Tok, cur: &mut Vec<Piece>, lines: &mut Vec<Vec<Piece>>, max_cols: usize) -> usize {
    let mut text = String::new();
    for &c in &src[from..to.min(src.len())] {
        if c == '\n' {
            push(cur, kind, std::mem::take(&mut text), max_cols);
            lines.push(std::mem::take(cur));
            continue;
        }
        if c == '\t' {
            // Tabulator rozwijamy na cztery miejsca, a odstępy robimy nierozdzielające,
            // żeby wcięcia nie znikły przy rysowaniu kawałków osobnymi polami tekstu.
            text.push_str("\u{a0}\u{a0}\u{a0}\u{a0}");
        } else if c == ' ' {
            text.push('\u{a0}');
        } else if c != '\r' {
            text.push(c);
        }
    }
    push(cur, kind, text, max_cols);
    to.max(from + 1)
}

/// Dokłada tekst do wiersza; ten samego rodzaju łączymy, żeby kawałków było jak najmniej.
fn push(cur: &mut Vec<Piece>, kind: Tok, text: String, max_cols: usize) {
    if text.is_empty() {
        return;
    }
    let len: usize = cur.iter().map(|(_, t)| t.chars().count()).sum();
    if len >= max_cols {
        return;
    }
    let text = if len + text.chars().count() > max_cols { text.chars().take(max_cols - len).collect() } else { text };
    match cur.last_mut() {
        Some((k, t)) if *k == kind => t.push_str(&text),
        _ => cur.push((kind, text)),
    }
}

/// Markdown do czytania: bez znaczników, z wyróżnieniami – nagłówki (słowo kluczowe), kod
/// (napis), linki (typ), cytaty (komentarz), punkty list jako „•”.
pub fn markdown(text: &str, max_lines: usize) -> Vec<Vec<Piece>> {
    let mut out = Vec::new();
    let mut code = false;
    for line in text.lines().take(max_lines) {
        let t = line.trim_end();
        if t.trim_start().starts_with("```") {
            code = !code;
            continue;
        }
        if code {
            out.push(vec![(Tok::Str, format!("  {t}"))]);
            continue;
        }
        let lead = t.trim_start();
        if let Some(h) = lead.strip_prefix('#') {
            let h = h.trim_start_matches('#').trim();
            out.push(vec![(Tok::Key, h.to_string())]);
            continue;
        }
        if let Some(q) = lead.strip_prefix('>') {
            out.push(vec![(Tok::Comment, format!("│ {}", q.trim()))]);
            continue;
        }
        let (prefix, body) = if let Some(r) = lead.strip_prefix("- ").or_else(|| lead.strip_prefix("* ")).or_else(|| lead.strip_prefix("+ ")) {
            (format!("{}• ", " ".repeat(t.len() - lead.len())), r)
        } else {
            (String::new(), t)
        };
        let mut pieces: Vec<Piece> = Vec::new();
        if !prefix.is_empty() {
            pieces.push((Tok::Punct, prefix));
        }
        inline(body, &mut pieces);
        out.push(pieces);
    }
    out
}

/// Wnętrze wiersza: `kod`, **pogrubienie** / *kursywa* (bez gwiazdek), [tekst](adres) jako tekst.
fn inline(s: &str, out: &mut Vec<Piece>) {
    let mut plain = String::new();
    let mut rest = s;
    let flush = |plain: &mut String, out: &mut Vec<Piece>| {
        if !plain.is_empty() {
            out.push((Tok::Plain, std::mem::take(plain)));
        }
    };
    while let Some(c) = rest.chars().next() {
        if c == '`' {
            if let Some(end) = rest[1..].find('`') {
                flush(&mut plain, out);
                out.push((Tok::Str, rest[1..1 + end].to_string()));
                rest = &rest[end + 2..];
                continue;
            }
        }
        if c == '[' {
            if let Some(close) = rest.find("](") {
                if let Some(end) = rest[close..].find(')') {
                    flush(&mut plain, out);
                    out.push((Tok::Type, rest[1..close].to_string()));
                    rest = &rest[close + end + 1..];
                    continue;
                }
            }
        }
        if c == '*' || c == '_' {
            let n = rest.chars().take_while(|&x| x == c).count();
            rest = &rest[n..];
            continue;
        }
        plain.push(c);
        rest = &rest[c.len_utf8()..];
    }
    flush(&mut plain, out);
}
