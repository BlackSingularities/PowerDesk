//! Próba odtwarzacza: `cargo run -p powerdesk-core --example media_probe -- plik.mp4`
fn main() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    let path = std::env::args().nth(1).expect("ścieżka");
    let mut p = powerdesk_core::media::Player::open(&path, true).expect("open");
    let mut frames = 0;
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(15));
        if let Some(f) = p.frame() {
            frames += 1;
            if frames == 30 {
                let lit = f.rgba.chunks(4).filter(|px| px[0] as u32 + px[1] as u32 + px[2] as u32 > 30).count();
                println!("klatka {}x{}, jasnych pikseli {lit}", f.width, f.height);
            }
        }
    }
    println!("{:?}, klatek {frames}", p.status());
}
