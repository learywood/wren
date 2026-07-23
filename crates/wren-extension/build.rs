use std::{env, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let rustc = env::var_os("RUSTC").expect("Cargo should provide the Rust compiler path");
    let output = Command::new(rustc)
        .arg("--version")
        .arg("--verbose")
        .output()
        .expect("the Rust compiler version should be readable");
    assert!(
        output.status.success(),
        "the Rust compiler version should be readable"
    );

    let version =
        String::from_utf8(output.stdout).expect("the Rust compiler version should be UTF-8");
    let commit = version
        .lines()
        .find_map(|line| line.strip_prefix("commit-hash: "))
        .expect("the Rust compiler should report its commit hash");
    let target = env::var("TARGET").expect("Cargo should provide the compilation target");
    let profile = env::var("PROFILE").expect("Cargo should provide the compilation profile");
    let panic = env::var("CARGO_CFG_PANIC").unwrap_or_else(|_| String::from("unwind"));

    println!("cargo:rustc-env=WREN_EXTENSION_RUSTC_COMMIT={commit}");
    println!("cargo:rustc-env=WREN_EXTENSION_TARGET={target}");
    println!("cargo:rustc-env=WREN_EXTENSION_PROFILE={profile}");
    println!("cargo:rustc-env=WREN_EXTENSION_PANIC={panic}");
}
