use std::process::Command;

#[test]
fn upgrade_help_lists_supported_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_flup"))
        .args(["upgrade", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--project"));
    assert!(help.contains("--flutter"));
    assert!(help.contains("--base-flutter"));
    assert!(help.contains("--no-git"));
    assert!(help.contains("--force"));
}

#[test]
fn unknown_command_is_rejected() {
    let output = Command::new(env!("CARGO_BIN_EXE_flup"))
        .arg("unknown")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"));
}
