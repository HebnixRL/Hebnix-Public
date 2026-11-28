use eframe::egui;
use std::borrow::Cow;
use wry::{
    Rect, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows,
    dpi::{PhysicalPosition, PhysicalSize},
    http::{Method, Response, StatusCode, header::CONTENT_TYPE},
};


fn rank_asset(path: &str) -> Option<(&'static str, Cow<'static, [u8]>)> {
    const PLAYLISTS: [&[u8]; 10] = [
        include_bytes!("../assets/ingame-rank/Playlists/0.png"),
        include_bytes!("../assets/ingame-rank/Playlists/1.png"),
        include_bytes!("../assets/ingame-rank/Playlists/2.png"),
        include_bytes!("../assets/ingame-rank/Playlists/3.png"),
        include_bytes!("../assets/ingame-rank/Playlists/4.png"),
        include_bytes!("../assets/ingame-rank/Playlists/5.png"),
        include_bytes!("../assets/ingame-rank/Playlists/6.png"),
        include_bytes!("../assets/ingame-rank/Playlists/7.png"),
        include_bytes!("../assets/ingame-rank/Playlists/8.png"),
        include_bytes!("../assets/ingame-rank/Playlists/9.png"),
    ];
    const TIERS: [&[u8]; 23] = [
        include_bytes!("../assets/ingame-rank/Tiers/0.png"),
        include_bytes!("../assets/ingame-rank/Tiers/1.png"),
        include_bytes!("../assets/ingame-rank/Tiers/2.png"),
        include_bytes!("../assets/ingame-rank/Tiers/3.png"),
        include_bytes!("../assets/ingame-rank/Tiers/4.png"),
        include_bytes!("../assets/ingame-rank/Tiers/5.png"),
        include_bytes!("../assets/ingame-rank/Tiers/6.png"),
        include_bytes!("../assets/ingame-rank/Tiers/7.png"),
        include_bytes!("../assets/ingame-rank/Tiers/8.png"),
        include_bytes!("../assets/ingame-rank/Tiers/9.png"),
        include_bytes!("../assets/ingame-rank/Tiers/10.png"),
        include_bytes!("../assets/ingame-rank/Tiers/11.png"),
        include_bytes!("../assets/ingame-rank/Tiers/12.png"),
        include_bytes!("../assets/ingame-rank/Tiers/13.png"),
        include_bytes!("../assets/ingame-rank/Tiers/14.png"),
        include_bytes!("../assets/ingame-rank/Tiers/15.png"),
        include_bytes!("../assets/ingame-rank/Tiers/16.png"),
        include_bytes!("../assets/ingame-rank/Tiers/17.png"),
        include_bytes!("../assets/ingame-rank/Tiers/18.png"),
        include_bytes!("../assets/ingame-rank/Tiers/19.png"),
        include_bytes!("../assets/ingame-rank/Tiers/20.png"),
        include_bytes!("../assets/ingame-rank/Tiers/21.png"),
        include_bytes!("../assets/ingame-rank/Tiers/22.png"),
    ];
    let name = path.strip_prefix("/rank-icons/")?.strip_suffix(".png")?;
    let (kind, index) = name.split_once('-')?;
    let index = index.parse::<usize>().ok()?;
    let bytes = match kind {
        "playlist" => PLAYLISTS.get(index),
        "tier" => TIERS.get(index),
        _ => None,
    }?;
    Some(("image/png", Cow::Borrowed(*bytes)))
}

pub(crate) const SIDEBAR_WIDTH: f32 = 216.0;

pub(crate) fn styled_controls(ui: &mut egui::Ui, render: impl FnOnce(&mut egui::Ui)) {
    use egui::{Color32, Stroke};
    ui.scope(|ui| {
        // the egui shell has no web page to match, so these controls follow the active theme
        if SEE_THROUGH_PAGES.load(std::sync::atomic::Ordering::Relaxed) {
            let spacing = &mut ui.style_mut().spacing;
            spacing.button_padding = egui::vec2(9.0, 5.0);
            spacing.interact_size.y = spacing.interact_size.y.max(26.0);
            render(ui);
            return;
        }
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(Color32::from_rgb(232, 240, 253));
        visuals.panel_fill = Color32::from_rgb(13, 23, 38);
        visuals.window_fill = Color32::from_rgb(16, 29, 48);
        visuals.extreme_bg_color = Color32::from_rgb(10, 18, 29);
        visuals.faint_bg_color = Color32::from_rgb(18, 31, 50);
        visuals.text_edit_bg_color = Some(visuals.extreme_bg_color);
        visuals.selection.bg_fill = Color32::from_rgb(35, 65, 106);
        visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(114, 200, 255));
        for (widget, fill) in [
            (&mut visuals.widgets.noninteractive, [16, 29, 48]),
            (&mut visuals.widgets.inactive, [23, 38, 59]),
            (&mut visuals.widgets.hovered, [32, 53, 81]),
            (&mut visuals.widgets.active, [38, 70, 109]),
            (&mut visuals.widgets.open, [27, 47, 73]),
        ] {
            widget.bg_fill = Color32::from_rgb(fill[0], fill[1], fill[2]);
            widget.weak_bg_fill = widget.bg_fill;
            widget.bg_stroke = Stroke::new(1.0, Color32::from_rgb(45, 67, 98));
            widget.corner_radius = egui::CornerRadius::same(5);
        }
        ui.style_mut().visuals = visuals;
        let spacing = &mut ui.style_mut().spacing;
        spacing.button_padding = egui::vec2(9.0, 5.0);
        spacing.interact_size.y = spacing.interact_size.y.max(26.0);
        render(ui);
    });
}
const HEADER_HEIGHT: f32 = 52.0;
const fn default_action_width() -> u16 {
    180
}
const SCHEME: &str = "hebnix-ui";
// pages are transparent in the egui shell so see-through themes reach them; the CSS supplies the opaque default
static SEE_THROUGH_PAGES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const PAGE: &str = "hebnix-ui://app/index.html";
const ABOUT_PAGE: &str = "hebnix-ui://app/about.html";
const REPLAY_PAGE: &str = "hebnix-ui://app/replay-upload.html";
const SWAPPER_PAGE: &str = "hebnix-ui://app/swapper.html";
const SWAPPER_URL: &str = "https://hebnix-ui.app/swapper.html";
const CONSOLE_PAGE: &str = "hebnix-ui://app/console.html";
const CONSOLE_URL: &str = "https://hebnix-ui.app/console.html";
const ABOUT_URL: &str = "https://hebnix-ui.app/about.html";
const REPLAY_URL: &str = "https://hebnix-ui.app/replay-upload.html";

const HEADER_PAGE: &str = "hebnix-ui://app/header.html";
const DIALOG_PAGE: &str = "hebnix-ui://app/dialog.html";
const TRACKER_PAGE: &str = "hebnix-ui://app/tracker.html";
const TRACKER_URL: &str = "https://hebnix-ui.app/tracker.html";
const MANAGE_URL: &str = "https://hebnix-ui.app/manage.html";
pub(crate) const PLUGIN_WINDOW_PAGE: &str = "hebnix-ui://app/manage.html";
const ARENA_URL: &str = "https://hebnix-ui.app/arena-editor.html";
const LIVE_MATCH_URL: &str = "https://hebnix-ui.app/live-match.html";
const BROWSE_MAPS_URL: &str = "https://hebnix-ui.app/browse-maps.html";

#[derive(serde::Serialize)]
pub(crate) struct TabLabels {
    pub(crate) label: String,
    pub(crate) children: Vec<String>,
    pub(crate) descendants: Vec<Vec<(usize, String)>>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum NavigationMessage {
    ContentPrepared { url: String, revision: u64 },
    Nested { tab: usize, child: usize, leaf: usize },
    Ready,
    AboutReady,
    HeaderReady,
    PanelBack,
    PanelSelect { index: usize },
    PanelAdvanced { enabled: bool },
    DialogReady,
    Dialog { id: String, key: String },
    DismissHeader,
    Header {
        action: HeaderAction,
    },
    TrackerReady,
    LiveMatchReady,
    BrowseMapsReady,
    ArenaReady,
    ManageReady,
    Manage {
        action: ManageAction,
    },
    Arena {
        action: crate::ui::workshop::background_changer::ArenaAction,
    },
    BrowseMaps {
        action: crate::ui::workshop::BrowseMapsAction,
    },
    Tracker {
        action: TrackerAction,
    },
    SwapperReady,
    Swapper {
        action: SwapperAction,
    },
    ConsoleReady,
    Console {
        action: ConsoleAction,
    },
    ReplayUploadReady,
    ReplayUpload {
        action: ReplayUploadAction,
    },
    Maintenance,
    AboutLink {
        link: AboutLink,
    },
    Select {
        tab: usize,
        child: Option<usize>,
    },
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AboutLink {
    Website,
    Hebbins,
    Nixvio64,
    Xplodingeggo,
    Rlyvision,
    Noxygalaxy,
    Foxynoxy,
}

impl AboutLink {
    pub(crate) fn url(self) -> &'static str {
        match self {
            Self::Website => "https://hebnix.com",
            Self::Hebbins => "https://github.com/Hebbins",
            Self::Nixvio64 => "https://github.com/nixvio64",
            Self::Xplodingeggo => "https://github.com/xplodingeggo",
            Self::Rlyvision => "https://github.com/rlyvision",
            Self::Noxygalaxy => "https://github.com/noxygalaxy",
            Self::Foxynoxy => "https://github.com/F0xyN0xy",
        }
    }
}

pub(crate) enum Action {
    Nested { tab: usize, child: usize, leaf: usize },
    PanelBack,
    PanelSelect { index: usize },
    PanelAdvanced { enabled: bool },
    Select { tab: usize, child: Option<usize> },
    Maintenance,
    AboutLink(AboutLink),
    Header(HeaderAction),
    Dialog { id: String, key: String },
    Tracker(TrackerAction),
    BrowseMaps(crate::ui::workshop::BrowseMapsAction),
    Arena(crate::ui::workshop::background_changer::ArenaAction),
    Manage(ManageAction),
    Swapper(SwapperAction),
    Console(ConsoleAction),
    ReplayUpload(ReplayUploadAction),
}

#[derive(serde::Serialize, PartialEq, Eq)]
pub(crate) struct About {
    pub(crate) version: String,
    pub(crate) maintenance: String,
}

#[derive(Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HeaderPanel {
    Settings,
    Actions,
    Notifications,
    Network,
}

impl HeaderPanel {
    fn height(self, action_count: usize) -> f32 {
        match self {
            Self::Actions => HEADER_HEIGHT + 14.0 + 36.0 * action_count as f32,
            Self::Notifications | Self::Network => 382.0,
            Self::Settings => 250.0,
        }
    }

    fn width(self, action_width: u16) -> f32 {
        match self {
            Self::Actions => f32::from(action_width),
            Self::Notifications => 360.0,
            Self::Network => 400.0,
            Self::Settings => 360.0,
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum HeaderAction {
    Panel {
        panel: Option<HeaderPanel>,
        #[serde(default)]
        action_inset: u16,
        #[serde(default = "default_action_width")]
        action_width: u16,
    },
    ActionAnchor {
        inset: u16,
        width: u16,
    },
    Run {
        action: crate::config::ActionButtonAction,
    },
    OwnedAccount { account: String },
    RefreshSave,
    OwnedOnly { enabled: bool },
    MoreSettings,
    ClearNotifications,
    Network {
        action: crate::app::network::WebAction,
    },
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ManageAction {
    MenuTextures { key: String, value: serde_json::Value },
    UpkHistory { key: String, index: Option<usize>, value: serde_json::Value },
    Anthem { key: String, id: Option<String>, value: serde_json::Value },
    ItemArtwork { key: String, value: serde_json::Value },
    StudioDecal { key: String, id: Option<String>, value: serde_json::Value },
    Maintenance { key: String, enabled: Option<bool> },
    StudioCustomPage { index: usize },
    StudioAsset { key: String, id: Option<String> },
    StudioAssetPreviews { ids: Vec<String> },
    GameColours { settings: Option<crate::patcher::colours::ColourSettings>, restore: bool },
    BallAppearance { key: String, value: serde_json::Value },
    WheelSelect { index: usize },
    WheelApply { settings: Option<crate::patcher::wheel_alignment::Settings>, remove: Option<String> },
    LiveCosmetics { settings: crate::patcher::live_cosmetics::Settings },
    MenuMap { action: crate::ui::workshop::WebMapAction },
    Multiplayer { action: crate::ui::workshop::MultiplayerAction },
    Titles { action: TitlesAction },
    PluginSettingsSelect { id: String },
    PluginSettingsInput { plugin: String, id: String, value: serde_json::Value },
    PluginSettingsExpand { plugin: String, id: String, open: bool },
    PluginSettingsInstalled,
    ReplayLibrary { action: crate::replay_timeline::WebAction },
    StudioRestore { kind: Option<String>, target: Option<String> },
    GameMod { id: String, key: String, value: serde_json::Value },
    AddMaps { action: crate::ui::workshop::AddMapsAction },
    Files { key: String, value: Option<String> },
    LaunchSetup { key: String, draft: Option<crate::config::RlLaunchCfg>, candidate: Option<usize> },
    Appearance { key: String, value: serde_json::Value },
    SystemToggle { key: String, enabled: bool },
    SystemFps { fps: u32 },
    SystemCommand { key: String },
    SystemBackups { key: String },
    SpeedItems { enabled: bool },
    SpeedChoose,
    SpeedRun { speed: f32, restore: bool },
    StudioAppearancePage { index: usize },
    GoalExplosion { volume: bool, value: f32, restore: bool },
    DiscordOption { key: String, enabled: bool },
    DiscordMessage { message: String },
    DiscordLink { code: String },
    Preset { action: PresetAction },
    WorkshopMenu { action: crate::ui::workshop::ingame_menu::WebMenuAction },
    ProfilePicture { key: String, value: serde_json::Value },
    PlayerRank { id: i32, enabled: Option<bool>, rank: Option<i32>, division: Option<u8>, mmr: Option<String> },
    PlayerRankApply { restore: bool },
    Loadouts { action: crate::ui::loadouts::web::WebAction },
    LoadoutsReloadCatalogs,
    FreeplayToggle { enabled: bool },
    FreeplayPick { index: usize, value: usize },
    FreeplayReset,
    PlayerName { name: String, commit: bool },
    PlayerNameEnabled { enabled: bool },
    FriendNamesEnabled { enabled: bool },
    FriendName { id: String, enabled: Option<bool>, name: Option<String> },
    RevertFriendNames,
    Scoreboard { key: String, value: serde_json::Value },
    ActionButtonVisibility { enabled: bool },
    ActionButtonToggle { running: bool, index: usize, enabled: bool },
    ActionButtonMove { running: bool, from: usize, to: usize },
    PluginPreviews { ids: Vec<String> },
    OverlayMove { id: String, step: i8 },
    PluginToggle {
        id: String,
        enabled: bool,
    },
    PluginSettings {
        id: String,
    },
    PluginDelete {
        id: String,
    },
    PluginInstall {
        id: String,
    },
    PluginCommand {
        key: String,
    },
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum TitlesAction {
    Reload,
    Account { id: Option<String> },
    Owned { enabled: bool },
    Copy { id: String },
    Text { text: String },
    Colour { glow: bool, rgb: [u8; 3] },
    Glow { enabled: bool },
    Target { id: Option<String> },
    Save,
    Edit { index: usize },
    Remove { index: usize },
    Apply,
    Restore { key: String },
    Admin,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum PresetAction {
    Capture { name: String, patches: bool },
    Refresh,
    ImportFile,
    Select { index: usize },
    Apply,
    Edit,
    Delete,
    Draft { name: String, patches: bool },
    RemoveSwap { index: usize },
    RemovePatch { kind: String, key: Option<String> },
    Save,
    Cancel,
    RemoveMissing,
    IgnoreMissing,
    ShowKey,
    ImportKey { key: String },
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum TrackerAction {
    Select { id: i64 },
    RetryAvatar,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SwapperAction {
    Category {
        index: usize,
    },
    Reload,
    Inventory {
        account: Option<String>,
    },
    Apply {
        swap: crate::swapper::WebSwap,
    },
    Restore {
        key: Option<String>,
    },
    Infinium {
        bakkes: bool,
    },
    Previews {
        category: Option<usize>,
        ids: Vec<String>,
    },
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ConsoleAction {
    Draft { value: String },
    HistoryDraft { value: String },
    Submit { value: String },
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ReplayUploadAction {
    Enable { value: bool },
    SaveLocally { value: bool },
    UploadEnabled { value: bool },
    Provider { value: String },
    ApiKey { value: String },
    NamingTemplate { value: String },
    Visibility { value: String },
    GroupId { value: String },
    DebugLogging { value: bool },
    Check,
    Open { guid: String },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentPage {
    About,
    Tracker,
    LiveMatch,
    BrowseMaps,
    Arena,
    Manage,
    Swapper,
    Console,
    ReplayUpload,
}

impl ContentPage {
    fn url(self) -> &'static str {
        match self {
            Self::About => ABOUT_URL,
            Self::Tracker => TRACKER_URL,
            Self::LiveMatch => LIVE_MATCH_URL,
            Self::BrowseMaps => BROWSE_MAPS_URL,
            Self::Arena => ARENA_URL,
            Self::Manage => MANAGE_URL,
            Self::Swapper => SWAPPER_URL,
            Self::Console => CONSOLE_URL,
            Self::ReplayUpload => REPLAY_URL,
        }
    }

    fn update_function(self) -> &'static str {
        match self {
            Self::About => "showAbout",
            Self::Tracker => "showTracker",
            Self::LiveMatch => "showLiveMatch",
            Self::BrowseMaps => "showBrowseMaps",
            Self::Arena => "showArenaEditor",
            Self::Manage => "showManager",
            Self::Swapper => "showSwapper",
            Self::Console => "showConsole",
            Self::ReplayUpload => "showReplayUpload",
        }
    }
}

struct ContentView {
    page: ContentPage,
    surface: WebSurface,
}

type TabSelection = (usize, Option<usize>, Option<usize>);

struct WebSurface {
    webview: WebView,
    ready: bool,
    shown: bool,
    bounds: Option<Rect>,
    parent_position: Option<egui::Pos2>,
    data: Option<String>,
    theme: Option<String>,
    wait_for_paint: bool,
    revision: u64,
    painted: u64,
}

pub(crate) struct Navigation {
    // None when egui draws the sidebar and header
    webview: Option<WebView>,
    pub(crate) egui_shell: bool,
    messages: crossbeam_channel::Receiver<String>,
    shown: bool,
    ready: bool,
    selected: Option<TabSelection>,
    content_selection: Option<TabSelection>,
    size: Option<PhysicalSize<u32>>,
    content: Option<ContentView>,
    cached_content: Vec<ContentView>,
    header: Option<WebSurface>,
    dialog: Option<WebSurface>,
    pub(crate) header_panel: Option<HeaderPanel>,
    pub(crate) action_inset: u16,
    pub(crate) action_width: u16,
    workspace: Option<egui::Rect>,
    header_region: Option<(i32, i32, i32, i32, i32)>,
    theme: String,
    sidebar_theme: String,
    // pages and the dialog are built on first use, each WebView2 costs ~90ms at startup
    parent: ParentWindow,
    ctx: egui::Context,
    sender: crossbeam_channel::Sender<String>,
    dialog_tried: bool,
}

/// the main window's HWND, kept so views can be built after startup
struct ParentWindow(std::num::NonZeroIsize);

impl raw_window_handle::HasWindowHandle for ParentWindow {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let handle = raw_window_handle::Win32WindowHandle::new(self.0);
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw_window_handle::RawWindowHandle::Win32(handle)) })
    }
}

impl Navigation {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, tabs: &[TabLabels], egui_shell: bool) -> wry::Result<Self> {
        SEE_THROUGH_PAGES.store(egui_shell, std::sync::atomic::Ordering::Relaxed);
        let labels = serde_json::to_string(tabs).expect("tab labels are serializable");
        let script =
            include_str!("navigation.js").replace("/*TABS*/[]", &labels.replace('<', "\\u003c"));
        let (sender, messages) = crossbeam_channel::unbounded();
        let parent = match raw_window_handle::HasWindowHandle::window_handle(cc)?.as_raw() {
            raw_window_handle::RawWindowHandle::Win32(handle) => ParentWindow(handle.hwnd),
            _ => return Err(raw_window_handle::HandleError::NotSupported.into()),
        };
        let ctx = cc.egui_ctx.clone();
        let (webview, content, header) = if egui_shell {
            (None, None, None)
        } else {
            let webview = Self::build_view(cc, &ctx, PAGE, script, sender.clone())?;
            let content =
                Self::optional_surface(cc, &ctx, ABOUT_PAGE, sender.clone()).map(|mut surface| { surface.wait_for_paint = true; ContentView {
                    page: ContentPage::About,
                    surface,
                }});
            let header = Self::optional_surface(cc, &ctx, HEADER_PAGE, sender.clone());
            (Some(webview), content, header)
        };
        Ok(Self {
            webview,
            egui_shell,
            messages,
            shown: false,
            ready: false,
            selected: None,
            content_selection: None,
            size: None,
            content,
            cached_content: Vec::new(),
            header,
            dialog: None,
            header_panel: None,
            action_inset: 0,
            action_width: default_action_width(),
            workspace: None,
            header_region: None,
            theme: "null".into(),
            sidebar_theme: String::new(),
            parent,
            ctx,
            sender,
            dialog_tried: false,
        })
    }

    fn optional_surface(
        parent: &impl raw_window_handle::HasWindowHandle,
        ctx: &egui::Context,
        page: &'static str,
        sender: crossbeam_channel::Sender<String>,
    ) -> Option<WebSurface> {
        match Self::build_view(parent, ctx, page, String::new(), sender) {
            Ok(webview) => Some(WebSurface {
                webview,
                ready: false,
                shown: false,
                bounds: None,
                parent_position: None,
                data: None,
                theme: None,
                wait_for_paint: false,
                revision: 0,
                painted: 0,
            }),
            Err(error) => {
                tracing::warn!(%error, page, "could not initialize the Modern UI web view");
                None
            }
        }
    }

    pub(crate) fn build_view(
        parent: &impl raw_window_handle::HasWindowHandle,
        ctx: &egui::Context,
        page: &'static str,
        script: String,
        sender: crossbeam_channel::Sender<String>,
    ) -> wry::Result<WebView> {
        let browser_page = page.replacen("hebnix-ui://", "https://hebnix-ui.", 1);
        let ctx = ctx.clone();
        WebViewBuilder::new()
            .with_visible(false)
            .with_background_color(if page == DIALOG_PAGE || SEE_THROUGH_PAGES.load(std::sync::atomic::Ordering::Relaxed) { (0, 0, 0, 0) } else { (16, 20, 29, 255) })
            .with_transparent(page == DIALOG_PAGE || SEE_THROUGH_PAGES.load(std::sync::atomic::Ordering::Relaxed))
            .with_hotkeys_zoom(false)
            .with_browser_accelerator_keys(false)
            .with_default_context_menus(false)
            .with_initialization_script(if matches!(page, HEADER_PAGE | DIALOG_PAGE) {
                ""
            } else {
                r#"document.addEventListener("pointerdown", () => window.ipc.postMessage('{"type":"dismiss_header"}'));"#
            })
            .with_initialization_script(include_str!("controller.js"))
            .with_https_scheme(true)
            .with_custom_protocol(SCHEME.into(), move |_, request| {
                let asset: Option<(&str, Cow<'static, [u8]>)> =
                    match (request.method(), request.uri().path()) {
                        (&Method::GET, "/index.html") => Some((
                            "text/html; charset=utf-8",
                            Cow::Borrowed(include_bytes!("index.html")),
                        )),
                        (&Method::GET, "/bootstrap.min.css") => Some((
                            "text/css; charset=utf-8",
                            Cow::Borrowed(include_bytes!("bootstrap.min.css")),
                        )),
                        (&Method::GET, "/styles.css") => Some((
                            "text/css; charset=utf-8",
                            Cow::Borrowed(include_bytes!("styles.css")),
                        )),
                        (&Method::GET, "/about.html") => Some((
                            "text/html; charset=utf-8",
                            Cow::Borrowed(include_bytes!("about.html")),
                        )),
                        (&Method::GET, "/about.js") => Some((
                            "text/javascript; charset=utf-8",
                            Cow::Borrowed(include_bytes!("about.js")),
                        )),
                        (&Method::GET, "/replay-upload.html") => Some((
                            "text/html; charset=utf-8",
                            Cow::Borrowed(include_bytes!("replay-upload.html")),
                        )),
                        (&Method::GET, "/replay-upload.js") => Some((
                            "text/javascript; charset=utf-8",
                            Cow::Borrowed(include_bytes!("replay-upload.js")),
                        )),
                        (&Method::GET, "/console.html") => Some((
                            "text/html; charset=utf-8",
                            Cow::Borrowed(include_bytes!("console.html")),
                        )),
                        (&Method::GET, "/console.js") => Some((
                            "text/javascript; charset=utf-8",
                            Cow::Borrowed(include_bytes!("console.js")),
                        )),
                        (&Method::GET, "/swapper.html") => Some((
                            "text/html; charset=utf-8",
                            Cow::Borrowed(include_bytes!("swapper.html")),
                        )),
                        (&Method::GET, "/swapper.js") => Some((
                            "text/javascript; charset=utf-8",
                            Cow::Borrowed(include_bytes!("swapper.js")),
                        )),
                        (&Method::GET, "/header.html") => Some((
                            "text/html; charset=utf-8",
                            Cow::Borrowed(include_bytes!("header.html")),
                        )),
                        (&Method::GET, "/header.js") => Some((
                            "text/javascript; charset=utf-8",
                            Cow::Borrowed(include_bytes!("header.js")),
                        )),
                        (&Method::GET, "/dialog.html") => Some((
                            "text/html; charset=utf-8", Cow::Borrowed(include_bytes!("dialog.html")),
                        )),
                        (&Method::GET, "/dialog.js") => Some((
                            "text/javascript; charset=utf-8", Cow::Borrowed(include_bytes!("dialog.js")),
                        )),
                        (&Method::GET, "/tracker.html") => Some((
                            "text/html; charset=utf-8",
                            Cow::Borrowed(include_bytes!("tracker.html")),
                        )),
                        (&Method::GET, "/tracker.js") => Some((
                            "text/javascript; charset=utf-8",
                            Cow::Borrowed(include_bytes!("tracker.js")),
                        )),
                        (&Method::GET, "/live-match.html") => Some((
                            "text/html; charset=utf-8",
                            Cow::Borrowed(include_bytes!("live-match.html")),
                        )),
                        (&Method::GET, "/live-match.js") => Some((
                            "text/javascript; charset=utf-8",
                            Cow::Borrowed(include_bytes!("live-match.js")),
                        )),
                        (&Method::GET, "/browse-maps.html") => Some((
                            "text/html; charset=utf-8",
                            Cow::Borrowed(include_bytes!("browse-maps.html")),
                        )),
                        (&Method::GET, "/browse-maps.js") => Some((
                            "text/javascript; charset=utf-8",
                            Cow::Borrowed(include_bytes!("browse-maps.js")),
                        )),
                        (&Method::GET, "/tracker-logo.png") => Some((
                            "image/png",
                            Cow::Borrowed(include_bytes!("../assets/hebnix.png")),
                        )),
                        (&Method::GET, "/network-globe.svg") => Some((
                            "image/svg+xml",
                            Cow::Borrowed(include_bytes!("../assets/network-globe.svg")),
                        )),
                        (&Method::GET, "/notification-bell.svg") => Some((
                            "image/svg+xml",
                            Cow::Borrowed(include_bytes!("../assets/notification-bell.svg")),
                        )),
                        (&Method::GET, "/arena-editor.html") => Some((
                            "text/html; charset=utf-8", Cow::Borrowed(include_bytes!("arena-editor.html")),
                        )),
                        (&Method::GET, "/arena-editor.js") => Some((
                            "text/javascript; charset=utf-8", Cow::Borrowed(include_bytes!("arena-editor.js")),
                        )),
                        (&Method::GET, "/manage.html") => Some((
                            "text/html; charset=utf-8", Cow::Borrowed(include_bytes!("manage.html")),
                        )),
                        (&Method::GET, "/patch-progress.js") => Some((
                            "text/javascript; charset=utf-8", Cow::Borrowed(include_bytes!("patch-progress.js")),
                        )),
                        (&Method::GET, "/manage.js") => Some((
                            "text/javascript; charset=utf-8", Cow::Borrowed(include_bytes!("manage.js")),
                        )),
                        (&Method::GET, "/navigation.js") => Some((
                            "text/javascript; charset=utf-8",
                            Cow::Owned(script.as_bytes().to_vec()),
                        )),
                        (&Method::GET, path) => rank_asset(path),
                        _ => None,
                    };
                match asset {
                    Some((mime, bytes)) => {
                        Response::builder().header(CONTENT_TYPE, mime).body(bytes)
                    }
                    None => Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(Cow::<[u8]>::Borrowed(&[])),
                }
                .expect("static asset response")
            })
            .with_navigation_handler(move |url| {
                if matches!(page, PAGE | HEADER_PAGE | DIALOG_PAGE) {
                    url == page || url == browser_page
                } else {
                    matches!(
                        url.as_str(),
                        ABOUT_PAGE
                            | ABOUT_URL
                            | REPLAY_PAGE
                            | REPLAY_URL
                            | CONSOLE_PAGE
                            | CONSOLE_URL
                            | SWAPPER_PAGE
                            | SWAPPER_URL
                            | TRACKER_PAGE
                            | TRACKER_URL
                            | LIVE_MATCH_URL
                            | ARENA_URL | MANAGE_URL
                            | PLUGIN_WINDOW_PAGE
                            | BROWSE_MAPS_URL
                    )
                }
            })
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_ipc_handler(move |message| {
                if sender.send(message.into_body()).is_ok() {
                    ctx.request_repaint();
                }
            })
            .with_url(page)
            .build_as_child(parent)
    }

    pub(crate) fn take_actions(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        while let Ok(message) = self.messages.try_recv() {
            match serde_json::from_str(&message) {
                Ok(NavigationMessage::Nested { tab, child, leaf }) if self.shown => actions.push(Action::Nested { tab, child, leaf }),
                Ok(NavigationMessage::PanelBack) => actions.push(Action::PanelBack),
                Ok(NavigationMessage::PanelSelect { index }) if self.content_visible(ContentPage::Manage) => actions.push(Action::PanelSelect { index }),
                Ok(NavigationMessage::PanelAdvanced { enabled }) if self.content_visible(ContentPage::Manage) => actions.push(Action::PanelAdvanced { enabled }),
                Ok(NavigationMessage::DialogReady) => {
                    if let Some(dialog) = &mut self.dialog { dialog.ready = true; }
                }
                Ok(NavigationMessage::Dialog { id, key }) if self.dialog.as_ref().is_some_and(|dialog| dialog.shown) => {
                    actions.push(Action::Dialog { id, key });
                }
                Ok(NavigationMessage::Ready) => {
                    if !self.ready {
                        tracing::info!("Modern UI web navigation loaded");
                    }
                    self.ready = true;
                    self.selected = None;
                }
                Ok(NavigationMessage::HeaderReady) => {
                    if let Some(header) = &mut self.header {
                        header.ready = true;
                        header.data = None;
                    }
                }
                Ok(NavigationMessage::DismissHeader) => self.header_panel = None,
                Ok(NavigationMessage::Header { action })
                    if self.header.as_ref().is_some_and(|header| header.shown) =>
                {
                    actions.push(Action::Header(action));
                }
                Ok(NavigationMessage::ContentPrepared { url, revision }) => {
                    for content in self.content.iter_mut().chain(self.cached_content.iter_mut()) {
                        if content.page.url() == url && content.surface.revision == revision { content.surface.painted = revision; }
                    }
                }
                Ok(NavigationMessage::TrackerReady) => self.content_ready(ContentPage::Tracker),
                Ok(NavigationMessage::LiveMatchReady) => self.content_ready(ContentPage::LiveMatch),
                Ok(NavigationMessage::ManageReady) => self.content_ready(ContentPage::Manage),
                Ok(NavigationMessage::Manage { action })
                    if self.content_visible(ContentPage::Manage) =>
                {
                    if let Some(content) = &mut self.content {
                        content.surface.data = None;
                    }
                    actions.push(Action::Manage(action));
                }
                Ok(NavigationMessage::ArenaReady) => self.content_ready(ContentPage::Arena),
                Ok(NavigationMessage::Arena { action })
                    if self.content_visible(ContentPage::Arena) =>
                {
                    actions.push(Action::Arena(action));
                }
                Ok(NavigationMessage::BrowseMapsReady) => {
                    self.content_ready(ContentPage::BrowseMaps)
                }
                Ok(NavigationMessage::BrowseMaps { action })
                    if self.content_visible(ContentPage::BrowseMaps) =>
                {
                    actions.push(Action::BrowseMaps(action));
                }
                Ok(NavigationMessage::Tracker { action })
                    if self.content_visible(ContentPage::Tracker) =>
                {
                    actions.push(Action::Tracker(action));
                }
                Ok(NavigationMessage::ConsoleReady) => self.content_ready(ContentPage::Console),
                Ok(NavigationMessage::Console { action })
                    if self.content_visible(ContentPage::Console) =>
                {
                    actions.push(Action::Console(action));
                }
                Ok(NavigationMessage::SwapperReady) => self.content_ready(ContentPage::Swapper),
                Ok(NavigationMessage::Swapper { action })
                    if self.content_visible(ContentPage::Swapper) =>
                {
                    actions.push(Action::Swapper(action));
                }
                Ok(NavigationMessage::AboutReady) => self.content_ready(ContentPage::About),
                Ok(NavigationMessage::ReplayUploadReady) => {
                    self.content_ready(ContentPage::ReplayUpload)
                }
                Ok(NavigationMessage::Select { tab, child }) if self.shown => {
                    actions.push(Action::Select { tab, child });
                }
                Ok(NavigationMessage::Maintenance) if self.content_visible(ContentPage::About) => {
                    actions.push(Action::Maintenance);
                }
                Ok(NavigationMessage::AboutLink { link })
                    if self.content_visible(ContentPage::About) =>
                {
                    actions.push(Action::AboutLink(link));
                }
                Ok(NavigationMessage::ReplayUpload { action })
                    if self.content_visible(ContentPage::ReplayUpload) =>
                {
                    if let Some(content) = &mut self.content {
                        content.surface.data = None;
                    }
                    actions.push(Action::ReplayUpload(action));
                }
                _ => {}
            }
        }
        actions
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        selected: TabSelection,
    ) -> wry::Result<()> {
        let Some(webview) = &self.webview else {
            return Ok(());
        };
        let scale = ctx.pixels_per_point();
        let height = ctx.input(|input| {
            input
                .viewport()
                .inner_rect
                .unwrap_or(input.viewport_rect())
                .height()
        });
        let size = PhysicalSize::new(
            (SIDEBAR_WIDTH * scale).round() as u32,
            (height * scale).round() as u32,
        );
        if self.size != Some(size) {
            webview.set_bounds(Rect {
                position: PhysicalPosition::new(0, 0).into(),
                size: size.into(),
            })?;
            self.size = Some(size);
        }
        if self.ready && self.selected != Some(selected) {
            let (tab, child, leaf) = selected;
            let child = child.map_or_else(|| "null".to_owned(), |index| index.to_string());
            let leaf = leaf.map_or_else(|| "null".to_owned(), |index| index.to_string());
            webview
                .evaluate_script(&format!("window.selectHebnixTab({tab}, {child}, {leaf})"))?;
            self.selected = Some(selected);
        }
        if self.ready && self.sidebar_theme != self.theme {
            webview.evaluate_script(&format!("window.applyHebnixTheme({})", self.theme))?;
            self.sidebar_theme.clone_from(&self.theme);
        }
        if !self.shown {
            webview.set_visible(true)?;
            self.shown = true;
        }
        Ok(())
    }

    fn content_ready(&mut self, page: ContentPage) {
        for content in self.content.iter_mut().chain(self.cached_content.iter_mut()).filter(|content| content.page == page) {
            content.surface.ready = true;
            content.surface.data = None;
            content.surface.theme = None;
        }
    }

    fn content_visible(&self, page: ContentPage) -> bool {
        self.content
            .as_ref()
            .is_some_and(|content| content.page == page && content.surface.shown)
    }

    pub(crate) fn keep_content(&mut self, page: Option<ContentPage>) {
        if page.is_none() {
            for content in self.content.iter_mut().chain(self.cached_content.iter_mut()) { content.surface.hide(); }
        }
    }

    pub(crate) fn show_header(
        &mut self,
        ui: &egui::Ui,
        data: &impl serde::Serialize,
        action_count: usize,
    ) -> wry::Result<Option<f32>> {
        let Some(header) = self.header.as_mut().filter(|header| header.ready) else {
            return Ok(None);
        };
        let hwnd = header.webview.hwnd();
        let scale = ui.ctx().pixels_per_point();
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
        let web_scale = dpi as f32 / 96.0;
        let height = self
            .header_panel
            .map_or(HEADER_HEIGHT, |panel| panel.height(action_count))
            * web_scale;
        let rect = egui::Rect::from_min_size(
            ui.available_rect_before_wrap().min,
            egui::vec2(ui.available_width(), height / scale),
        );
        header.present(ui.ctx(), rect, "showHeader", data)?;
        header.apply_theme(&self.theme)?;
        let width = (rect.width() * scale).round() as i32;
        let region = self.header_panel.map(|panel| {
            let panel_width = (panel.width(self.action_width) * web_scale).round() as i32;
            let right_inset = if panel == HeaderPanel::Actions {
                (f32::from(self.action_inset) * web_scale).round() as i32
            } else {
                0
            }
            .min((width - panel_width).max(0));
            let right = width - right_inset;
            (
                width,
                height.round() as i32,
                (HEADER_HEIGHT * web_scale).round() as i32,
                (right - panel_width).max(0),
                right,
            )
        });
        if self.header_region != region {
            clip_header_window(hwnd, region)?;
            self.header_region = region;
        }
        if region.is_some() {
            use windows::Win32::UI::WindowsAndMessaging::{
                HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
            };
            unsafe {
                SetWindowPos(
                    hwnd,
                    Some(HWND_TOP),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                )
            }
            .map_err(std::io::Error::other)?;
        }
        Ok(Some(HEADER_HEIGHT * web_scale / scale))
    }

    pub(crate) fn set_workspace(&mut self, rect: Option<egui::Rect>) {
        self.workspace = rect;
    }

    pub(crate) fn set_theme(&mut self, name: &str, ctx: &egui::Context) {
        self.apply_window_opacity(ctx);
        if name == "Dark" {
            self.theme = "null".into();
            return;
        }
        use serde_json::json;
        let style = ctx.global_style();
        let visuals = &style.visuals;
        let color = |value: egui::Color32| {
            let [r, g, b, a] = value.to_srgba_unmultiplied();
            if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("rgba({r},{g},{b},{:.3})", f32::from(a) / 255.0) }
        };
        let presentation = crate::theme::presentation(ctx);
        let values = json!({
            "mode": if ctx.theme() == egui::Theme::Light { "light" } else { "dark" },
            "background": color(visuals.panel_fill),
            "surface": color(visuals.window_fill),
            "input": color(visuals.text_edit_bg_color.unwrap_or(visuals.extreme_bg_color)),
            "text": color(visuals.text_color()),
            "muted": color(visuals.weak_text_color.unwrap_or_else(|| visuals.widgets.noninteractive.fg_stroke.color)),
            "border": color(visuals.window_stroke.color),
            "accent": color(visuals.hyperlink_color),
            "radius": visuals.window_corner_radius.nw,
            "padding": presentation.as_ref().map(|p| p.layout.content_padding),
            "navigation": presentation.as_ref().map(|p| (&p.components.navigation.surface, &p.components.navigation.selected, &p.components.navigation.text)),
        });
        self.theme = values.to_string();
    }

    fn apply_window_opacity(&self, ctx: &egui::Context) {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetAncestor, GA_ROOT, GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE,
            WS_EX_LAYERED, SetLayeredWindowAttributes, LWA_ALPHA,
        };
        use windows::Win32::Foundation::COLORREF;
        let opacity = ctx.data_mut(|data| data.get_temp::<f32>(egui::Id::new("hebnix.theme.opacity"))).unwrap_or(1.0);
        unsafe {
            let window = GetAncestor(windows::Win32::Foundation::HWND(self.parent.0.get() as *mut core::ffi::c_void), GA_ROOT);
            let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
            SetWindowLongPtrW(window, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as isize);
            if let Err(error) = SetLayeredWindowAttributes(window, COLORREF(0), (opacity.clamp(0.5, 1.0) * 255.0).round() as u8, LWA_ALPHA) {
                tracing::debug!(%error, "could not set Wry window opacity");
            }
        }
    }

    pub(crate) fn panel_input(&self, events: &[crate::panel::PadEvent]) {
        let surface = self.dialog.as_ref().filter(|dialog| dialog.shown)
            .or_else(|| self.content.as_ref().map(|view| &view.surface).filter(|surface| surface.shown));
        let Some(surface) = surface else { return };
        for event in events {
            use crate::panel::PadEvent::*;
            let key = match event {
                Up => "up", Down => "down", Left => "left", Right => "right", Accept => "accept",
                Back => "back", PrevListPage => "previous", NextListPage => "next",
                PrevPage | NextPage => continue,
            };
            if let Err(error) = surface.webview.evaluate_script(&format!("window.navigateController?.('{key}')")) {
                tracing::debug!(%error, "could not send controller input to Wry");
            }
        }
    }

    pub(crate) fn show_dialog(&mut self, ctx: &egui::Context, data: Option<&serde_json::Value>) -> wry::Result<()> {
        if data.is_some() && self.dialog.is_none() && !self.dialog_tried {
            self.dialog_tried = true;
            self.dialog = Self::optional_surface(&self.parent, &self.ctx, DIALOG_PAGE, self.sender.clone());
        }
        let Some(dialog) = &mut self.dialog else { return Ok(()) };
        let Some(data) = data else {
            if dialog.shown { dialog.webview.evaluate_script("window.clearDialog()")?; }
            dialog.hide();
            return Ok(());
        };
        if !dialog.ready { return Ok(()) }
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, ctx.viewport_rect().size());
        dialog.present(ctx, rect, "showDialog", data)?;
        dialog.apply_theme(&self.theme)?;
        use windows::Win32::UI::WindowsAndMessaging::{HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos};
        unsafe {
            SetWindowPos(dialog.webview.hwnd(), Some(HWND_TOP), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE)
        }.map_err(std::io::Error::other)?;
        Ok(())
    }

    pub(crate) fn show_content(
        &mut self,
        ui: &egui::Ui,
        page: ContentPage,
        data: &impl serde::Serialize,
    ) -> wry::Result<bool> {
        if self.content.is_none() {
            // the egui shell starts without a page view
            if let Some(mut surface) = Self::optional_surface(&self.parent, &self.ctx, page.url(), self.sender.clone()) {
                surface.wait_for_paint = true;
                self.content = Some(ContentView { page, surface });
            }
        }
        if self.content.as_ref().is_some_and(|content| content.page != page) {
            if let Some(index) = self.cached_content.iter().position(|content| content.page == page) {
                let content = self.content.as_mut().unwrap();
                content.surface.webview.evaluate_script("document.activeElement?.blur();document.documentElement.style.pointerEvents='none';")?;
                std::mem::swap(content, &mut self.cached_content[index]);
                content.surface.webview.evaluate_script("if(document.documentElement)document.documentElement.style.pointerEvents='auto';")?;
            } else if let Some(mut surface) = Self::optional_surface(&self.parent, &self.ctx, page.url(), self.sender.clone()) {
                // first visit: build the page now and keep the old one cached
                surface.wait_for_paint = true;
                let content = self.content.as_mut().unwrap();
                content.surface.webview.evaluate_script("document.activeElement?.blur();document.documentElement.style.pointerEvents='none';")?;
                let previous = std::mem::replace(content, ContentView { page, surface });
                self.cached_content.push(previous);
            }
        }
        let Some(content) = &mut self.content else {
            return Ok(false);
        };
        if content.page != page {
            content.surface.hide();
            content.surface.webview.load_url(page.url())?;
            content.page = page;
            content.surface.ready = false;
        }
        if !content.surface.ready {
            let rect = self
                .workspace
                .unwrap_or_else(|| ui.available_rect_before_wrap());
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(16, 20, 29));
            return Ok(true);
        }
        content.surface.apply_theme(&self.theme)?;
        let was_shown = content.surface.shown;
        content.surface.present(
            ui.ctx(),
            self.workspace
                .unwrap_or_else(|| ui.available_rect_before_wrap()),
            page.update_function(),
            data,
        )?;
        if content.surface.shown {
            if !was_shown || self.content_selection != self.selected {
                content.surface.webview.evaluate_script("window.animateHebnixTab()")?;
                self.content_selection = self.selected;
            }
            for previous in &mut self.cached_content { previous.surface.hide(); }
        }
        Ok(true)
    }

    pub(crate) fn hide_header(&mut self) {
        if let Some(header) = &mut self.header {
            header.hide();
        }
        self.header_panel = None;
    }

    pub(crate) fn hide(&mut self) {
        if let Some(dialog) = &mut self.dialog { dialog.hide(); }
        self.hide_shell();
    }

    pub(crate) fn hide_shell(&mut self) {
        self.workspace = None;
        self.keep_content(None);
        self.hide_header();
        self.hide_sidebar();
    }

    pub(crate) fn hide_sidebar(&mut self) {
        if self.shown {
            self.shown = false;
            if let Some(Err(error)) = self.webview.as_ref().map(|webview| webview.set_visible(false)) {
                tracing::warn!(%error, "could not hide Wry navigation");
            }
        }
    }
}

fn clip_header_window(
    hwnd: windows::Win32::Foundation::HWND,
    region: Option<(i32, i32, i32, i32, i32)>,
) -> std::io::Result<()> {
    use windows::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{CreatePolygonRgn, DeleteObject, HGDIOBJ, SetWindowRgn, WINDING},
    };
    // Clip the WebView's input region as well as its drawing.
    let handle = region.map(|(width, height, strip, left, right)| {
        let strip = strip.min(height);
        let points = [
            POINT { x: 0, y: 0 },
            POINT { x: width, y: 0 },
            POINT { x: width, y: strip },
            POINT { x: right, y: strip },
            POINT {
                x: right,
                y: height,
            },
            POINT { x: left, y: height },
            POINT { x: left, y: strip },
            POINT { x: 0, y: strip },
        ];
        unsafe { CreatePolygonRgn(&points, WINDING) }
    });
    if handle.is_some_and(|handle| handle.is_invalid()) {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { SetWindowRgn(hwnd, handle, true) } == 0 {
        let error = std::io::Error::last_os_error();
        if let Some(handle) = handle {
            // Windows takes ownership only after SetWindowRgn succeeds.
            let _ = unsafe { DeleteObject(HGDIOBJ(handle.0)) };
        }
        return Err(error);
    }
    Ok(())
}

impl WebSurface {
    fn apply_theme(&mut self, theme: &str) -> wry::Result<()> {
        if self.theme.as_deref() != Some(theme) {
            self.webview.evaluate_script(&format!("window.applyHebnixTheme({theme})"))?;
            self.theme = Some(theme.to_string());
        }
        Ok(())
    }

    fn present(
        &mut self,
        ctx: &egui::Context,
        rect: egui::Rect,
        update: &str,
        data: &impl serde::Serialize,
    ) -> wry::Result<()> {
        let position = ctx.input(|input| input.viewport().outer_rect.map(|rect| rect.min));
        if position != self.parent_position {
            self.parent_position = position;
            // WebView2 owns native popups; notify it when the host moves even
            // when the child view's bounds within the host did not change.
            if let Err(error) = unsafe { self.webview.controller().NotifyParentWindowPositionChanged() } {
                tracing::debug!(%error, "could not notify WebView2 of parent movement");
            }
        }
        let scale = ctx.pixels_per_point();
        let bounds = Rect {
            position: PhysicalPosition::new(
                (rect.min.x * scale).round() as i32,
                (rect.min.y * scale).round() as i32,
            )
            .into(),
            size: PhysicalSize::new(
                (rect.width() * scale).round() as u32,
                (rect.height() * scale).round() as u32,
            )
            .into(),
        };
        if self.bounds != Some(bounds) {
            self.webview.set_bounds(bounds)?;
            self.bounds = Some(bounds);
        }
        let json = serde_json::to_string(data).expect("content data is serializable");
        if self.data.as_ref() != Some(&json) {
            let mut script = format!("document.documentElement.style.pointerEvents='auto';window.{update}({json});");
            if self.wait_for_paint && !self.shown {
                self.revision = self.revision.wrapping_add(1);
                // Hidden WebView2 surfaces suspend animation frames. Flush DOM
                // layout before the handover without waiting on a hidden rAF.
                script.push_str(&format!("queueMicrotask(()=>{{document.body.getBoundingClientRect();window.ipc.postMessage(JSON.stringify({{type:'content_prepared',url:location.href,revision:{}}}));}});", self.revision));
            }
            self.webview.evaluate_script(&script)?;
            self.data = Some(json);
        }
        if !self.shown && (!self.wait_for_paint || self.painted == self.revision) {
            self.webview.set_visible(true)?;
            self.shown = true;
        }
        Ok(())
    }

    fn hide(&mut self) {
        if self.shown {
            self.shown = false;
            self.data = None;
            if let Err(error) = self.webview.set_visible(false) {
                tracing::warn!(%error, "could not hide the Modern UI content view");
            }
        }
    }
}
