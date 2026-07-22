use std::process::Command;

#[test]
fn harness_starts_and_stops() {
    let mut harness = Command::new(env!("CARGO_BIN_EXE_wren"))
        .spawn()
        .expect("compiled Wren harness should start");

    let status = harness.wait().expect("Wren harness should stop");

    assert!(status.success(), "Wren harness exited with {status}");
}
