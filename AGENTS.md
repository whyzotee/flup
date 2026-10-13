# Flup development guide

## Purpose

`flup upgrade` upgrades an existing Flutter **app** from the current directory. The original project must remain usable if discovery, template comparison, dependency resolution, or file commit fails.

## Upgrade rules

- Use the target Flutter SDK from `PATH` by default. Find a locally installed base SDK whose Git revision matches the project's `.metadata`, accept `--base-flutter`, or create a temporary base SDK checkout from the target SDK's local Git history. Never switch the user's target SDK checkout.
- Make a complete backup and a working copy in a temporary sibling directory. Run `flutter create --no-pub` for both SDK versions and `flutter pub upgrade --major-versions` only against temporary paths.
- Compare generated files in supported platform directories, `.gitignore`, and `analysis_options.yaml` against the user's files. Apply a new template file when the user's version equals the old template; merge independent line edits when the result is unambiguous. Keep overlapping or unsafe edits as conflicts, with bounded previews of both sides. When a new template file introduces a new format or extension (such as Kotlin DSL alongside Groovy Gradle), add the new template file alongside the user's existing file so the user can compare and migrate custom configurations. When `--force` is passed, overwrite conflicting template files with target templates.
- Do not remove files that disappeared from Flutter's new template automatically. Keep user code, assets, and other files outside template scope.
- Commit only reported template changes, `.metadata`, `pubspec.yaml`, and `pubspec.lock`. When the project is a Git repository, create and commit these changes on a separate branch named `migrate-to-flutter-<version>`, unless `--no-git` is passed. Restore touched files from the backup if commit fails. Keep the temporary backup on any commit error for inspection or recovery.
- Reject project shapes that cannot be reproduced safely, including base revisions missing from both installed SDKs and local Git history, divergent platform base revisions, path dependencies, and pub workspaces.

## Multi-agent work

- When subagents are available, delegate independent exploration, review, or implementation tasks that can run in parallel. Keep short tasks and dependent steps with the main agent.
- Give each subagent a clear scope, expected result, and file ownership before it edits code. Agents must not edit the same files concurrently; coordinate any shared-file changes through the main agent.
- The main agent reviews and integrates subagent results, resolves conflicts, and runs the final verification commands below against the combined changes.

## Verification

`src/cli/mod.rs` parses Clap arguments and starts the command. `src/cli/output.rs` owns terminal output and exit codes. `src/upgrade/mod.rs` orchestrates the upgrade through `UpgradeBackend` and `UpgradeWorkspace` and returns an outcome without printing progress or reports. `src/upgrade/runtime.rs` connects those contracts to concrete operations. Keep `src/main.rs` limited to CLI startup.

`src/upgrade/project.rs` owns project validation and metadata transformation; `src/storage/project_store.rs` reads and writes project files. `src/flutter/mod.rs` discovers SDKs; `src/flutter/process.rs` executes Git and Flutter; `src/upgrade/dependencies.rs` runs and inspects the dependency upgrade. `src/upgrade/template.rs` decides file changes without filesystem access; `src/upgrade/text_merge.rs` merges independent text edits and renders bounded diff previews; `src/storage/template_store.rs` reads templates and applies those decisions to the stage. `src/storage/transaction.rs` owns the workspace lifetime; `src/storage/tree_copy.rs` stages the full copy; `src/storage/commit.rs` applies selected files and rolls back on failure. Pass commit only relative paths; it must not depend on the template report type.

`src/error.rs` owns the typed `FlupError` and project `Result` alias. Preserve underlying I/O errors as sources and use specific error variants for project, SDK, template, dependency, and commit failures.

Keep test files in `tests/unit/` and executable CLI tests in `tests/cli.rs`.

Run `cargo fmt --check`, `cargo test --offline`, and `cargo clippy --offline --all-targets -- -D warnings`. Test a real Flutter app when both old and target SDKs are available. Verify that conflict and pub failures leave the original unchanged.
