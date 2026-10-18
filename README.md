# Flup a safe in-place Flutter project upgrade tool.

Upgrade a Flutter app in place after making a complete temporary copy.

### Requirements

- [Flutter](https://docs.flutter.dev/get-started/install) (lastest) and [Git](https://git-scm.com/downloads) must be installed and available on `PATH`.

## Installation

### macOS and Linux

```sh
curl -fsSL https://github.com/whyzotee/flup/releases/latest/download/flup-installer.sh | sh
```

### Windows (PowerShell)

```powershell
irm https://github.com/whyzotee/flup/releases/latest/download/flup-installer.ps1 | iex
```

Reopen your terminal after installation so it picks up any `PATH` changes, then run `flup --version`.

## Usage

Start from your Flutter app's root directory, where `pubspec.yaml` and `.metadata` are located. Save your work in Git before upgrading so you can review the changes afterward.

Check that the Flutter SDK on `PATH` is the version you want to upgrade to:

```sh
cd path/to/flutter_app
flutter --version
flup upgrade
```

flup uses that SDK as the target; it does not download the latest Flutter release or switch your installed SDK to another revision. It finds the app's old SDK revision from `.metadata`, then looks for a matching local SDK or recreates it from the target SDK's local Git history.

During the upgrade, flup:

1. Creates a complete backup and working copy in a temporary directory beside the app.
2. Compares the old and target Flutter templates, preserves customizations, and merges independent edits.
3. Runs `flutter pub upgrade --major-versions` in the working copy. This can change dependency constraints in `pubspec.yaml` and package versions in `pubspec.lock`.
4. Applies the selected template files, `.metadata`, and dependency files to your app.

Each step shows elapsed time; template details also report cache hits/misses and generation times.

### `--project <directory>`

Select the Flutter app to upgrade. Defaults to the current directory. Use this option to upgrade another project without changing directories:

```sh
flup upgrade --project "path/to/flutter_app"
```

### `--flutter <SDK directory>`

Select the installed Flutter SDK to upgrade the app to. Defaults to the SDK on `PATH`. Point to the SDK root containing `bin` and `packages`:

```sh
flup upgrade --flutter "path/to/target/flutter"
```

### `--base-flutter <SDK directory>`

Select the Flutter SDK used as the app's old template baseline. Its Git revision must match the revision in the app's `.metadata` exactly. Use this option if automatic discovery cannot find the old SDK:

```sh
flup upgrade --flutter "path/to/target/flutter" --base-flutter "path/to/old/flutter"
```

The path should point to the old SDK root containing `bin` and `packages`. These options can be combined with `--project`.

### `--force`

If both your app and Flutter changed the same part of a template file, flup shows the conflicting paths and a preview of each side's edits, then stops before applying changes to your app. Save the preview and use it to decide how to migrate your custom settings.

If you choose to replace conflicting files with the target templates, run:

```sh
flup upgrade --force
```

`--force` overwrites conflicting template files, including custom settings inside those files. It does not bypass dependency failures or project validation. When rerunning, include any `--project`, `--flutter`, and `--base-flutter` options you used originally.

When a dependency upgrade fails, fix the reported dependency or environment problem and rerun; the dependency command runs against the temporary copy. If applying files back fails, flup attempts to restore touched files and prints the retained backup location for recovery.

### `--no-git`

For a Git repository, flup creates or checks out `migrate-to-flutter-<version>` before applying files. The current implementation leaves changes uncommitted; review and commit them yourself. To skip branch creation and switching, use:

```sh
flup upgrade --no-git
```

`--no-git` still applies changes to your app.

### `--help`

Show all upgrade options:

```sh
flup upgrade --help
```

### After upgrading

Follow the notes printed by flup after the upgrade and complete the listed changes before running or building the app.

Use the target Flutter SDK to analyze, test, and build the app. For example, when the target SDK is on `PATH`:

```sh
git diff
flutter analyze
flutter test
flutter build <platform> # Test building the app after upgrading
```

### Exit codes

| Exit code | Meaning |
| --- | --- |
| `0` | Upgrade completed; review and test the app. |
| `1` | An error occurred; read the error and any recovery instructions. |
| `2` | Template conflicts require review; no upgrade files were applied. |

Supported projects are Flutter **apps** with a single known base revision available from an installed SDK or the target SDK's local Git history. Path dependencies, pub workspaces, and divergent platform base revisions are not supported.

## Tested upgrades

| From Flutter | To Flutter | Platform | Result |
| --- | --- | --- | --- |
| 3.16.9 | 3.47 | Android | Upgraded app works |
| 3.16.9 | 3.47 | iOS | Not tested yet |

Results are from one app; other projects may need additional migration work.

## Features & Roadmap

- [x] In-place Flutter app upgrade with temporary workspace staging
- [x] Automatic base SDK discovery matching `.metadata` revision
- [x] 3-way template comparison and conflict detection
- [x] Isolated dependency resolution via `flutter pub upgrade --major-versions`
- [x] Atomic commit with automatic rollback on failure
- [x] Automatic migration branch creation (`migrate-to-flutter-<version>`)
- [x] Force mode (`--force`) to overwrite template conflicts and migrate Groovy to Kotlin DSL
- [ ] Support projects with multiple platform base revisions
- [ ] Support path dependencies and pub workspaces in the temporary copy
- [ ] Add a real SDK end-to-end fixture and structured change report
