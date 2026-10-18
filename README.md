# Flup a safe in-place Flutter project upgrade tool.

Upgrade a Flutter app in place after making a temporary backup and working copy.

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

1. Creates a backup and working copy beside the app, skipping rebuildable output such as `build/`, `.dart_tool/`, Gradle cache/build directories, and platform `ephemeral/` directories. Original files are left in place.
2. Compares the old and target Flutter templates, preserves customizations, and merges independent edits.
3. Runs `flutter pub upgrade --major-versions` in the working copy. This can change dependency constraints in `pubspec.yaml` and package versions in `pubspec.lock`.
4. Applies the selected template files, `.metadata`, and dependency files to your app.

Each step shows elapsed time. Template work also shows live timers for SDK cloning, template generation or cache loading, and file comparison.

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

Select the old Flutter SDK if automatic discovery fails. Its Git revision must match the app's `.metadata`:

```sh
flup upgrade --flutter "path/to/target/flutter" --base-flutter "path/to/old/flutter"
```

### `--force`

Overwrite conflicting files with the target Flutter templates. Custom settings in those files will be replaced; dependency errors still stop the upgrade:

```sh
flup upgrade --force
```

### `--no-git`

Apply changes without creating or switching to a migration branch:

```sh
flup upgrade --no-git
```

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

## Tested upgrades

| From Flutter | To Flutter | Platform | Result |
| --- | --- | --- | --- |
| 3.16.9 | 3.47 | Android | Upgraded app works |
| 3.16.9 | 3.47 | iOS | Not tested yet |

| Cache | Approx. run time |
| --- | --- |
| Without cache | 1m 15s |
| With cache | 2.51s |

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
