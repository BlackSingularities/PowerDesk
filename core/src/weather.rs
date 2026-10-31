//! Pogoda dla widżetu: Open-Meteo (bez klucza). Najpierw nazwa miejscowości na współrzędne
//! (geokodowanie), potem bieżąca temperatura i prognoza na kilka dni. Wysyłamy tylko nazwę miejscowości.

use serde_json::Value;

/// Rodzaj pogody do ikony: 0 – bezchmurnie, 1 – częściowe zachmurzenie, 2 – pochmurno, 3 – mgła,
/// 4 – deszcz, 5 – śnieg, 6 – burza.
pub fn kind(code: i64) -> u8 {
    match code {
        0 => 0,
        1 | 2 => 1,
        3 => 2,
        45 | 48 => 3,
        51..=67 | 80..=82 => 4,
        71..=77 | 85 | 86 => 5,
        95..=99 => 6,
        _ => 2,
    }
}

/// Opis pogody po polsku (kody WMO).
pub fn describe(code: i64) -> &'static str {
    match code {
        0 => "Bezchmurnie",
        1 => "Przeważnie słonecznie",
        2 => "Częściowe zachmurzenie",
        3 => "Pochmurno",
        45 | 48 => "Mgła",
        51 | 53 | 55 => "Mżawka",
        56 | 57 => "Marznąca mżawka",
        61 | 63 => "Deszcz",
        65 => "Ulewa",
        66 | 67 => "Marznący deszcz",
        71 | 73 => "Śnieg",
        75 => "Intensywny śnieg",
        77 => "Ziarna śniegu",
        80..=82 => "Przelotny deszcz",
        85 | 86 => "Przelotny śnieg",
        95 => "Burza",
        96..=99 => "Burza z gradem",
        _ => "",
    }
}

#[derive(Clone, Debug, Default)]
pub struct Day {
    pub date: String,
    pub code: i64,
    pub max: f64,
    pub min: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Weather {
    /// Nazwa miejscowości z geokodowania (z poprawną pisownią).
    pub place: String,
    pub temp: f64,
    pub code: i64,
    pub wind: f64,
    pub days: Vec<Day>,
    /// Temperatura odczuwalna i wilgotność (%).
    pub feels: f64,
    pub humidity: f64,
    /// Wschód i zachód słońca dziś („06:42”).
    pub sunrise: String,
    pub sunset: String,
    /// Temperatury na kolejne 24 godziny od bieżącej (co godzinę) i godzina pierwszej.
    pub hours: Vec<f64>,
    pub hour0: u32,
}

/// JSON spod adresu (też dla innych widżetów, np. kursów NBP).
pub fn get_json(url: &str) -> Result<Value, String> {
    get(url)
}

fn get(url: &str) -> Result<Value, String> {
    ureq::get(url)
        .set("User-Agent", "PowerDesk")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Pogoda dla miejscowości (nazwa po polsku albo w innym języku).
pub fn fetch(city: &str) -> Result<Weather, String> {
    let g = get(&format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=pl&format=json", enc(city.trim())))?;
    let place = g["results"].get(0).ok_or_else(|| format!("Nie znaleziono miejscowości „{city}”"))?;
    let (lat, lon) = (place["latitude"].as_f64().unwrap_or(0.0), place["longitude"].as_f64().unwrap_or(0.0));
    let f = get(&format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current=temperature_2m,weather_code,wind_speed_10m,apparent_temperature,relative_humidity_2m&hourly=temperature_2m&daily=weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset&timezone=auto&forecast_days=7"
    ))?;
    let d = &f["daily"];
    let n = d["time"].as_array().map_or(0, |a| a.len());
    let days = (0..n)
        .map(|i| Day {
            date: d["time"][i].as_str().unwrap_or_default().to_string(),
            code: d["weather_code"][i].as_i64().unwrap_or(0),
            max: d["temperature_2m_max"][i].as_f64().unwrap_or(0.0),
            min: d["temperature_2m_min"][i].as_f64().unwrap_or(0.0),
        })
        .collect();
    // Godziny prognozy od bieżącej (czas lokalny miejscowości – „2026-10-31T14:00”).
    let now = f["current"]["time"].as_str().unwrap_or_default().get(..13).unwrap_or_default().to_string();
    let times = f["hourly"]["time"].as_array().cloned().unwrap_or_default();
    let start = times.iter().position(|t| t.as_str().is_some_and(|t| t.starts_with(&now))).unwrap_or(0);
    let hours: Vec<f64> = (start..(start + 24).min(times.len())).filter_map(|i| f["hourly"]["temperature_2m"][i].as_f64()).collect();
    let hour0 = now.get(11..13).and_then(|h| h.parse().ok()).unwrap_or(0);
    let clock = |v: &Value| v.as_str().and_then(|t| t.get(11..16)).unwrap_or_default().to_string();
    Ok(Weather {
        feels: f["current"]["apparent_temperature"].as_f64().unwrap_or(0.0),
        humidity: f["current"]["relative_humidity_2m"].as_f64().unwrap_or(0.0),
        sunrise: clock(&d["sunrise"][0]),
        sunset: clock(&d["sunset"][0]),
        hours,
        hour0,
        place: place["name"].as_str().unwrap_or(city).to_string(),
        temp: f["current"]["temperature_2m"].as_f64().unwrap_or(0.0),
        code: f["current"]["weather_code"].as_i64().unwrap_or(0),
        wind: f["current"]["wind_speed_10m"].as_f64().unwrap_or(0.0),
        days,
    })
}
