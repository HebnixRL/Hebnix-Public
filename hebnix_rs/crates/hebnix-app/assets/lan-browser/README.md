# LAN browser movie

`lan_browser_ui.swf` is Rocket League's `GFX_ServerBrowserMenu.ServerBrowserMenu`
Flash movie (the Local Matches list) with a SYNC MAP entry added to
`menus.ServerBrowserMenu`: right-click on a row, Y on a controller, or the
button in the legend. It logs `HBXSYNC: owner|server|map` to Launch.log, which
Hebnix reads (`ui/workshop/ingame_menu.rs`) to sync the map slots and fetch the
hosted map if it is missing. Hebnix writes it into `GFX_ServerBrowserMenu_SF.upk`
with the in-game Workshop menu (`patcher/workshop_menu_ui.rs`), but only over
the vanilla movie (sha256 `49cb7605...13ac`); the original package is kept in
the backups folder as `.bak`.

Rebuild: export the vanilla movie from the package, then with RABCDAsm
(`abcexport`, `rabcdasm`, `rabcasm`, `abcreplace`):

1. `abcexport in.swf` and `rabcdasm in-0.abc`, which makes the folder `in-0`
2. run `build/build_browser.py` from the folder holding `in.swf` and `in-0`;
   it writes `s1/o.swf`
3. copy `s1/o.swf` over `lan_browser_ui.swf`
