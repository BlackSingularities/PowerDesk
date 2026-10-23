//! Kalkulator i przeliczanie jednostek dla wyszukiwania w menu Start: „2+2*3”, „(5,5-1)^2”,
//! „15% z 80”, „10 km na mi”, „72 f w c”, „3 gb na mb”.

/// Wynik dla zapytania, jeśli to działanie albo przeliczenie: (tekst wyniku, liczba do schowka).
pub fn evaluate(input: &str) -> Option<(String, String)> {
    let q = input.trim().to_lowercase().replace(',', ".");
    if let Some(r) = convert(&q) {
        return Some(r);
    }
    if let Some(r) = percent_of(&q) {
        return Some(r);
    }
    // Samo jedno słowo albo liczba to nie działanie.
    if !q.chars().any(|c| "+-*/^()".contains(c)) || q.chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    let mut p = Parser { s: q.as_bytes(), i: 0 };
    let v = p.expr()?;
    p.ws();
    if p.i != p.s.len() || !v.is_finite() {
        return None;
    }
    let n = num(v);
    Some((format!("= {n}"), n))
}

fn num(v: f64) -> String {
    let r = (v * 1e10).round() / 1e10;
    if r == r.trunc() && r.abs() < 1e15 {
        format!("{}", r as i64)
    } else {
        let s = format!("{r:.10}");
        s.trim_end_matches('0').trim_end_matches('.').replace('.', ",")
    }
}

fn percent_of(q: &str) -> Option<(String, String)> {
    let (a, b) = q.split_once("% z ").or_else(|| q.split_once("% of "))?;
    let (a, b): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    let n = num(a / 100.0 * b);
    Some((format!("= {n}"), n))
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i] == b' ' {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(c) = self.peek() {
            match c {
                b'+' => {
                    self.i += 1;
                    v += self.term()?;
                }
                b'-' => {
                    self.i += 1;
                    v -= self.term()?;
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        while let Some(c) = self.peek() {
            match c {
                b'*' | b'x' => {
                    self.i += 1;
                    v *= self.power()?;
                }
                b'/' | b':' => {
                    self.i += 1;
                    v /= self.power()?;
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn power(&mut self) -> Option<f64> {
        let b = self.unary()?;
        if self.peek() == Some(b'^') {
            self.i += 1;
            let e = self.power()?;
            return Some(b.powf(e));
        }
        Some(b)
    }
    fn unary(&mut self) -> Option<f64> {
        if self.peek() == Some(b'-') {
            self.i += 1;
            return Some(-self.unary()?);
        }
        self.atom()
    }
    fn atom(&mut self) -> Option<f64> {
        if self.peek() == Some(b'(') {
            self.i += 1;
            let v = self.expr()?;
            if self.peek() != Some(b')') {
                return None;
            }
            self.i += 1;
            return self.pct(v);
        }
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') {
            self.i += 1;
        }
        let v: f64 = std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()?;
        self.pct(v)
    }
    fn pct(&mut self, v: f64) -> Option<f64> {
        if self.peek() == Some(b'%') {
            self.i += 1;
            return Some(v / 100.0);
        }
        Some(v)
    }
}

/// Jednostki: (nazwy, rodzaj, mnożnik do jednostki podstawowej). Temperatura osobno.
const UNITS: [(&[&str], &str, f64); 26] = [
    (&["mm", "milimetr", "milimetrów"], "len", 0.001),
    (&["cm", "centymetr", "centymetrów"], "len", 0.01),
    (&["m", "metr", "metrów", "metry"], "len", 1.0),
    (&["km", "kilometr", "kilometrów"], "len", 1000.0),
    (&["in", "cal", "cali", "cale", "\""], "len", 0.0254),
    (&["ft", "stopa", "stóp", "stopy"], "len", 0.3048),
    (&["yd", "jard", "jardów"], "len", 0.9144),
    (&["mi", "mila", "mil", "mile"], "len", 1609.344),
    (&["mg"], "mass", 0.001),
    (&["g", "gram", "gramów"], "mass", 1.0),
    (&["dag", "deko"], "mass", 10.0),
    (&["kg", "kilogram", "kilogramów"], "mass", 1000.0),
    (&["t", "tona", "ton"], "mass", 1_000_000.0),
    (&["oz", "uncja", "uncji"], "mass", 28.349523125),
    (&["lb", "lbs", "funt", "funtów"], "mass", 453.59237),
    (&["ml", "mililitr"], "vol", 0.001),
    (&["l", "litr", "litrów"], "vol", 1.0),
    (&["gal", "galon", "galonów"], "vol", 3.785411784),
    (&["b", "bajt", "bajtów"], "data", 1.0),
    (&["kb"], "data", 1024.0),
    (&["mb"], "data", 1024.0 * 1024.0),
    (&["gb"], "data", 1024.0 * 1024.0 * 1024.0),
    (&["tb"], "data", 1024.0 * 1024.0 * 1024.0 * 1024.0),
    (&["s", "sek", "sekund"], "time", 1.0),
    (&["min", "minut"], "time", 60.0),
    (&["h", "godz", "godzin", "godziny"], "time", 3600.0),
];

fn unit(name: &str) -> Option<(&'static str, f64, &'static str)> {
    UNITS.iter().find(|u| u.0.contains(&name)).map(|u| (u.1, u.2, u.0[0]))
}

fn temp(name: &str) -> Option<char> {
    match name {
        "c" | "°c" | "celsjusz" | "celsjusza" | "celsjusze" => Some('c'),
        "f" | "°f" | "fahrenheit" | "fahrenheita" => Some('f'),
        "k" | "kelwin" | "kelwinów" => Some('k'),
        _ => None,
    }
}

/// „<liczba> <jednostka> (na|w|to|->|=) <jednostka>”.
fn convert(q: &str) -> Option<(String, String)> {
    let words: Vec<&str> = q.split_whitespace().collect();
    // Liczba sklejona z jednostką („10km na mi”) – rozdzielamy.
    let (value, from, rest): (f64, String, Vec<&str>) = {
        let first = *words.first()?;
        let split = first.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(first.len());
        if split > 0 && split < first.len() {
            (first[..split].parse().ok()?, first[split..].to_string(), words[1..].to_vec())
        } else {
            (first.parse().ok()?, words.get(1)?.to_string(), words.get(2..)?.to_vec())
        }
    };
    let (sep, to) = (*rest.first()?, *rest.get(1)?);
    if !["na", "w", "to", "->", "=", "in"].contains(&sep) || rest.len() != 2 {
        return None;
    }
    if let (Some(a), Some(b)) = (temp(&from), temp(to)) {
        let c = match a {
            'c' => value,
            'f' => (value - 32.0) * 5.0 / 9.0,
            _ => value - 273.15,
        };
        let r = match b {
            'c' => c,
            'f' => c * 9.0 / 5.0 + 32.0,
            _ => c + 273.15,
        };
        let n = num((r * 100.0).round() / 100.0);
        let sym = |t: char| match t {
            'c' => "°C",
            'f' => "°F",
            _ => "K",
        };
        return Some((format!("{} {} = {n} {}", num(value), sym(a), sym(b)), n));
    }
    let (ka, fa, na) = unit(&from)?;
    let (kb, fb, nb) = unit(to)?;
    if ka != kb {
        return None;
    }
    let r = value * fa / fb;
    let n = num((r * 1e6).round() / 1e6);
    Some((format!("{} {na} = {n} {nb}", num(value)), n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        assert_eq!(evaluate("2+2*3").unwrap().1, "8");
        assert_eq!(evaluate("(5,5-1)^2").unwrap().1, "20,25");
        assert_eq!(evaluate("10/4").unwrap().1, "2,5");
        assert!(evaluate("notatka").is_none());
        assert!(evaluate("42").is_none());
    }

    #[test]
    fn percent() {
        assert_eq!(evaluate("15% z 80").unwrap().1, "12");
        assert_eq!(evaluate("200*10%").unwrap().1, "20");
    }

    #[test]
    fn units() {
        assert_eq!(evaluate("10 km na mi").unwrap().1, "6,213712");
        assert_eq!(evaluate("212 f w c").unwrap().1, "100");
        assert_eq!(evaluate("2gb na mb").unwrap().1, "2048");
        assert!(evaluate("10 km na kg").is_none());
    }
}
