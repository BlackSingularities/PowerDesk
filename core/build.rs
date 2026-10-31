fn main() {
    test_manifest();
    embed_dialog();
}

/// Biblioteka okien „Otwórz”/„Zapisz jako” (skrzynka `dialog`) osadzona w programie – ścieżki do
/// gotowych plików .dll w zmiennych POWERDESK_DIALOG_DLL64 i POWERDESK_DIALOG_DLL32 (ustawia je
/// budowa wydania). Bez nich osadzamy puste pliki: program działa, tylko bez tej integracji.
fn embed_dialog() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    for (var, name) in [("POWERDESK_DIALOG_DLL64", "dialog64.dll"), ("POWERDESK_DIALOG_DLL32", "dialog32.dll")] {
        println!("cargo:rerun-if-env-changed={var}");
        let data = match std::env::var(var) {
            Ok(p) if !p.is_empty() => {
                println!("cargo:rerun-if-changed={p}");
                let d = std::fs::read(&p).unwrap_or_else(|e| panic!("{var}={p}: {e}"));
                assert!(d.starts_with(b"MZ"), "{var}={p}: to nie jest biblioteka .dll");
                d
            }
            _ => Vec::new(),
        };
        std::fs::write(out.join(name), data).expect("dialog dll");
    }
}

/// Testy linkują kod korzystający z Common Controls 6 (okna dialogowe powłoki), więc bez manifestu
/// nie startują (STATUS_ENTRYPOINT_NOT_FOUND). Aplikacja dostaje manifest od tauri-build, testy – stąd.
fn test_manifest() {
    println!("cargo:rerun-if-env-changed=OFP_TEST_MANIFEST");
    const DEP: &str = "type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'";
    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        // Argument trafia też do programu (dwa manifesty), więc tylko na życzenie przy testach:
        // OFP_TEST_MANIFEST=1 cargo test --test fs_ops
        Ok("gnu") if std::env::var("OFP_TEST_MANIFEST").is_ok() => {
            // Manifest jako zasób skompilowany przez windres (jeśli jest dostępny).
            let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
            let xml = out.join("tests.manifest");
            let rc = out.join("tests-manifest.rc");
            let obj = out.join("tests-manifest.o");
            let manifest = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\"><dependency><dependentAssembly><assemblyIdentity {} /></dependentAssembly></dependency></assembly>\n",
                DEP.replace('\'', "\"")
            );
            if std::fs::write(&xml, manifest).is_err() || std::fs::write(&rc, format!("1 24 \"{}\"\n", xml.display().to_string().replace('\\', "/"))).is_err() {
                return;
            }
            let ok = std::process::Command::new("windres").arg(&rc).arg("-O").arg("coff").arg("-o").arg(&obj).status().map(|s| s.success()).unwrap_or(false);
            if ok {
                println!("cargo:rustc-link-arg-tests={}", obj.display());
            }
        }
        _ => {}
    }
}
