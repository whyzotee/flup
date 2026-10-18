use std::process::Command;

#[test]
fn stage_timer_finishes_cleanly_when_output_is_redirected() {
    const CHILD: &str = "FLUP_TEST_STAGE_TIMER";
    if std::env::var_os(CHILD).is_some() {
        use flup::cli::output::StageProgress;
        use flup::upgrade::UpgradeStage;
        let mut progress = StageProgress::default();
        progress.start(UpgradeStage::GeneratingTemplates);
        progress.start(UpgradeStage::SubstepStarted {
            id: "base-generate",
            label: "Generating base template...",
        });
        progress.start(UpgradeStage::SubstepStarted {
            id: "target-generate",
            label: "Generating target template...",
        });
        progress.start(UpgradeStage::SubstepFinished {
            id: "target-generate",
            elapsed: std::time::Duration::from_millis(120),
            failed: false,
        });
        std::thread::sleep(std::time::Duration::from_millis(350));
        progress.start(UpgradeStage::SubstepFinished {
            id: "base-generate",
            elapsed: std::time::Duration::from_millis(1500),
            failed: true,
        });
        progress.start(UpgradeStage::Detail {
            message: "Base template: cache hit".into(),
        });
        progress.finish();
        progress.finish();
        drop(progress);
        println!("timer stopped");
        return;
    }

    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "stage_timer_finishes_cleanly_when_output_is_redirected",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains('\u{1b}'));
    assert!(!stdout.contains('\r'));
    assert!(stdout.contains("Base template: cache hit\n"));
    assert_eq!(
        stdout
            .matches("Generating target template... (120ms)")
            .count(),
        1
    );
    assert_eq!(
        stdout
            .matches("Generating base template... failed (1.50s)")
            .count(),
        1
    );
    assert!(
        stdout.find("Generating target template...").unwrap()
            < stdout.find("Generating base template...").unwrap()
    );
    assert!(stdout.contains("s)\ntimer stopped\n"));
    assert_eq!(
        stdout.matches("Generating Flutter templates... (").count(),
        1
    );
}

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

#[test]
fn format_duration_formats_various_ranges() {
    use flup::cli::output::format_duration;
    use std::time::Duration;

    assert_eq!(format_duration(Duration::from_micros(250)), "0.25ms");
    assert_eq!(format_duration(Duration::from_millis(450)), "450ms");
    assert_eq!(format_duration(Duration::from_millis(1500)), "1.50s");
    assert_eq!(format_duration(Duration::from_secs(12)), "12.00s");
    assert_eq!(
        format_duration(Duration::from_secs(65) + Duration::from_millis(400)),
        "1m 05s"
    );
}

#[test]
fn upgrade_reports_elapsed_duration_on_failure() {
    let output = Command::new(env!("CARGO_BIN_EXE_flup"))
        .args(["upgrade", "--project", "non-existent-dir-for-test"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot open project"));
    assert!(stderr.contains(" in "));
}
