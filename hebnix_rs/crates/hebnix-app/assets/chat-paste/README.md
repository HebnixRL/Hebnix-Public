# Chat paste HUD movie

`chat_paste_ui.swf` is Rocket League's `GFX_Hud.HUD` Flash movie with Ctrl+V
added to the match chat box. The chat classes (`ChatTextInput`, `HudChatLogView`)
also live in the shared code library, which loads first and wins, so a patch
there never runs; instead the `HudScreen` constructor, which only this movie
has, adds a capture `KEY_DOWN` listener to its chat log view. The new `hbxPaste`
method reads the clipboard through the game's own `Shell.PasteFromClipboard`, turns
line breaks into spaces, cuts it to the box's `maxChars` and puts it in at the
caret of the text field that has focus. Hebnix swaps it into `GFX_Hud_SF.upk` when chat paste is switched on
(`patcher/chat_paste.rs`), but only over the vanilla movie (sha256
`a3011bd5...33ac`). The boost meter textures live in the same package, so only
the movie is swapped and the vanilla movie is kept in `Backups/ChatPaste`.

Rebuild: export the vanilla movie from the package, set its signature to `FWS`
and save it as `in.swf`, then with RABCDAsm (`abcexport`, `rabcdasm`, `rabcasm`,
`abcreplace`):

1. `abcexport in.swf` and `rabcdasm in-0.abc`, renaming the folder `in-0` to `rt`
2. run `build/build_chat.py v1` from that folder; it writes `v1/o.swf`
3. copy `v1/o.swf` over `chat_paste_ui.swf`
