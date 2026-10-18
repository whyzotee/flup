pub mod output;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::upgrade::UpgradeOptions;
use crate::upgrade::runtime::upgrade;

#[derive(Debug, Parser)]
#[command(name = "flup", version, about = "Safely upgrade Flutter app projects")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Upgrade the current Flutter app after staging a complete copy
    Upgrade {
        /// Project directory (defaults to the current directory)
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Target Flutter SDK directory (defaults to Flutter on PATH)
        #[arg(long)]
        flutter: Option<PathBuf>,
        /// Flutter SDK matching the project's .metadata revision
        #[arg(long)]
        base_flutter: Option<PathBuf>,
        /// Do not create or switch to a git branch, even if the project is in a Git repository
        #[arg(long)]
        no_git: bool,
        /// Force applying Flutter templates, overwriting conflicting files and migrating Groovy to Kotlin DSL
        #[arg(long)]
        force: bool,
    },
}

pub fn run() -> ExitCode {
    let Cli { command } = Cli::parse();
    let Commands::Upgrade {
        project,
        flutter,
        base_flutter,
        no_git,
        force,
    } = command;

    let options = UpgradeOptions {
        project,
        flutter,
        base_flutter,
        no_git,
        force,
    };

    let start_time = std::time::Instant::now();
    let progress = std::cell::RefCell::new(output::StageProgress::default());
    let result = upgrade(
        options,
        |stage| progress.borrow_mut().start(stage),
        |summary| {
            progress.borrow_mut().finish();
            output::print_summary(summary);
        },
    );
    progress.borrow_mut().finish();

    output::finish(result, start_time.elapsed())
}
