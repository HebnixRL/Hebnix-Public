//! Compatibility checks for saved startup tabs and the new navigation layout.
use super::{ItemsMode, StudioSection, Tab};

#[test]
fn saved_tabs_survive_the_studio_reorganisation() {
    for key in ["Patcher", "UPK Studio", "Colours"] {
        assert_eq!(Tab::from_config_key(key), Tab::Patcher);
    }
    assert_eq!(Tab::from_config_key("Items"), Tab::Items);
    for key in ["Spoofer", "Customisation", "Player"] {
        assert_eq!(Tab::from_config_key(key), Tab::Spoofer);
    }
    assert_eq!(Tab::from_config_key("Experimental"), Tab::Replays);
    assert_eq!(Tab::from_config_key("Workshop"), Tab::Workshop);
    for tab in Tab::ALL {
        assert_eq!(Tab::from_config_key(tab.config_key()), tab);
    }
    assert_eq!(Tab::ALL.len(), 10);
    assert_eq!(Tab::ALL[0], Tab::Home);
    assert_eq!(Tab::ALL[1], Tab::Workshop);
    assert_eq!(Tab::ALL[2], Tab::Items);
    assert_eq!(Tab::ALL[3], Tab::Patcher);
    let settings = Tab::ALL.iter().position(|tab| *tab == Tab::Settings).unwrap();
    assert_eq!(Tab::ALL[settings + 1], Tab::Console);
    assert_eq!(Tab::from_config_key("unknown"), Tab::Home);
}

#[test]
fn game_file_tools_share_the_studio_sections() {
    assert_eq!(ItemsMode::Recolour.section(), None);
    assert_eq!(ItemsMode::ItemsActive.section(), None);
    assert_eq!(
        ItemsMode::ItemNames.section(),
        Some(StudioSection::CustomAssets)
    );
    assert_eq!(ItemsMode::SpeedPatch.section(), Some(StudioSection::Tools));
    assert_eq!(
        ItemsMode::GameColours.section(),
        Some(StudioSection::Appearance)
    );
    assert_eq!(
        ItemsMode::ArenaBackgrounds.section(),
        Some(StudioSection::Appearance)
    );
    assert_eq!(
        ItemsMode::MenuBackground.section(),
        Some(StudioSection::Menus)
    );
    assert_eq!(ItemsMode::Manage.section(), Some(StudioSection::Active));
}

#[test]
fn home_is_default_and_existing_settings_can_be_migrated_once() {
    let fresh = crate::config::SettingsCfg::default();
    assert_eq!(fresh.default_tab, "Home");
    assert!(fresh.home_default_migrated);
    let old: crate::config::SettingsCfg = toml::from_str("default_tab = 'Console'").unwrap();
    assert!(!old.home_default_migrated);
    let saved: crate::config::SettingsCfg =
        toml::from_str("default_tab = 'Maps'\nhome_default_migrated = true").unwrap();
    assert_eq!(saved.default_tab, "Maps");
    assert!(saved.home_default_migrated);
}
