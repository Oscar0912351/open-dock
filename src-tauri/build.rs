fn main() {
    build_cpu_sensor();
    tauri_build::build();

    #[cfg(target_os = "windows")]
    println!(
        "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
    );
}

fn build_cpu_sensor() {
    use sha2::{Digest, Sha256};
    use std::{fs, path::PathBuf, process::Command};

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86_64")
    {
        return;
    }

    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let resources = root.join("resources/cpu-sensor");
    let manifest = resources.join("libraries.json");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let raw = fs::read(&manifest).expect("CPU sensor library manifest missing");
    let metadata: serde_json::Value =
        serde_json::from_slice(&raw).expect("Invalid CPU sensor library manifest");

    let mut fingerprint = Sha256::new();
    let build_rs_path = root.join("build.rs");
    println!("cargo:rerun-if-changed={}", build_rs_path.display());
    fingerprint.update(fs::read(&build_rs_path).unwrap());
    fingerprint.update(&raw);

    for entry in metadata["files"]
        .as_array()
        .expect("CPU sensor library list missing")
    {
        let name = entry["file"].as_str().unwrap();
        assert!(!name.contains(['/', '\\']) && name.ends_with(".dll"));
        let path = resources.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(&path).expect("Pinned CPU sensor library missing");
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            entry["sha256"].as_str().unwrap(),
            "CPU sensor library hash mismatch: {name}"
        );
        fingerprint.update(bytes);
    }

    let sources = [
        root.join("sensors/TemperatureSelection.cs"),
        root.join("sensors/Program.cs"),
    ];
    for path in &sources {
        println!("cargo:rerun-if-changed={}", path.display());
        fingerprint.update(fs::read(path).unwrap());
    }

    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let hash = format!("{:x}", fingerprint.finalize());
    // Framework csc embeds a new assembly ID on each compilation. Cargo check,
    // test and build use different OUT_DIRs, but must package identical bytes.
    let cache = root.join("target/cpu-sensor").join(hash);
    fs::create_dir_all(&cache).unwrap();
    let exe = cache.join("OpenDock.CpuSensors.exe");
    if !exe.is_file() {
        let compiler =
            PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
                .join("Microsoft.NET/Framework64/v4.0.30319/csc.exe");
        let mut command = Command::new(&compiler);
        let windows_path = |path: &std::path::Path| path.to_string_lossy().replace('/', "\\");
        command
            .args(["/nologo", "/optimize+", "/target:exe", "/platform:x64"])
            .arg(format!("/out:{}", windows_path(&exe)))
            .arg(format!(
                "/reference:{}",
                windows_path(&resources.join("LibreHardwareMonitorLib.dll"))
            ))
            .arg("/reference:System.Web.Extensions.dll")
            .args(sources.iter().map(|path| windows_path(path)));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let result = command
            .output()
            .expect("Windows .NET Framework C# compiler is required to build CPU sensor helper");
        assert!(
            result.status.success(),
            "CPU sensor helper build failed: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    let bytes = fs::read(exe).unwrap();
    for destination in [
        out.join("OpenDock.CpuSensors.exe"),
        resources.join("OpenDock.CpuSensors.exe"),
    ] {
        if fs::read(&destination).ok().as_deref() != Some(bytes.as_slice()) {
            fs::write(destination, &bytes).unwrap();
        }
    }
}
