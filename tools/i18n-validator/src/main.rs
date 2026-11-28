use fluent_syntax::{ast::Entry, parser};
use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

fn parse_message_ids(source: &str) -> Result<BTreeSet<String>, Vec<String>> {
    let resource = parser::parse(source).map_err(|(_resource, errors)| {
        errors
            .into_iter()
            .map(|error| format!("{error:?}"))
            .collect::<Vec<String>>()
    })?;
    let mut ids = BTreeSet::new();
    for entry in &resource.body {
        if let Entry::Message(message) = entry {
            if !ids.insert(message.id.name.to_string()) {
                return Err(vec![format!("duplicate message ID '{}'", message.id.name)]);
            }
        }
    }
    Ok(ids)
}

fn read_locale(directory: &Path) -> Result<BTreeSet<String>, String> {
    let mut files = fs::read_dir(directory)
        .map_err(|error| format!("Cannot read {}: {error}", directory.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    files.retain(|path| path.extension().and_then(|ext| ext.to_str()) == Some("ftl"));
    files.sort();
    if files.is_empty() {
        return Err(format!(
            "Configured locale '{}' has no .ftl files.",
            directory.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    let mut ids = BTreeSet::new();
    for file in files {
        let text = fs::read_to_string(&file)
            .map_err(|error| format!("Cannot read {}: {error}", file.display()))?;
        let parsed = parse_message_ids(&text)
            .map_err(|errors| format!("{}: {}", file.display(), errors.join("; ")))?;
        for id in parsed {
            if !ids.insert(id.clone()) {
                return Err(format!("{}: duplicate message ID '{id}'", file.display()));
            }
        }
    }
    Ok(ids)
}

fn parse_locale_codes(source: &str) -> Result<BTreeSet<String>, String> {
    let config: toml::Value =
        toml::from_str(source).map_err(|error| format!("Invalid locales.toml: {error}"))?;

    let locales = config
        .get("locale")
        .and_then(toml::Value::as_array)
        .ok_or("locales.toml must contain [[locale]] entries")?;

    let mut codes = BTreeSet::new();

    for (index, locale) in locales.iter().enumerate() {
        let code = locale
            .get("code")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("Locale entry {} is missing a string 'code'", index + 1))?;

        if code.trim().is_empty() {
            return Err(format!("Locale entry {} has an empty code", index + 1));
        }

        if !codes.insert(code.to_string()) {
            return Err(format!("Duplicate locale code: {code}"));
        }
    }

    if !codes.contains("en") {
        return Err("locales.toml must include the English locale 'en'".to_string());
    }

    Ok(codes)
}

fn compare_ids(
    english_ids: &BTreeSet<String>,
    translated_ids: &BTreeSet<String>,
) -> (Vec<String>, Vec<String>) {
    let unknown = translated_ids.difference(english_ids).cloned().collect();

    let missing = english_ids.difference(translated_ids).cloned().collect();

    (unknown, missing)
}

fn main() -> ExitCode {
    let root = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let locales = root.join("hebnix_rs/crates/hebnix-app/locales");
    let english = locales.join("en/hebnix.ftl");
    let locale_config = locales.join("locales.toml");

    let config_source = match fs::read_to_string(&locale_config) {
        Ok(source) => source,
        Err(error) => {
            eprintln!(
                "ERROR: Could not read locale configuration {}: {error}",
                locale_config.display()
            );
            return ExitCode::FAILURE;
        }
    };

    let configured_locales = match parse_locale_codes(&config_source) {
        Ok(codes) => codes,
        Err(error) => {
            eprintln!("ERROR: {error}");
            return ExitCode::FAILURE;
        }
    };

    println!("Hebnix standalone i18n validator");
    println!("Checking: {}", locales.display());

    if !english.is_file() {
        eprintln!(
            "ERROR: English source file not found: {}",
            english.display()
        );
        eprintln!("Pass the repository root as the first argument.");
        return ExitCode::FAILURE;
    }

    let english_ids = match read_locale(&locales.join("en")) {
        Ok(ids) => ids,
        Err(error) => {
            eprintln!("ERROR: English Fluent syntax: {error}");
            return ExitCode::FAILURE;
        }
    };

    println!("English source parsed successfully.");
    println!("Found {} English message IDs.", english_ids.len());

    let mut failed = false;

    let actual_locales = match fs::read_dir(&locales) {
        Ok(entries) => {
            let mut codes = BTreeSet::new();

            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        eprintln!("ERROR: Cannot read a locale directory entry: {error}");
                        failed = true;
                        continue;
                    }
                };

                if !entry.path().is_dir() {
                    continue;
                }

                if let Some(code) = entry.file_name().to_str() {
                    codes.insert(code.to_string());
                }
            }

            codes
        }
        Err(error) => {
            eprintln!("ERROR: Could not read locale folders: {error}");
            return ExitCode::FAILURE;
        }
    };

    for code in configured_locales
        .iter()
        .filter(|code| code.as_str() != "en")
    {
        let directory = locales.join(code);

        if !directory.is_dir() {
            eprintln!("ERROR: Configured locale '{code}' has no directory.");
            failed = true;
            continue;
        }

        let translated_ids = match read_locale(&directory) {
            Ok(ids) => ids,
            Err(error) => {
                eprintln!("ERROR: {error}");
                failed = true;
                continue;
            }
        };
        let (unknown, missing) = compare_ids(&english_ids, &translated_ids);
        for id in unknown {
            eprintln!("ERROR: [{code}] '{id}' is not defined in English.");
            failed = true;
        }
        for id in missing {
            println!("WARN: [{code}] missing '{id}' (English fallback may be used).");
        }
        println!("Parsed {}", directory.display());
    }

    for code in actual_locales.difference(&configured_locales) {
        eprintln!("ERROR: Locale folder '{code}' is not configured in locales.toml.");
        failed = true;
    }

    if failed {
        eprintln!("Validation failed.");
        ExitCode::FAILURE
    } else {
        println!("Syntax and message-ID checks completed.");
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_fluent_messages() {
        let source = "hello = Hello\nwelcome = Welcome, { $name }!\n";

        let ids = parse_message_ids(source).expect("valid Fluent should parse");

        assert!(ids.contains("hello"));
        assert!(ids.contains("welcome"));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn rejects_malformed_fluent_syntax() {
        let source = "hello = { $name\n";

        assert!(parse_message_ids(source).is_err());
    }

    #[test]
    fn detects_unknown_message_ids() {
        let english = BTreeSet::from(["hello".to_string(), "welcome".to_string()]);
        let translated = BTreeSet::from(["hello".to_string(), "obsolete-key".to_string()]);

        let (unknown, missing) = compare_ids(&english, &translated);

        assert_eq!(unknown, vec!["obsolete-key"]);
        assert_eq!(missing, vec!["welcome"]);
    }

    #[test]
    fn detects_missing_translations_without_marking_them_as_unknown() {
        let english = BTreeSet::from(["hello".to_string(), "goodbye".to_string()]);
        let translated = BTreeSet::from(["hello".to_string()]);

        let (unknown, missing) = compare_ids(&english, &translated);

        assert!(unknown.is_empty());
        assert_eq!(missing, vec!["goodbye"]);
    }
    #[test]
    fn parses_configured_locale_codes() {
        let source = r#"
            [[locale]]
            code = "en"
            name = "English"

            [[locale]]
            code = "de"
            name = "Deutsch"
        "#;

        let codes = parse_locale_codes(source).expect("valid locale config should parse");

        assert!(codes.contains("en"));
        assert!(codes.contains("de"));
        assert_eq!(codes.len(), 2);
    }

    #[test]
    fn rejects_locale_config_without_english() {
        let source = r#"
            [[locale]]
            code = "de"
            name = "Deutsch"
        "#;

        assert!(parse_locale_codes(source).is_err());
    }

    #[test]
    fn rejects_duplicate_locale_codes() {
        let source = r#"
            [[locale]]
            code = "en"

            [[locale]]
            code = "en"
        "#;

        assert!(parse_locale_codes(source).is_err());
    }
}
