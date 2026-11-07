# Workshop menu movie

`workshop_menu_ui.swf` is Rocket League's `GFX_WorkshopMenu.WorkshopMenu` Flash
movie with the LAN controls added to `menus.WorkshopMenu`: a tickbox per map
(the game's own `OptionsCheckBoxMovie`), the garage's `ModalSearchTextInput`
as a search box, a "LAN READY" caption, right-click / Y to host, and a HOST LAN
entry in the button legend. Hebnix writes it into `GFX_WorkshopMenu_SF.upk` when
the in-game Workshop menu is enabled (`patcher/workshop_menu_ui.rs`), but only
over the vanilla movie (sha256 `c0fcdf7a...0ec4`); the original package is kept
in the backups folder.

Rebuild: export the vanilla movie from the package, then with RABCDAsm
(`abcexport`, `rabcdasm`, `rabcasm`, `abcreplace`):

1. `abcexport in.swf` and `rabcdasm in-0.abc`, naming the folder `rt/v-0`
2. run `build/build_menu.py` from a folder holding `in.swf`, `rt/v-0` and
   `build/mk2.py`; it writes `v18/o.swf`
3. copy `v18/o.swf` over `workshop_menu_ui.swf`

The menu logs `HBXTICK` lines to Launch.log when a box is ticked, which Hebnix
reads (`ui/workshop/ingame_menu.rs`).
