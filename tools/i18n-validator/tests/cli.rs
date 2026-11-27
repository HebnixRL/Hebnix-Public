use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

struct TestRepo {
    _temp: TempDir,
    root: std::path::PathBuf,
    locales: std::path::PathBuf,
}

impl TestRepo {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let root = temp.path().to_path_buf();
        let locales = root.join("hebnix_rs/crates/hebnix-app/locales");

        fs::create_dir_all(locales.join("en")).unwrap();
        fs::write(locales.join("en/hebnix.ftl"), "hello = Hello\n").unwrap();
        fs::write(
            locales.join("locales.toml"),
            r#"
[[locale]]
code = "en"
name = "English"

[[locale]]
code = "de"
name = "Deutsch"
"#,
        )
        .unwrap();
        fs::create_dir_all(locales.join("de")).unwrap();
        fs::write(locales.join("de/hebnix.ftl"), "hello = Hallo\n").unwrap();

        Self {
            _temp: temp,
            root,
            locales,
        }
    }

    fn run(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_hebnix-i18n-validator"))
            .arg(&self.root)
            .output()
            .expect("run validator")
    }
}

#[test]
fn accepts_valid_repository() {
    let repo = TestRepo::new();
    let output = repo.run();

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn rejects_missing_english_source() {
    let repo = TestRepo::new();
    fs::remove_file(repo.locales.join("en/hebnix.ftl")).unwrap();

    let output = repo.run();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("English source file not found"));
}

#[test]
fn rejects_missing_configured_locale_directory() {
    let repo = TestRepo::new();
    fs::remove_dir_all(repo.locales.join("de")).unwrap();

    let output = repo.run();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Configured locale 'de' has no directory")
    );
}

#[test]
fn rejects_locale_without_fluent_files() {
    let repo = TestRepo::new();
    fs::remove_file(repo.locales.join("de/hebnix.ftl")).unwrap();

    let output = repo.run();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("Configured locale 'de' has no .ftl files"));
}

#[test]
fn rejects_unknown_message_ids() {
    let repo = TestRepo::new();
    fs::write(
        repo.locales.join("de/hebnix.ftl"),
        "hello = Hallo\nobsolete-key = Alt\n",
    )
    .unwrap();

    let output = repo.run();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("'obsolete-key' is not defined in English"));
}

#[test]
fn rejects_malformed_fluent_syntax() {
    let repo = TestRepo::new();
    fs::write(repo.locales.join("de/hebnix.ftl"), "hello = { $name\n").unwrap();

    let output = repo.run();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ERROR:"));
}

#[test]
fn rejects_unconfigured_locale_directory() {
    let repo = TestRepo::new();
    fs::create_dir_all(repo.locales.join("fr")).unwrap();

    let output = repo.run();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Locale folder 'fr' is not configured")
    );
}

#[test]
fn rejects_missing_locale_configuration() {
    let repo = TestRepo::new();
    fs::remove_file(repo.locales.join("locales.toml")).unwrap();

    let output = repo.run();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Could not read locale configuration"));
}

#[test]
fn combines_all_english_and_translated_files() {
    let repo = TestRepo::new();
    fs::write(repo.locales.join("en/navigation.ftl"), "nav-home = Home\n").unwrap();
    fs::write(
        repo.locales.join("de/navigation.ftl"),
        "nav-home = Startseite\n",
    )
    .unwrap();
    let output = repo.run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("missing"));
}

#[test]
fn checks_syntax_in_secondary_english_files() {
    let repo = TestRepo::new();
    fs::write(repo.locales.join("en/navigation.ftl"), "broken = { $name\n").unwrap();
    let output = repo.run();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("navigation.ftl"));
}

#[test]
fn rejects_duplicate_keys_across_locale_files() {
    let repo = TestRepo::new();
    fs::write(
        repo.locales.join("de/navigation.ftl"),
        "hello = Noch einmal\n",
    )
    .unwrap();
    let output = repo.run();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate message ID 'hello'"));
}

#[test]
fn rejects_duplicate_keys_in_one_file() {
    let repo = TestRepo::new();
    fs::write(
        repo.locales.join("en/hebnix.ftl"),
        "hello = Hello\nhello = Again\n",
    )
    .unwrap();
    let output = repo.run();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate message ID 'hello'"));
}
