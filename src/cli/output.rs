use crate::error::Result;
use crate::upgrade::template::Report;
use crate::upgrade::{UpgradeOutcome, UpgradeStage, UpgradeSummary};
use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::Duration;

const EXIT_CODE_CONFLICTS: u8 = 2;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";
const BOLD_RED: &str = "\x1b[1;31m";
const BOLD_GREEN: &str = "\x1b[1;32m";
const BOLD_YELLOW: &str = "\x1b[1;33m";
const BOLD_CYAN: &str = "\x1b[1;36m";
const BG_YELLOW_BLACK: &str = "\x1b[1;30;43m";

fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

fn colorize(text: &str, style: &str) -> String {
    if colors_enabled() {
        format!("{style}{text}{RESET}")
    } else {
        text.to_string()
    }
}

fn badge(label: &str, bg_style: &str) -> String {
    if colors_enabled() {
        format!("{bg_style} {label} {RESET}")
    } else {
        format!("[{label}]")
    }
}

pub fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs >= 60 {
        let mins = secs / 60;
        let rem_secs = secs % 60;
        format!("{mins}m {rem_secs:02}s")
    } else if secs >= 1 {
        format!("{:.2}s", duration.as_secs_f64())
    } else if duration.as_millis() > 0 {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{:.2}ms", duration.as_secs_f64() * 1000.0)
    }
}

pub fn finish(outcome: Result<UpgradeOutcome>, duration: Duration) -> ExitCode {
    let duration_str = format_duration(duration);
    match outcome {
        Ok(UpgradeOutcome::Completed {
            summary,
            dependency_output,
            dependency_files,
            git_branch,
        }) => {
            print!("{dependency_output}");
            for file in dependency_files {
                println!("{}: {file} changed", colorize("dependency", CYAN));
            }
            if let Some(branch) = git_branch {
                println!(
                    "{}: committed changes to branch {}",
                    colorize("git", BOLD_GREEN),
                    colorize(&branch, BOLD_CYAN)
                );
            }
            println!(
                "{}: {} in {}",
                colorize("Upgrade complete", BOLD_GREEN),
                summary.project.display(),
                colorize(&duration_str, BOLD_CYAN)
            );

            if !summary.report.coexistence_notices.is_empty() {
                println!();
                println!(
                    "{} {}",
                    badge("NEXT STEPS", BG_YELLOW_BLACK),
                    colorize("Action items for manual migration:", BOLD_YELLOW)
                );
                for (new_file, old_file) in &summary.report.coexistence_notices {
                    println!(
                        "  {} Migrate settings from {} -> {}",
                        colorize("*", BOLD_YELLOW),
                        colorize(old_file, YELLOW),
                        colorize(new_file, BOLD_GREEN)
                    );
                    println!(
                        "    Then delete {} before running flutter build/run",
                        colorize(old_file, BOLD_RED)
                    );
                }
            }

            ExitCode::SUCCESS
        }
        Ok(UpgradeOutcome::Conflicts(summary)) => {
            eprintln!(
                "{}: {} file conflict(s); original project was not changed in {}",
                colorize("flup", BOLD_RED),
                summary.report.conflicts.len(),
                colorize(&duration_str, DIM)
            );
            eprintln!(
                "  {} Run with {} to overwrite conflicting template files",
                colorize("hint:", CYAN),
                colorize("--force", BOLD_CYAN)
            );
            ExitCode::from(EXIT_CODE_CONFLICTS)
        }
        Err(message) => {
            eprintln!(
                "{}: {message} in {}",
                colorize("flup", BOLD_RED),
                colorize(&duration_str, DIM)
            );
            ExitCode::FAILURE
        }
    }
}

pub fn print_stage(stage: UpgradeStage) {
    let (icon, text) = match stage {
        UpgradeStage::Staging => ("[1/4]", "Staging complete project copy..."),
        UpgradeStage::GeneratingTemplates => ("[2/4]", "Generating Flutter templates..."),
        UpgradeStage::UpgradingDependencies => {
            ("[3/4]", "Upgrading dependencies in temporary copy...")
        }
        UpgradeStage::Committing => (
            "[4/4]",
            "Applying verified changes to the original project...",
        ),
    };
    println!("{} {}", colorize(icon, BOLD_CYAN), colorize(text, BOLD));
}

pub fn print_summary(summary: &UpgradeSummary) {
    println!(
        "{} {} -> {}",
        colorize("Flutter SDK:", BOLD),
        colorize(&summary.base_revision, CYAN),
        colorize(&summary.target_revision, BOLD_GREEN)
    );
    print_report(&summary.report);
}

fn print_report(report: &Report) {
    print_section("updated", &report.updated);
    print_section("merged", &report.merged);
    print_section("added", &report.added);
    print_section("removed", &report.removed);
    print_section("forced", &report.forced);
    print_section("removed upstream (kept)", &report.removed_upstream);

    for path in &report.conflicts {
        println!(
            "{}: {} {}",
            colorize("conflict", BOLD_RED),
            colorize(path, BOLD),
            colorize("(overlapping or unsafe template edits)", DIM)
        );

        if let Some(preview) = report.conflict_previews.get(path) {
            println!(
                "  {}",
                colorize("project changes from old template:", BOLD_YELLOW)
            );
            print_diff_block(&preview.project_changes);
            println!(
                "  {}",
                colorize("Flutter changes from old template:", BOLD_CYAN)
            );
            print_diff_block(&preview.flutter_changes);
        }
    }

    if !report.coexistence_notices.is_empty() {
        println!();
        println!(
            "{} {}",
            badge("NOTE", BG_YELLOW_BLACK),
            colorize(
                "Action required - manual configuration migration needed:",
                BOLD_YELLOW
            )
        );
        for (new_file, old_file) in &report.coexistence_notices {
            println!(
                "  {} {} was added alongside existing {}",
                colorize("*", BOLD_YELLOW),
                colorize(new_file, BOLD_GREEN),
                colorize(old_file, YELLOW)
            );
            println!(
                "    {} Compare configurations, migrate custom settings into {},",
                colorize("->", CYAN),
                colorize(new_file, BOLD_GREEN)
            );
            println!(
                "       then delete {} before building.",
                colorize(old_file, BOLD_RED)
            );
        }
        println!();
    }

    let mut counts = Vec::new();
    if !report.updated.is_empty() {
        counts.push(colorize(&format!("{} updated", report.updated.len()), CYAN));
    } else {
        counts.push(format!("{} updated", report.updated.len()));
    }

    if !report.merged.is_empty() {
        counts.push(colorize(&format!("{} merged", report.merged.len()), GREEN));
    } else {
        counts.push(format!("{} merged", report.merged.len()));
    }

    if !report.added.is_empty() {
        counts.push(colorize(
            &format!("{} added", report.added.len()),
            BOLD_GREEN,
        ));
    } else {
        counts.push(format!("{} added", report.added.len()));
    }

    if !report.removed.is_empty() {
        counts.push(colorize(
            &format!("{} removed", report.removed.len()),
            YELLOW,
        ));
    }

    if !report.forced.is_empty() {
        counts.push(colorize(
            &format!("{} forced", report.forced.len()),
            MAGENTA,
        ));
    }

    if !report.coexistence_notices.is_empty() {
        counts.push(colorize(
            &format!("{} migration note(s)", report.coexistence_notices.len()),
            BOLD_YELLOW,
        ));
    }

    if !report.conflicts.is_empty() {
        counts.push(colorize(
            &format!("{} conflict(s)", report.conflicts.len()),
            BOLD_RED,
        ));
    } else {
        counts.push(colorize("0 conflict(s)", BOLD_GREEN));
    }

    println!(
        "{}: {}",
        colorize("Template changes", BOLD),
        counts.join(", ")
    );
}

fn print_section(label: &str, paths: &[String]) {
    let style = match label {
        "updated" => CYAN,
        "merged" => GREEN,
        "added" => BOLD_GREEN,
        "removed" => YELLOW,
        "forced" => MAGENTA,
        "removed upstream (kept)" => DIM,
        _ => RESET,
    };

    for path in paths {
        println!("{}: {path}", colorize(label, style));
    }
}

fn print_diff_block(diff: &str) {
    for line in diff.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('+') {
            println!("{}", colorize(line, GREEN));
        } else if trimmed.starts_with('-') {
            println!("{}", colorize(line, RED));
        } else {
            println!("{}", colorize(line, DIM));
        }
    }
}
