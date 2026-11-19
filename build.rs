fn find_rc() -> Option<std::path::PathBuf> {
    if std::process::Command::new("rc.exe")
        .arg("/?")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return Some("rc.exe".into());
    }
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        _ => "x86",
    };
    if let Some(bin) = std::env::var_os("WindowsSdkVerBinPath") {
        let candidate = std::path::PathBuf::from(bin).join(arch).join("rc.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    for env_key in ["ProgramFiles(x86)", "ProgramFiles"] {
        let Some(root) = std::env::var_os(env_key) else {
            continue;
        };
        let kits_bin = std::path::PathBuf::from(root).join("Windows Kits\\10\\bin");
        let Ok(entries) = std::fs::read_dir(kits_bin) else {
            continue;
        };
        let mut versions: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        versions.sort();
        for dir in versions.into_iter().rev() {
            let candidate = dir.join(arch).join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-changed=raptim.manifest");
    println!("cargo:rerun-if-changed=raptim.rc");
    println!("cargo:rerun-if-changed=ui/icons/raptim.ico");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        && let Some(dir) = std::env::var_os("CARGO_MANIFEST_DIR")
    {
        let dir = std::path::PathBuf::from(dir);
        let manifest = dir.join("raptim.manifest");
        println!("cargo:rustc-link-arg-bin=raptim=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=raptim=/MANIFESTINPUT:{}",
            manifest.display()
        );
        if let (Some(rc), Some(out_dir)) = (find_rc(), std::env::var_os("OUT_DIR")) {
            let res = std::path::PathBuf::from(out_dir).join("raptim.res");
            let status = std::process::Command::new(rc)
                .current_dir(&dir)
                .args(["/nologo", "/fo"])
                .arg(&res)
                .arg(dir.join("raptim.rc"))
                .status()
                .expect("Failed to execute rc.exe");
            assert!(status.success(), "rc.exe failed to compile raptim.rc");
            println!("cargo:rustc-link-arg-bin=raptim={}", res.display());
        }
    }
    let rustc = std::env::var_os("RUSTC").expect("Cargo must provide RUSTC");
    let output = std::process::Command::new(rustc)
        .arg("--version")
        .output()
        .expect("Cannot read Rust compiler version");
    assert!(output.status.success(), "rustc --version failed");
    let version = String::from_utf8(output.stdout).expect("Rust version must be UTF-8");
    let version = version
        .split_whitespace()
        .nth(1)
        .expect("Missing Rust version");
    println!("cargo:rustc-env=RAPTIM_RUST_VERSION={version}");
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .expect("Slint UI compilation failed");
}
