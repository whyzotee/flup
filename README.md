# Flup - Safe in-place Flutter project upgrade tool.

Upgrade a Flutter app in place after making a complete temporary copy.

```sh
cargo install --path .
cd path/to/flutter_app
flup upgrade
```

`flup upgrade` uses the Flutter SDK on `PATH` as the target. It searches locally for the SDK revision recorded in `.metadata`. If no matching SDK is installed but that revision exists in the target SDK's local Git history, flup checks it out into the temporary workspace automatically. If needed, specify `--base-flutter <SDK directory>`. Use `--project <directory>` to upgrade a project outside the current directory, `--flutter <SDK directory>` to select the target SDK, `--no-git` to modify files in place without creating a migration branch or commit, or `--force` to overwrite conflicting files with the target Flutter template.

The command upgrades generated platform files when they still match their old Flutter template and merges independent text edits. Conflicts show a short preview of project and Flutter changes relative to the old template. When Flutter introduces new files or formats (such as Kotlin DSL `.kts` files alongside existing Groovy `.gradle` files), flup adds the new template files alongside your existing files so you can easily compare configurations before cleaning up obsolete files. Any unresolved conflict or dependency failure cancels the upgrade unless `--force` is used. Review the reported changes and test the app afterward.

Exit status `0` means the upgrade completed, `2` means template conflicts require review, and `1` means another error occurred. When run in a Git repository, flup creates and checks out a new `migrate-to-flutter-<version>` branch and commits the upgraded files (unless `--no-git` is passed). If writing changes back fails, flup attempts to restore the touched files and retains its temporary backup path for recovery.

Currently supported: Flutter `app` projects with a single known base revision that is available from an installed SDK or the target SDK's local Git history. Path dependencies and pub workspaces are not supported yet.

## Features & Roadmap

- [x] In-place Flutter app upgrade with temporary workspace staging
- [x] Automatic base SDK discovery matching `.metadata` revision
- [x] 3-way template comparison and conflict detection
- [x] Isolated dependency resolution via `flutter pub upgrade --major-versions`
- [x] Atomic commit with automatic rollback on failure
- [x] Automatic migration branch creation (`migrate-to-flutter-<version>`) with Git commit
- [x] Force mode (`--force`) to overwrite template conflicts and migrate Groovy to Kotlin DSL
- [ ] Support projects with multiple platform base revisions
- [ ] Support path dependencies and pub workspaces in the temporary copy
- [ ] Add a real SDK end-to-end fixture and structured change report
