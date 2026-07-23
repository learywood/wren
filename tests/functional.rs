use std::{env, path::PathBuf, process::Command};

#[test]
fn harness_starts_and_stops() {
    let mut harness = Command::new(env!("CARGO_BIN_EXE_wren"))
        .spawn()
        .expect("compiled Wren harness should start");

    let status = harness.wait().expect("Wren harness should stop");

    assert!(status.success(), "Wren harness exited with {status}");
}

#[test]
fn harness_loads_and_executes_extension() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = repository.join("target/functional-fixture");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let build = Command::new(cargo)
        .current_dir(&repository)
        .args([
            "build",
            "--quiet",
            "--package",
            "wren-fixture-extension",
            "--target-dir",
        ])
        .arg(&target)
        .status()
        .expect("fixture extension should build");
    assert!(
        build.success(),
        "fixture extension build exited with {build}"
    );

    let library = target.join("debug").join(format!(
        "{}wren_fixture_extension{}",
        env::consts::DLL_PREFIX,
        env::consts::DLL_SUFFIX
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_wren"))
        .arg("--extension")
        .arg(&library)
        .output()
        .expect("compiled Wren harness should execute");

    assert!(
        output.status.success(),
        "Wren harness exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("harness output should be UTF-8"),
        "initialized extension: functional-test-fixture\n"
    );
}
