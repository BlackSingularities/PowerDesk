//! Podstawowe informacje o komputerze do pulpitu „Komputer”.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use sysinfo::System;

static SYS: Lazy<Mutex<System>> = Lazy::new(|| {
    let mut s = System::new();
    s.refresh_cpu_usage();
    s.refresh_memory();
    Mutex::new(s)
});

#[derive(Serialize)]
pub struct Info {
    pub host: String,
    pub os: String,
    pub cpu: String,
    pub cores: usize,
    pub threads: usize,
    pub cpu_usage: f32,
    pub mem_total: u64,
    pub mem_used: u64,
    pub uptime: u64,
}

pub fn info() -> Info {
    let mut s = SYS.lock();
    // Zużycie procesora liczy się z różnicy między odczytami – kolejne wywołania co kilka sekund są dokładne.
    s.refresh_cpu_usage();
    s.refresh_memory();
    Info {
        host: System::host_name().unwrap_or_default(),
        os: System::long_os_version().unwrap_or_default(),
        cpu: s.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
        cores: s.physical_core_count().unwrap_or(0),
        threads: s.cpus().len(),
        cpu_usage: s.global_cpu_usage(),
        mem_total: s.total_memory(),
        mem_used: s.used_memory(),
        uptime: System::uptime(),
    }
}
