//! Ręczne sprawdzenie pomiarów nakładki na żywym systemie: `cargo test -p powerdesk-core --test perf_live -- --ignored --nocapture`.

#[test]
#[ignore]
fn fps_and_gpu() {
    powerdesk_core::fpsmon::start();
    powerdesk_core::gpumon::start();
    std::thread::sleep(std::time::Duration::from_secs(3));
    println!("ETW: {:?}", powerdesk_core::fpsmon::state());
    if let Some((pid, name, _)) = powerdesk_core::taskwin::foreground_app() {
        println!("na pierwszym planie: {name} ({pid}) → {:?}", powerdesk_core::fpsmon::stats(pid, 10).map(|s| (s.fps, s.avg_ms)));
    }
    println!("GPU: {:?}", powerdesk_core::gpumon::snapshot());
    powerdesk_core::fpsmon::stop();
}

#[test]
#[ignore]
fn virtual_desktops() {
    println!("pulpity: {:?}", powerdesk_core::vdesk::list());
}
