# Instant Suite winner movie

`winner_menu_ui.swf` is Rocket League's `GFX_WinnerMenu.WinnerMenu` Flash movie
with one call added where `prompts.WinnerPrompt.onWinningTeamChanged` shows the
winner mid-screen, and a fallback at the end of `showPodiumSpotlight`. It asks
once per match. Hebnix writes it into
`GFX_WinnerMenu_SF.upk` when Instant Suite is switched on
(`patcher/instant_suite.rs`), but only over the vanilla movie (sha256
`28f28cd0...0069`); the original package is kept in the backups folder.

The call loads `img://http://127.0.0.1:47853/is/<time>.png` from Hebnix's
preview server (`preview_server.rs`). The image's width is the action Hebnix
has picked right now, so it can change while the game runs:

| width | action |
|---|---|
| 1 | requeue (`onQuickRequeue`) |
| 2 | freeplay (`handleFreeplayRequest`) |
| 3 | main menu (`Shell.ExitToMainMenu`) |
| 4 | load the Workshop list row whose `Filename` holds `n` (`LoadWorkshopMap`) |

Hebnix only answers within 30 seconds of the Stats API's `MatchEnded`, and holds
the request up to 3 seconds for it, since both come at the same moment. Otherwise,
or with Hebnix closed, it answers 404 and the winner screen stays as it is.

Rebuild: export the vanilla movie from the package, set its signature to `FWS`
and save it as `in.swf`, then with RABCDAsm (`abcexport`, `rabcdasm`, `rabcasm`,
`abcreplace`):

1. `abcexport in.swf` and `rabcdasm in-0.abc`, renaming the folder `in-0` to `rt`
2. run `build/build_winner.py v2` from that folder; it writes `v2/o.swf`
3. copy `v2/o.swf` over `winner_menu_ui.swf`
