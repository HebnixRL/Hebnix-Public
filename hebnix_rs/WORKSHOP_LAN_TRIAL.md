# Workshop LAN trial

This tests custom-map LAN joining without replacing Utopia Retro. It is an
experiment, not a confirmed Error 40 fix. Both players should run the new Hebnix
build so the host shares the original package filename as well as its title.

The joining player enables the tests under Workshop > In-game menu
(experimental) > LAN custom map test. Rocket League must be closed when changing
test mode. Enable the in-game Workshop menu first.

1. Select **1. Test map titles**, then start Rocket League and connect both
   players to Hebnix multiplayer. Host the map from the game's Workshop menu.
   Keep Hebnix on its Workshop tab and allow about 15 seconds for peer metadata
   to arrive. Reopen/refresh **Join LAN Match**. Its map column should show the
   Workshop title, or the host's filename if no title metadata exists.
2. On the joining player, download the hosting player's map to the Workshop
   folder through Hebnix if offered. An older install with a different filename
   will be offered again. Close Rocket League and select **2. Test custom map
   joining**, restart, and join through **Join LAN Match**. There is no need to
   load freeplay or select Utopia Retro first.
3. Record whether the title appears and whether joining succeeds independently.
   The joining client's log now contains `HBX LAN join preparation:` followed by
   its selected server ID. If a matching local file is available, it also logs
   `HBX LAN preload result:`. `None` indicates the experimental loader did not
   resolve the world object. A resolved object followed by Error 40 means this
   loading step was insufficient; it does not prove network package registration.
4. Close Rocket League and select **Disable test** to restore the two original
   LAN functions while preserving other package patches.

The title bridge uses the sidecar's IPv4 aliases and port 7777. It writes bounded
peer titles and locally resolved map paths to `TAGame/HbxLanTrial`. It does not
execute peer-supplied commands or paths. Test state and original function bytes
are saved in `TAGame/CookedPCConsole/Backups/WorkshopLanTrial.json`.

The engine only resolves a package by bare name through its package cache, and
that cache only indexes `.upk` files in its scanned folders. Each offered map is
therefore hardlinked (else copied) to `CookedPCConsole/<name>.upk`, with a
`<name>.staged` marker in the bridge folder, and removed again when the offer
goes. A different package already holding the name is never replaced. The join
then calls `DynamicLoadObject("<name>.TheWorld")` before the normal LAN join.
Full or relative paths, and a `.udk` in `mods`, all returned `None`. Verified on
two PCs: the bare name resolved and the join succeeded.

Staging by map name needs a game restart, because the package cache is built at
launch. To avoid it, every PC keeps a pool of 128 small placeholder packages,
`hbx_slot001` to `hbx_slot128`, in `CookedPCConsole`. While the trial is on, the
host's in-game Workshop list points each map at a copy of it (a hardlink on NTFS,
a copy elsewhere) in `TAGame/HbxSlots`, named for its slot, so the game loads the
map as `hbx_slotNN` and the map keeps its own title in the list. The slot is
announced as the map's `package`. A joiner who has the map swaps the same-named
placeholder to it while the game runs and joins with the usual preload. The first
run on a PC creates the placeholders and needs one game restart. A slot the game
has loaded cannot be swapped. Only maps ticked in the Hebnix map list get a slot copy, and more than 128 ticked maps leave the extra ones unslotted.
Copies show as progress cards in the overlay toasts.

The second build searches the selected Workshop folder, Hebnix's fallback folder,
and mods by original package filename, so a local catalog ID may differ from the
host's ID. Multiple matches outside the exact-ID folder are treated as ambiguous.
For older hosts without a package filename, the last matching join error supplies
the package name for the next attempt. A title without a preload-result line
means the package loading attempt has not been established.

Developer tool: `tools/avatar-proof/target/release/workshop-lan-proof.exe` supports
staging on a package copy, switching live modes with the game closed, and writing
a manual bridge entry. Run it without arguments for usage. Local validation
covers bridge updates, original filename selection, package save/reload, and
restoring all exports; it cannot establish that LAN joining succeeds.

Changed existing functions:

- `fetch_peer_info` on Windows and Linux: accept and validate the package stem.
- `HostedMap::new`, `HostedMap::current`, `describe`: read the loader's title and
  announce the original filename separately.
- `MapManager::install_to_workshop_folder`: validate the supplied package name.
- `WorkshopState::start_relay`, its caller `render_multiplayer`,
  `render_join_prompt`, `install_peer_map`, and `render_player_maps`: carry the
  loader metadata, publish local bridge files, and preserve the host's filename.
- `InGameMenuState::render`: expose the two test modes and restoration.
- `workshop_menu::restore`, `workshop_menu::reapply`: restore the trial before
  removing Workshop hooks, and retain it when another feature rebuilds the package.

New functions include `peer_workshop_file`, `valid_package_stem`, and the
`workshop_lan_trial` module's mode switching, bridge writing, and bytecode builders.
