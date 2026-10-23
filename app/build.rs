//! Ikony programu (`icons.ts` – nazwa: ścieżka SVG) – generujemy z nich globalny `Icons` dla Slint.

use std::fmt::Write;

fn main() {
    let src = "icons.ts";
    println!("cargo:rerun-if-changed={src}");
    let ts = std::fs::read_to_string(src).expect("icons.ts");
    let mut rs = String::from("/// Ścieżki SVG ikon po nazwie z icons.ts.
pub fn icon(name: &str) -> &'static str {
    match name {
");
    let mut out = String::from("// Wygenerowane przez build.rs z app/icons.ts – nie edytować.\nexport global Icons {\n");
    for line in ts.lines() {
        let l = line.trim();
        // Linie w postaci `nazwa: 'ścieżka SVG',`
        let Some((name, rest)) = l.split_once(": '") else { continue };
        let Some(path) = rest.strip_suffix("',") else { continue };
        if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let _ = writeln!(out, "    out property <string> {}: \"{}\";", kebab(name), path);
        let _ = writeln!(rs, "        \"{name}\" => \"{path}\",");
    }
    out.push_str("}\n");
    rs.push_str("        _ => \"\",\n    }\n}\n");
    std::fs::write("ui/icons.slint", out).expect("ui/icons.slint");
    std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons.rs"), rs).expect("icons.rs");
    // Ikona i opis pliku .exe (Eksplorator, pasek zadań, „Aplikacje i funkcje”). Brak narzędzia
    // do zasobów (rc.exe / windres) – budowa idzie dalej, tylko bez ikony w pliku.
    println!("cargo:rerun-if-changed=icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icon.ico").set("FileDescription", "PowerDesk").set("ProductName", "PowerDesk").set("CompanyName", "adam001d");
        if let Err(e) = res.compile() {
            println!("cargo:warning=ikona .exe pominięta: {e}");
        }
    }
    slint_build::compile_with_config("ui/main.slint", slint_build::CompilerConfiguration::new().with_style("fluent".into())).expect("slint");
}

fn kebab(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            o.push('-');
            o.push(c.to_ascii_lowercase());
        } else {
            o.push(c);
        }
    }
    o
}
