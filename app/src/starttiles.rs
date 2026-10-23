//! Kafelki menu Start jak w Windows 10: grupy z nazwami, kafelki w czterech rozmiarach
//! (siatka 6 pól na szerokość), foldery. Tu tylko dane i układ – rysuje start.slint, a menu
//! i zdarzenia prowadzi start.rs.

use crate::model::{StartGroup, StartPin};

/// Pola siatki w wierszu grupy.
pub const COLS: usize = 6;
/// Pole siatki z odstępem (48 px + 4 px).
pub const CELL: f32 = 52.0;
pub const GAP: f32 = 4.0;
/// Nagłówek grupy (nazwa) i odstęp między grupami.
pub const HEAD: f32 = 34.0;
pub const GROUP_GAP: f32 = 14.0;

/// Miejsce kafelka: grupa, kafelek, a w folderze – numer w folderze.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct At {
    pub group: usize,
    pub tile: usize,
    pub child: Option<usize>,
}

/// Kafelek ułożony na ekranie (piksele logiczne w obszarze kafelków).
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub at: At,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Nagłówek grupy i jej pionowy zakres (od nagłówka do końca kafelków).
#[derive(Clone, Copy, Debug)]
pub struct Head {
    pub group: usize,
    pub y: f32,
    pub bottom: f32,
}

/// Gdzie upuścić przeciągany kafelek.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// Przed tym kafelkiem (w jego grupie albo folderze).
    Before(At),
    /// Na środek kafelka – razem tworzą folder (albo dochodzi do istniejącego).
    Into(At),
    /// Na koniec grupy.
    End(usize),
    /// Pod ostatnią grupą – nowa grupa.
    NewGroup,
}

/// Stary zapis (sama lista ścieżek) – jedna grupa bez nazwy ze średnimi kafelkami.
pub fn migrate(pins: &mut Vec<String>, groups: &mut Vec<StartGroup>) -> bool {
    if pins.is_empty() || !groups.is_empty() {
        return false;
    }
    groups.push(StartGroup { name: String::new(), tiles: pins.iter().map(|p| StartPin::app(p)).collect() });
    pins.clear();
    true
}

/// Pola zajęte przez kafelki: kolejno, każdy w pierwszym wolnym miejscu od góry.
/// Zwraca (kolumna, wiersz) każdego kafelka i liczbę wierszy.
pub fn pack(tiles: &[StartPin]) -> (Vec<(usize, usize)>, usize) {
    let mut busy: Vec<[bool; COLS]> = Vec::new();
    let mut out = Vec::with_capacity(tiles.len());
    for t in tiles {
        let (w, h) = t.cells();
        let w = w.min(COLS);
        let mut r = 0;
        let spot = loop {
            while busy.len() < r + h {
                busy.push([false; COLS]);
            }
            let free = (0..=COLS - w).find(|&c| (r..r + h).all(|rr| (c..c + w).all(|cc| !busy[rr][cc])));
            if let Some(c) = free {
                break (c, r);
            }
            r += 1;
        };
        for row in busy.iter_mut().skip(spot.1).take(h) {
            for cell in row.iter_mut().skip(spot.0).take(w) {
                *cell = true;
            }
        }
        out.push(spot);
    }
    let rows = busy.iter().rposition(|r| r.iter().any(|&b| b)).map_or(0, |i| i + 1);
    (out, rows)
}

fn place(tiles: &[StartPin], top: f32, at: impl Fn(usize) -> At, out: &mut Vec<Placed>) -> f32 {
    let (spots, rows) = pack(tiles);
    for (i, (t, (c, r))) in tiles.iter().zip(spots).enumerate() {
        let (w, h) = t.cells();
        out.push(Placed { at: at(i), x: c as f32 * CELL, y: top + r as f32 * CELL, w: w as f32 * CELL - GAP, h: h as f32 * CELL - GAP });
    }
    top + rows as f32 * CELL
}

/// Układ: wszystkie grupy jedna pod drugą albo – gdy otwarty folder – tylko jego kafelki.
/// Zwraca kafelki, nagłówki i wysokość całości.
pub fn layout(groups: &[StartGroup], folder: Option<(usize, usize)>) -> (Vec<Placed>, Vec<Head>, f32) {
    let mut tiles = Vec::new();
    let mut heads = Vec::new();
    if let Some((g, t)) = folder {
        let Some(f) = groups.get(g).and_then(|gr| gr.tiles.get(t)) else { return (tiles, heads, 0.0) };
        let end = place(&f.items, 0.0, |i| At { group: g, tile: t, child: Some(i) }, &mut tiles);
        return (tiles, heads, end);
    }
    let mut y = 0.0;
    for (g, gr) in groups.iter().enumerate() {
        let top = y;
        let end = place(&gr.tiles, y + HEAD, |i| At { group: g, tile: i, child: None }, &mut tiles);
        heads.push(Head { group: g, y: top, bottom: end });
        y = end + GROUP_GAP;
    }
    (tiles, heads, y)
}

pub fn get<'a>(groups: &'a [StartGroup], at: At) -> Option<&'a StartPin> {
    let t = groups.get(at.group)?.tiles.get(at.tile)?;
    match at.child {
        Some(c) => t.items.get(c),
        None => Some(t),
    }
}

pub fn get_mut(groups: &mut [StartGroup], at: At) -> Option<&mut StartPin> {
    let t = groups.get_mut(at.group)?.tiles.get_mut(at.tile)?;
    match at.child {
        Some(c) => t.items.get_mut(c),
        None => Some(t),
    }
}

/// Czy aplikacja jest przypięta (także w folderze).
pub fn contains(groups: &[StartGroup], path: &str) -> bool {
    groups.iter().flat_map(|g| g.tiles.iter()).any(|t| t.path == path || t.items.iter().any(|c| c.path == path))
}

/// Przypina na koniec ostatniej grupy (bez grup – nowa).
pub fn pin(groups: &mut Vec<StartGroup>, path: &str) {
    if contains(groups, path) {
        return;
    }
    if groups.is_empty() {
        groups.push(StartGroup::default());
    }
    if let Some(g) = groups.last_mut() {
        g.tiles.push(StartPin::app(path));
    }
}

/// Widżet na koniec ostatniej grupy (bez grup – nowa), w domyślnym rozmiarze.
pub fn add_widget(groups: &mut Vec<StartGroup>, kind: &str, size: u8) {
    if groups.is_empty() {
        groups.push(StartGroup::default());
    }
    if let Some(g) = groups.last_mut() {
        g.tiles.push(StartPin { widget: kind.to_string(), size, ..Default::default() });
    }
}

/// Usuwa kafelek (widżet albo aplikację) z miejsca `at`.
pub fn remove(groups: &mut Vec<StartGroup>, at: At) {
    let _ = take(groups, at);
    cleanup(groups);
}

/// Odpina aplikację wszędzie, gdzie jest.
pub fn unpin(groups: &mut Vec<StartGroup>, path: &str) {
    for g in groups.iter_mut() {
        g.tiles.retain(|t| t.path != path || t.is_folder());
        for t in g.tiles.iter_mut() {
            t.items.retain(|c| c.path != path);
        }
    }
    cleanup(groups);
}

/// Porządek: folder z jednym kafelkiem staje się tym kafelkiem, puste foldery i grupy znikają.
pub fn cleanup(groups: &mut Vec<StartGroup>) {
    for g in groups.iter_mut() {
        for t in g.tiles.iter_mut() {
            if t.path.is_empty() && t.widget.is_empty() && t.items.len() == 1 {
                *t = t.items.remove(0);
            }
        }
        g.tiles.retain(|t| !t.path.is_empty() || !t.items.is_empty() || !t.widget.is_empty());
    }
    groups.retain(|g| !g.tiles.is_empty());
}

fn take(groups: &mut [StartGroup], at: At) -> Option<StartPin> {
    let tiles = &mut groups.get_mut(at.group)?.tiles;
    match at.child {
        Some(c) => {
            let f = tiles.get_mut(at.tile)?;
            (c < f.items.len()).then(|| f.items.remove(c))
        }
        None => (at.tile < tiles.len()).then(|| tiles.remove(at.tile)),
    }
}

/// Po zabraniu kafelka `from` miejsca dalej w tym samym pojemniku przesuwają się o jedno.
fn shift(from: At, mut to: At) -> At {
    match (from.child, to.child) {
        (None, _) if from.group == to.group && from.tile < to.tile => to.tile -= 1,
        (Some(fc), Some(tc)) if from.group == to.group && from.tile == to.tile && fc < tc => to.child = Some(tc - 1),
        _ => {}
    }
    to
}

/// Przenosi kafelek `from` w miejsce `target`.
pub fn drop(groups: &mut Vec<StartGroup>, from: At, target: Target) {
    if matches!(target, Target::Before(t) | Target::Into(t) if t == from) {
        return;
    }
    // Folderu ani widżetu nie wkładamy do folderu; widżet nie tworzy folderu.
    if let (Target::Into(t), Some(p)) = (target, get(groups, from)) {
        if p.is_folder() || !p.widget.is_empty() || get(groups, t).is_some_and(|d| !d.widget.is_empty()) {
            return;
        }
    }
    let Some(pin) = take(groups, from) else { return };
    match target {
        Target::Before(t) => {
            let t = shift(from, t);
            let Some(gr) = groups.get_mut(t.group) else { return };
            match t.child {
                Some(c) => {
                    if let Some(f) = gr.tiles.get_mut(t.tile) {
                        let c = c.min(f.items.len());
                        f.items.insert(c, pin);
                    }
                }
                None => {
                    let i = t.tile.min(gr.tiles.len());
                    gr.tiles.insert(i, pin);
                }
            }
        }
        Target::Into(t) => {
            let t = shift(from, t);
            if let Some(dest) = get_mut(groups, t) {
                if dest.is_folder() {
                    dest.items.push(pin);
                } else {
                    let first = std::mem::take(dest);
                    *dest = StartPin { size: first.size, items: vec![first, pin], ..Default::default() };
                }
            }
        }
        Target::End(g) => {
            if let Some(gr) = groups.get_mut(g) {
                gr.tiles.push(pin);
            }
        }
        Target::NewGroup => groups.push(StartGroup { name: String::new(), tiles: vec![pin] }),
    }
    cleanup(groups);
}

/// Co jest pod punktem upuszczenia (x, y w obszarze kafelków). Środek kafelka – folder,
/// reszta kafelka – miejsce przed nim; poza kafelkami – koniec grupy albo nowa grupa.
pub fn target(tiles: &[Placed], heads: &[Head], folder: bool, x: f32, y: f32) -> Target {
    if let Some(p) = tiles.iter().find(|p| x >= p.x && x < p.x + p.w + GAP && y >= p.y && y < p.y + p.h + GAP) {
        let inner = x > p.x + p.w * 0.25 && x < p.x + p.w * 0.75 && y > p.y + p.h * 0.25 && y < p.y + p.h * 0.75;
        return if inner && !folder { Target::Into(p.at) } else { Target::Before(p.at) };
    }
    if folder {
        // Koniec folderu – za ostatnim kafelkiem.
        return match tiles.last() {
            Some(p) => Target::Before(At { child: p.at.child.map(|c| c + 1), ..p.at }),
            None => Target::NewGroup,
        };
    }
    match heads.iter().find(|h| y >= h.y && y < h.bottom + GROUP_GAP) {
        Some(h) => Target::End(h.group),
        None => Target::NewGroup,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(paths: &[&str]) -> StartGroup {
        StartGroup { name: String::new(), tiles: paths.iter().map(|p| StartPin::app(p)).collect() }
    }

    #[test]
    fn pack_fills_rows() {
        let tiles: Vec<StartPin> = (0..4).map(|i| StartPin::app(&i.to_string())).collect();
        let (spots, rows) = pack(&tiles);
        assert_eq!(spots, vec![(0, 0), (2, 0), (4, 0), (0, 2)]);
        assert_eq!(rows, 4);
    }

    #[test]
    fn move_before_later_tile() {
        let mut gs = vec![g(&["a", "b", "c"])];
        drop(&mut gs, At { group: 0, tile: 0, child: None }, Target::Before(At { group: 0, tile: 2, child: None }));
        let names: Vec<_> = gs[0].tiles.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(names, ["b", "a", "c"]);
    }

    #[test]
    fn folder_and_dissolve() {
        let mut gs = vec![g(&["a", "b"])];
        drop(&mut gs, At { group: 0, tile: 0, child: None }, Target::Into(At { group: 0, tile: 1, child: None }));
        assert_eq!(gs[0].tiles.len(), 1);
        assert!(gs[0].tiles[0].is_folder());
        drop(&mut gs, At { group: 0, tile: 0, child: Some(0) }, Target::NewGroup);
        assert_eq!(gs.len(), 2);
        assert!(!gs[0].tiles[0].is_folder());
    }
}
