# Hebnix Standalone i18n Validator

A small, standalone Rust tool for checking the Fluent (`.ftl`) locale files and locale configuration used by Hebnix.

## What it checks

- Merges every `en/*.ftl` file and uses their message IDs as the reference.
- Reads configured locale codes from `locales.toml`.
- Checks that each configured non-English locale has a directory and at least one `.ftl` file.
- Parses translated Fluent files and reports malformed Fluent syntax.
- Merges each translated locale before comparing it to English, so split files do not produce false missing-key warnings.
- Reports duplicate message IDs within a file or across locale files as errors.
- Reports translated message IDs that do not exist in English as errors.
- Reports English message IDs missing from a translation as warnings. The application may fall back to English for these messages.
- Reports locale directories that are not listed in `locales.toml`.
- Rejects invalid locale configuration, duplicate locale codes, and configurations without `en`.

## Requirements

- Rust toolchain with Cargo installed.
- Run commands from the repository root unless stated otherwise.

## Run the validator

From the root of the `Hebnix-Public` repository:

```powershell
cargo run --manifest-path .\tools\i18n-validator\Cargo.toml
```

To validate a repository at a different path:

```powershell
cargo run --manifest-path .\tools\i18n-validator\Cargo.toml -- "C:\path\to\Hebnix-Public"
```

## Run the checks

### Check formatting

```powershell
cargo fmt --manifest-path .\tools\i18n-validator\Cargo.toml -- --check
```

To apply formatting instead, omit `-- --check`.

### Check compilation

```powershell
cargo check --manifest-path .\tools\i18n-validator\Cargo.toml
```

### Run unit and integration tests

```powershell
cargo test --manifest-path .\tools\i18n-validator\Cargo.toml
```

### Validate the current locale files

```powershell
cargo run --manifest-path .\tools\i18n-validator\Cargo.toml
```

### Check Git whitespace and review changes

```powershell
git diff --check
git status --short
git diff
```

Note that `git diff` does not display untracked files. Use `git status --short` to identify them.

## Results and exit status

- **Errors** cause validation to fail with a non-zero exit status.
- **Missing translations** are warnings and do not, by themselves, fail validation. The application may use the English fallback for these messages.
- A successful run means the checks implemented by this tool passed. It does not establish that translations are natural, accurate, or contextually correct.

## Current scope and limitations

- English (`en`) is the reference locale.
- Locale metadata is read from `hebnix_rs/crates/hebnix-app/locales/locales.toml`.
- Locale files are read from `hebnix_rs/crates/hebnix-app/locales/<locale-code>/`.
- The validator checks Fluent syntax and message IDs. It does not evaluate translation quality, verify that translations preserve intended meaning, or guarantee that variables and formatting are semantically consistent across languages.
- Missing IDs are warnings, so incomplete translations can still pass validation.
- The validator is independent of the main Hebnix workspace so it can be run without building the application and its other assets.

## Example output

Output varies depending on the current locale files. A successful run may include:

```text
Hebnix standalone i18n validator
English source parsed successfully.
Found 816 English message IDs.
Syntax and message-ID checks completed.
```

Warnings about missing translated messages may appear between these lines.
