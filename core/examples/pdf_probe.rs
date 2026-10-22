//! Próba PDF: `cargo run -p powerdesk-core --example pdf_probe -- plik.pdf`
fn main() {
    let path = std::env::args().nth(1).expect("ścieżka");
    println!("stron: {:?}", powerdesk_core::pdf::page_count(&path));
    let r = powerdesk_core::pdf::render(&path, 1, 600).expect("render");
    println!("PNG {} B, stron {}", r.0.len(), r.1);
}
