//! Prędkość sieci dla widżetu: suma bajtów odebranych i wysłanych przez działające karty
//! sieciowe (bez pętli zwrotnej i kart wirtualnych bez ruchu), liczona z różnicy odczytów.

use parking_lot::Mutex;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

static LAST: Mutex<Option<(std::time::Instant, u64, u64)>> = Mutex::new(None);

/// Bajty odebrane i wysłane od startu systemu przez działające karty (do własnych pomiarów).
pub fn totals() -> Option<(u64, u64)> {
    unsafe {
        let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut t).is_err() || t.is_null() {
            return None;
        }
        let table = &*t;
        let rows = std::slice::from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize);
        let (mut down, mut up) = (0u64, 0u64);
        for r in rows {
            // 24 – pętla zwrotna; liczymy tylko połączone karty fizyczne i Wi-Fi.
            if r.Type == 24 || r.OperStatus != IfOperStatusUp || r.InterfaceAndOperStatusFlags._bitfield & 1 == 0 {
                continue;
            }
            down += r.InOctets;
            up += r.OutOctets;
        }
        FreeMibTable(t as *const _);
        Some((down, up))
    }
}

/// Bajty na sekundę (pobieranie, wysyłanie) od poprzedniego wywołania; pierwsze – zera.
pub fn sample() -> (f64, f64) {
    let Some((d, u)) = totals() else { return (0.0, 0.0) };
    let now = std::time::Instant::now();
    let mut last = LAST.lock();
    let out = match *last {
        Some((t, pd, pu)) => {
            let secs = now.duration_since(t).as_secs_f64().max(0.001);
            (d.saturating_sub(pd) as f64 / secs, u.saturating_sub(pu) as f64 / secs)
        }
        None => (0.0, 0.0),
    };
    *last = Some((now, d, u));
    out
}
