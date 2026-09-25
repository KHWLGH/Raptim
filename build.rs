fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");
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
    println!("cargo:rustc-env=DIASCIA_RUST_VERSION={version}");
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .expect("Slint UI compilation failed");
}
