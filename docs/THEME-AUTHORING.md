# Hebnix theme authoring guide for AI

Use this guide to create clean, readable Hebnix themes. The implementation is in `hebnix_rs/crates/hebnix-app/src/theme.rs` and `theme/presentation.rs`; these are authoritative if the application changes. Existing examples are in `hebnix_rs/examples/themes/`.

## Create and install a theme

1. Write a UTF-8 TOML file named for the theme, for example `Slate.toml`. Use a unique name other than the built-in `Dark` and `Light`.
2. Put it in Hebnix's `themes/` folder. In Settings, open the themes folder if needed, refresh the theme list, and select the file's name under Theme. Both full and Lite use the theme system, though available pages differ.
3. Put optional `.ttf` or `.otf` files in the adjacent `fonts/` folder. Set `font` to the filename **without its extension**, matching case-insensitively. A missing font falls back to the default. Version 2 can also use the bundled `Barlow-Medium` by name.
4. Test the theme in the app, including hover, active, selection, menus, text fields, workshop cards, different window sizes, and available pages. Check long and translated labels and switch themes to verify the result.

Start with `schema_version = 2` for the presentation options below. An absent version or version `1` uses the legacy renderer and ignores `layout`, `typography`, `components`, `pages`, and `assets`. Only versions 1 and 2 are supported. Fields are optional; omitted values use the selected base or the v2 defaults below. Set root keys before TOML table headers.

```toml
schema_version = 2
base = "dark"
font = "Barlow-Medium"

[colors]
window_bg = "#111820"
panel_bg = "#18222C"
widget_bg = "#263440"
accent = "#3A86A8"
text = "#EDF4F7"
weak_text = "#AFBDC5"
selection_text = "#FFFFFF"

[layout]
shell = "sidebar"
navigation_width = 184.0
content_padding = 12.0

[components.navigation]
surface = "#141F28"
selected = "#286680"
text = "#EDF4F7"
icons = true

[components.map_card]
surface = "#202D38"
border = "#41515E"
selected_border = "#65B8DA"
```

## Color and base

`base` is `"dark"` or `"light"` (case-insensitive); an empty or omitted base chooses dark. Light starts from Hebnix's muted light visuals. Colors are strings of six hex digits RGB or eight RGBA, with an optional `#`, for example `"#AABBCC"` or `"#AABBCC80"`. Alpha is the last pair. Version 2 rejects invalid entries in `[colors]`; component colors are also validated. Do not use CSS color names, shorthand hex, or `rgb()`.

| `[colors]` key | Purpose |
| --- | --- |
| `accent` | Selection background, default hyperlink, hovered and active widget fills. Hover fill is a dimmed accent. |
| `window_bg` | Window fill; also derives an extreme background unless explicitly overridden. |
| `panel_bg` | Panel fill; also derives a faint background unless explicitly overridden. |
| `widget_bg` | Background and weak background for noninteractive, inactive, and open widgets. |
| `extreme_bg` | Deep/recessed background. |
| `faint_bg` | Subtle background. |
| `code_bg` | Code background. |
| `text_edit_bg` | Text input background. |
| `text` | Global text color override. |
| `weak_text` | Subdued text. |
| `selection_text` | Selection stroke color. |
| `hyperlink` | Link color; overrides the link color derived from `accent`. |
| `warn`, `error` | Warning and error foreground colors. |

Explicit widget state settings override the corresponding colors derived from `widget_bg` and `accent`. `[style].selection_stroke` overrides `selection_text`. Some app-specific drawing may use colors outside this palette; check the rendered UI rather than assuming every element changes.

## General style

All `[style]` keys are optional. Color values use the format above; radii are integers from 0 to 255. Numeric spacing, widths, and animation time are floating-point values; negative values are clamped to zero. Time is in seconds, and UI sizes are egui points.

| Key | Effect |
| --- | --- |
| `window_corner_radius`, `menu_corner_radius`, `widget_corner_radius` | Window, menu, and all widget-state corner radii. Individual state radii override the widget radius. |
| `window_stroke`, `window_stroke_width` | Window border color and width. |
| `selection_stroke`, `selection_stroke_width` | Selection border/foreground stroke color and width. |
| `item_spacing_x`, `item_spacing_y` | Space between items. |
| `button_padding_x`, `button_padding_y` | Button inner padding. |
| `indent` | Indentation width. |
| `interact_height` | Minimum interaction height. |
| `slider_width`, `text_edit_width` | Default slider and text input widths. |
| `animation_time` | Animation duration. |
| `button_frame`, `collapsing_header_frame`, `indent_has_left_vline`, `striped`, `slider_trailing_fill` | Boolean switches for their corresponding egui visual settings. |

## Widget states

Each of `[widgets.noninteractive]`, `[widgets.inactive]`, `[widgets.hovered]`, `[widgets.active]`, and `[widgets.open]` accepts the same optional keys:

| Key | Type and meaning |
| --- | --- |
| `bg_fill`, `weak_bg_fill` | Color strings for normal and subtle background fills. |
| `bg_stroke`, `fg_stroke` | Background border and foreground stroke colors. |
| `bg_stroke_width`, `fg_stroke_width` | Numeric stroke widths, clamped to zero or greater. |
| `corner_radius` | Integer radius from 0 to 255. |
| `expansion` | Numeric expansion beyond widget bounds, clamped to zero or greater. |

Make inactive, hover, active, and open states distinguishable without relying only on saturation. Avoid large expansion or border widths that make layouts feel crowded.

## Version 2 presentation

The tables in this section apply only with `schema_version = 2`. Their unknown fields are rejected. Values shown are defaults, with inclusive validation ranges where applicable. Use TOML booleans (`true`/`false`) and numeric values, not quoted strings.

### `[layout]`

| Key | Default | Options and effect |
| --- | --- | --- |
| `shell` | `"classic"` | `"classic"`, `"topbar"`, `"sidebar"`, or `"rail"`. Classic retains the classic shell; topbar uses horizontal navigation; sidebar uses a left panel; rail uses a fixed 96-point left rail. |
| `navigation_width` | `184.0` | 64–320 points; sidebar width. |
| `content_padding` | `12.0` | 0–32 points. |
| `secondary_navigation_width` | `200.0` | 120–320 points. |
| `show_page_heading` | `true` | Show page headings. |
| `right_align_actions` | `false` | Align supported toolbar actions to the right. |
| `joined_actions` | `false` | Join supported action controls. |
| `navigation_order` | `[]` | Array of page IDs, e.g. `["Home", "Maps", "Settings"]`. Unknown and duplicate IDs cannot remove or duplicate pages; omitted available pages are appended. Some related pages have special placement rules, so this is a preference, not an absolute order. |

Common IDs include `Home`, `Console`, `Workshop`/`Maps`, `Spoofer`, `Colours`, `Items`, `UPK Studio`, `Game Mods`, `Freeplay`, `Plugins`, `Replays`, `Settings`, and `About`; use IDs applicable to the target build. Console is placed after Settings and Freeplay before Game Mods when both exist.

### `[typography]`

| Key | Default | Valid range, points |
| --- | --- | --- |
| `heading` | `24.0` | 12–40 |
| `body` | `14.0` | 10–22 |
| `small` | `11.0` | 9–18 |
| `button` | `14.0` | 10–22 |

### `[components.navigation]`

| Key | Default | Meaning |
| --- | --- | --- |
| `icons` | `false` | Show navigation icons. |
| `surface` | `"#16212C"` | Navigation background. |
| `selected` | `"#2E86C1"` | Selected item color. |
| `text` | `"#D6E4F0"` | Navigation text color. |
| `height` | `34.0` | Item height, 24–52 points. |
| `corner_radius` | `6` | Integer radius, 0–255. |

### `[components.map_card]`

| Key | Default | Meaning |
| --- | --- | --- |
| `surface` | `"#18212B"` | Card background. |
| `border` | `"#334155"` | Card border. |
| `selected_border` | `"#2E86C1"` | Selected card border. |
| `corner_radius` | `8` | Integer radius, 0–255. |
| `padding` | `10.0` | Inner padding, 0–24 points. |
| `min_width` | `240.0` | Minimum card width, 180–480 points. |
| `image_height` | `140.0` | Image height, 60–240 points. |
| `height` | `320.0` | Card height, 180–800 points. |
| `horizontal` | `false` | Use horizontal card layout. |

### `[pages.workshop]`

| Key | Default | Meaning |
| --- | --- | --- |
| `layout` | `"grid"` | `"grid"` or `"library_with_details"`. |
| `details_width` | `300.0` | Details pane width, 240–420 points. |
| `columns` | `0` | Integer 0 for automatic or 1–8 preferred columns. The grid reduces the count when available width cannot fit the cards. |

### `[assets]`

`background = "assets/my-background.png"` adds an optional content background. The path is relative to the `themes/` directory, must start with `assets/`, use forward slashes and normal path components, and end in lowercase `.png`, `.jpg`, `.jpeg`, or `.webp`. The image must exist, decode successfully, be no larger than 8 MiB, and be at most 4096 × 4096 pixels. Avoid busy images behind text. Omit the key for no image.

## Design and quality checklist

- Pick one clear accent and a restrained set of related surfaces. Separate window, panel, and widget surfaces subtly; avoid pure black/white everywhere and excessive borders.
- Ensure readable contrast for normal and subdued text, links, selected text, warnings, errors, and foregrounds across every widget state. Check contrast over any background image and under the user's window-opacity setting.
- Keep spacing consistent, controls large enough to use, and type sizes readable. Prefer modest radii and padding; ensure cards fit their contents and remain usable at narrow widths.
- In a light theme, explicitly choose light-compatible v2 navigation and card colors: their defaults are dark even when `base = "light"`.
- Set only supported options. Use explicit state overrides when the accent produces poor hover or active contrast; a global text override can affect readability on those fills.
- Validate TOML syntax and v2 ranges, then actually preview in Hebnix. Invalid v2 component colors, assets, ranges, or schema versions can prevent the theme from loading. Legacy color parsing is permissive and invalid values may silently leave defaults. A failed application falls back to Dark.
