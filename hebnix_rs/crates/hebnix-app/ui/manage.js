const field = id => document.getElementById(id);
const send = action => {
  if (state?.page === "plugin-window") {
    const type = {
      plugin_settings_input: "plugin_window_input",
      plugin_settings_expand: "plugin_window_expand",
    }[action.type];
    if (type) window.ipc.postMessage(JSON.stringify({ ...action, type }));
  } else window.ipc.postMessage(JSON.stringify({ type: "manage", action }));
};
const controls = field("controls");
const rows = field("rows");
const query = field("query");
const status = field("status");
const pageSize = 12;
let state;
let page = 0;
let pendingRemoval = null;
let requested = "";
let overlayOpen = false;

function element(tag, className, text = "") {
  const node = document.createElement(tag);
  node.className = className;
  node.textContent = text;
  return node;
}

function button(text, action, disabled = false, style = "outline-secondary") {
  const node = element("button", "btn btn-sm btn-" + style, text);
  node.type = "button";
  node.disabled = disabled;
  node.addEventListener("click", action);
  return node;
}

function icon(symbol, label, action, disabled = false, style) {
  const node = button(symbol, action, disabled, style);
  node.classList.add("ui-icon-button");
  node.title = label;
  node.setAttribute("aria-label", label);
  return node;
}

function pluginAction(kind, label, action, disabled = false) {
  const paths = {
    folder: "M3 7V5h6l2 2h10v12H3Z",
    zip: "M12 3v12m-5-5 5 5 5-5M4 17v4h16v-4",
    reload: "M20 7v5h-5M20 12a8 8 0 1 0-2 6",
    refresh: "M20 3v6h-6M4 21v-6h6M20 9a8 8 0 0 0-14-3M4 15a8 8 0 0 0 14 3",
  };
  const control = button("", action, disabled);
  control.classList.add("plugin-labelled-action");
  const icon = element("span", "plugin-labelled-icon");
  icon.setAttribute("aria-hidden", "true");
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  const path = document.createElementNS(svg.namespaceURI, "path");
  path.setAttribute("d", paths[kind]);
  svg.append(path); icon.append(svg);
  control.append(icon, element("span", "plugin-labelled-text", label));
  return control;
}

function command(key) {
  send({ type: "plugin_command", key });
}

function checkbox(plugin, id) {
  const box = element("input", "form-check-input mt-0");
  box.type = "checkbox";
  box.checked = plugin.enabled;
  box.disabled = !!plugin.error;
  box.title = "Enable or disable " + plugin.name;
  box.setAttribute("aria-label", "Enable " + plugin.name);
  box.addEventListener("change", () => send({ type: "plugin_toggle", id, enabled: box.checked }));
  return box;
}

function details(plugin) {
  const copy = element("div", "plugin-copy");
  copy.append(element("h2", "h6 mb-1 text-break", plugin.name),
    element("p", "small text-body-secondary mb-0", [plugin.version, plugin.author].filter(Boolean).join(" · ")));
  if (plugin.description) copy.append(element("p", "mb-0 small text-break", plugin.description));
  if (plugin.error) copy.append(element("p", "small text-warning mb-0", plugin.error));
  return copy;
}

function settingsButton(plugin, id) {
  return icon("⚙", "Settings for " + plugin.name, () => send({ type: "plugin_settings", id }), !plugin.enabled);
}

function installedRow(plugin) {
  const card = element("article", "tracker-detail plugin-installed-row");
  const actions = element("div", "plugin-actions");
  if (plugin.settings) actions.append(settingsButton(plugin, plugin.id));
  if (pendingRemoval === plugin.id) {
    actions.append(icon("✓", "Confirm removal of " + plugin.name, () => {
      pendingRemoval = null;
      send({ type: "plugin_delete", id: plugin.id });
      render();
    }, false, "outline-danger"), icon("×", "Cancel removal", () => {
      pendingRemoval = null;
      render();
    }));
  } else {
    actions.append(icon("🗑", "Remove " + plugin.name, () => {
      pendingRemoval = plugin.id;
      render();
    }, false, "outline-danger"));
  }
  card.append(checkbox(plugin, plugin.id), details(plugin), actions);
  return card;
}

function pluginWidget(widget) {
  const update = value => send({ type: "plugin_settings_input", plugin: state.selected, id: widget.id, value });
  const row = element("div", "plugin-widget");
  switch (widget.type) {
    case "heading": return element("h3", "h6 mb-1", widget.label);
    case "label": return element("p", "mb-1 text-break", widget.label);
    case "colored_label": {
      const label = element("p", "mb-1 text-break", widget.label);
      if (/^#?[0-9a-f]{6}(?:[0-9a-f]{2})?$/i.test(widget.color)) label.style.color = "#" + widget.color.replace(/^#/, "");
      return label;
    }
    case "separator": return element("hr", "my-2");
    case "space":
      row.style.height = Math.max(0, Math.min(widget.size, 1200)) + "px";
      return row;
    case "button": return button(widget.label, () => update(true));
    case "checkbox": return toggle(widget.label, widget.value, update);
    case "select":
      row.append(element("span", "", widget.label),
        loadoutSelect(widget.label, widget.options.map(choice => [choice, choice]), widget.value, update));
      return row;
    case "slider": {
      const range = element("input", "form-range w-auto");
      const number = element("input", "form-control form-control-sm w-auto");
      range.type = "range";
      number.type = "number";
      range.min = number.min = String(widget.min);
      range.max = number.max = String(widget.max);
      range.step = number.step = "any";
      range.value = number.value = String(widget.value);
      range.setAttribute("aria-label", widget.label);
      number.setAttribute("aria-label", widget.label + " value");
      range.addEventListener("input", () => { number.value = range.value; });
      range.addEventListener("change", () => update(Number(range.value)));
      number.addEventListener("change", () => { if (number.value !== "") update(Number(number.value)); });
      row.append(element("span", "", widget.label), range, number);
      return row;
    }
    case "color":
    case "text":
    case "password": {
      const field = element("input", "form-control form-control-sm w-auto");
      field.type = widget.type;
      field.value = widget.value;
      field.setAttribute("aria-label", widget.label);
      field.dataset.widget = widget.id;
      if (widget.type === "color") {
        field.classList.add("form-control-color");
        row.append(element("span", "", widget.label));
      } else {
        field.placeholder = widget.label;
      }
      field.addEventListener("change", () => update(field.value));
      row.append(field);
      return row;
    }
    case "readonly": {
      const text = element("pre", "plugin-widget-scroll mb-0", widget.label);
      text.style.maxHeight = widget.height + "px";
      return text;
    }
    case "image": {
      const img = element("img", "plugin-widget-image");
      img.src = widget.image;
      img.alt = "";
      if (widget.width !== null && widget.width >= 0) img.style.width = widget.width + "px";
      if (widget.height !== null && widget.height >= 0) img.style.height = widget.height + "px";
      if (widget.x !== null && widget.y !== null && widget.width !== null && widget.height !== null) {
        img.classList.add("plugin-widget-overlay");
        img.style.left = widget.x + "px";
        img.style.top = widget.y + "px";
      }
      if (widget.tint && /^#?[0-9a-f]{8}$/i.test(widget.tint)) img.style.opacity = parseInt(widget.tint.slice(-2), 16) / 255;
      return img;
    }
    case "collapsing": {
      const details = element("details", "plugin-settings-fold");
      details.open = widget.open;
      details.append(element("summary", "", widget.label), ...widget.children.map(pluginWidget));
      details.addEventListener("toggle", () => {
        if (details.open !== widget.open) send({ type: "plugin_settings_expand", plugin: state.selected, id: widget.id, open: details.open });
      });
      return details;
    }
    case "horizontal":
    case "columns":
    case "column":
    case "scroll": {
      row.classList.add("plugin-widget-" + widget.type);
      if (widget.type === "columns") row.style.setProperty("--plugin-column-width", widget.width + "px");
      if (widget.type === "scroll") {
        row.classList.add("plugin-widget-scroll");
        row.style.maxHeight = widget.height + "px";
      }
      row.append(...widget.children.map(pluginWidget));
      return row;
    }
    default: return row;
  }
}

function pluginSettings() {
  if (!state.plugins.length) {
    const card = panel();
    card.append(element("p", "text-body-secondary", state.empty),
      button(state.go, () => send({ type: "plugin_settings_installed" })));
    rows.append(card);
    return;
  }
  const layout = element("div", "loadout-layout plugin-settings-layout");
  layout.dataset.plugin = state.selected;
  const navigation = element("nav", "tracker-detail p-2 d-grid gap-1");
  for (const [id, label] of state.plugins) {
    navigation.append(button(label, () => send({ type: "plugin_settings_select", id }), false,
      id === state.selected ? "primary" : "outline-secondary"));
  }
  const view = panel();
  view.append(element("h2", "h5 mb-3", state.title));
  if (state.error) view.append(element("p", "text-warning", state.error));
  const widgets = element("div", "plugin-settings-widgets");
  widgets.append(...state.widgets.map(pluginWidget));
  view.append(widgets);
  layout.append(navigation, view);
  rows.append(layout);
}

function floatingPlugin() {
  const post = type => window.ipc.postMessage(JSON.stringify({ type }));
  const header = element("header", "d-flex gap-2 justify-content-between align-items-center pb-2 border-bottom");
  const grip = element("h1", "h6 mb-0 flex-grow-1", state.title);
  grip.style.cursor = "grab";
  grip.addEventListener("pointerdown", event => {
    if (event.button === 0) post("plugin_window_drag");
  });
  header.append(grip);
  if (state.close) header.append(icon("×", "Close plugin window", () => post("plugin_window_close")));
  const body = element("div", "position-relative p-2 d-grid gap-2");
  body.style.backgroundColor = "rgb(16 29 48 / " + Math.max(0, Math.min(1, state.opacity)) + ")";
  if (state.error) body.append(element("p", "text-danger", state.error));
  body.append(...state.widgets.map(pluginWidget));
  rows.append(header, body);
}

function overlayOrder(list) {
  const section = element("details", "tracker-detail p-3");
  section.open = overlayOpen;
  section.addEventListener("toggle", () => { overlayOpen = section.open; });
  section.append(element("summary", "", "Overlay order"));
  list.forEach(([id, name], index) => {
    const row = element("div", "d-flex gap-2 align-items-center mt-2");
    const move = step => send({ type: "overlay_move", id, step });
    row.append(element("span", "flex-grow-1", name),
      icon("↑", "Move " + name + " up", () => move(-1), index === 0),
      icon("↓", "Move " + name + " down", () => move(1), index === list.length - 1));
    section.append(row);
  });
  return section;
}

function catalogCard(plugin) {
  const card = element("article", "tracker-detail plugin-card");
  if (plugin.banner) {
    const banner = element("img", "plugin-banner");
    banner.src = plugin.banner;
    banner.alt = "";
    card.append(banner);
  }
  const body = element("div", "plugin-card-body");
  const actions = element("div", "plugin-actions mt-auto pt-2");
  if (plugin.installed) {
    const label = plugin.enabled ? "Disable" : "Enable";
    const availability = button(label, () => send({
      type: "plugin_toggle", id: plugin.installed, enabled: !plugin.enabled
    }), Boolean(plugin.error), plugin.enabled ? "outline-secondary" : "primary");
    availability.setAttribute("aria-label", label + " " + plugin.name);
    actions.append(availability);
    if (plugin.settings) actions.append(settingsButton(plugin, plugin.installed));
  } else {
    const busy = state.downloading === plugin.id;
    actions.append(button(busy ? "Installing…" : "Install", () => send({ type: "plugin_install", id: plugin.id }),
      !!state.downloading, "primary"));
  }
  body.append(details(plugin), actions);
  card.append(body);
  return card;
}

function panel() {
  return element("section", "tracker-detail p-3");
}

function controllerMenu() {
  const post = data => window.ipc.postMessage(JSON.stringify(data));
  const head = element("div", "d-flex align-items-center justify-content-between gap-2");
  head.append(element("h1", "h4 mb-0", "Hebnix"),
    button("Close", () => post({ type: "panel_back" })));
  const menu = panel();
  menu.append(head, element("p", "text-body-secondary small mt-2", "A Select · B Close"));
  const list = element("div", "d-grid gap-2");
  state.pages.forEach((label, index) => {
    const tile = button(label, () => post({ type: "panel_select", index }));
    tile.classList.add("py-3", "fs-5");
    list.append(tile);
  });
  menu.append(list, toggle("Advanced mode", state.advanced, enabled => post({ type: "panel_advanced", enabled })));
  rows.append(menu);
}

function toggle(label, checked, change, disabled = false) {
  const row = element("label", "d-flex align-items-center gap-2 mb-2");
  const box = element("input", "form-check-input mt-0");
  box.type = "checkbox";
  box.checked = checked;
  box.disabled = disabled;
  box.addEventListener("change", () => change(box.checked));
  row.append(box, label);
  return row;
}

function playerName() {
  const card = panel();
  card.classList.add("player-name-panel");
  const sendName = (name, commit) => send({ type: "player_name", name, commit });
  const head = element("div", "d-flex align-items-center justify-content-between gap-2");
  const count = element("span", "text-body-secondary small");
  const input = element("input", "form-control text-center mb-3");
  input.type = "text";
  input.value = state.name;
  input.setAttribute("aria-label", "Display name");
  const countName = () => { count.textContent = Array.from(input.value).length + " / " + state.max; };
  countName();
  input.addEventListener("input", () => {
    if (Array.from(input.value).length > state.max) input.value = Array.from(input.value).slice(0, state.max).join("");
    countName();
    sendName(input.value, false);
  });
  input.addEventListener("change", () => sendName(input.value, true));
  head.append(toggle("Display Name", state.enabled, enabled => send({ type: "player_name_enabled", enabled })), count);
  card.append(element("p", "text-body-secondary", "Choose your display name"), head, input,
    element("h2", "h6", "Previous names"));
  const history = element("div", "player-name-history");
  for (const name of state.history) {
    const item = button(name, () => sendName(name, true), false, name === state.name ? "primary" : "outline-secondary");
    item.classList.add("text-truncate");
    item.title = name;
    history.append(item);
  }
  card.append(state.history.length ? history : element("p", "text-body-secondary small", "No previous names yet."),
    element("p", "text-body-secondary small mt-3 mb-0", "Names are saved automatically."));
  rows.classList.add("player-name-layout");
  rows.append(card);
}

function friendNames() {
  query.hidden = false;
  query.placeholder = state.search;
  query.setAttribute("aria-label", state.search);
  const card = panel();
  controls.classList.remove("mb-3");
  controls.append(toggle(state.toggle, state.enabled, enabled => send({ type: "friend_names_enabled", enabled })));
  const term = query.value.trim().toLowerCase();
  // Keep the shared search input in place between model updates.
  for (const friend of state.rows.filter(friend => [friend.name, friend.alias].some(value => value.toLowerCase().includes(term)))) {
    const row = element("div", "player-friend-row mb-2");
    const update = values => send({ type: "friend_name", id: friend.id, ...values });
    const enabled = element("input", "form-check-input mt-0");
    enabled.type = "checkbox";
    enabled.checked = friend.enabled;
    enabled.setAttribute("aria-label", "Enable " + friend.name);
    enabled.addEventListener("change", () => update({ enabled: enabled.checked }));
    const original = element("input", "form-control form-control-sm");
    original.value = friend.name;
    original.readOnly = true;
    original.setAttribute("aria-label", friend.name);
    const alias = element("input", "form-control form-control-sm");
    alias.value = friend.alias;
    alias.placeholder = state.placeholder;
    alias.setAttribute("aria-label", state.placeholder + " · " + friend.name);
    alias.addEventListener("change", () => update({ name: alias.value }));
    row.append(enabled, original, alias);
    card.append(row);
  }
  const revert = button(state.revert, () => send({ type: "revert_friend_names" }));
  revert.classList.add("ms-auto");
  controls.append(revert);
  if (!state.rows.length) card.append(element("p", "text-body-secondary mb-0", state.empty));
  rows.append(card);
}

function scoreboard() {
  const labels = state.labels;
  const set = (key, value) => send({ type: "scoreboard", key, value });
  const names = panel(), ratings = panel();
  names.append(element("h2", "h6", labels.names),
    toggle(labels.enableNames, state.names, value => set("names", value), !state.ready),
    toggle(labels.ignore, state.ignore, value => set("ignore", value)));
  ratings.append(element("h2", "h6", labels.mmr),
    toggle(labels.enableMmr, state.mmr, value => set("mmr", value), !state.ready));
  for (const mode of ["playlist", "gamemode", "manual"]) {
    const label = element("label", "d-flex gap-2 align-items-center mb-2");
    const radio = element("input", "form-check-input mt-0");
    radio.type = "radio";
    radio.name = "mmr-source";
    radio.checked = state.source === mode;
    radio.addEventListener("change", () => set("source", mode));
    label.append(radio, labels[mode]);
    ratings.append(label);
  }
  if (state.source === "manual") {
    const select = element("select", "form-select form-select-sm w-auto");
    select.setAttribute("aria-label", labels.choose);
    select.append(new Option(labels.choose, "", false, state.playlist === null));
    select.firstChild.disabled = true;
    for (const [id, label] of state.playlists) select.append(new Option(label, id, false, state.playlist === id));
    select.addEventListener("change", () => set("playlist", Number(select.value)));
    ratings.append(select);
  }
  status.textContent = state.status || (state.loading ? "Loading…" : "");
  status.hidden = !status.textContent;
  rows.append(names, ratings);
}

function actionButtons() {
  const card = panel();
  card.append(toggle("Show action button in header", state.showInHeader, enabled => send({type: "action_button_visibility", enabled})));
  card.append(element("p", "mb-1", state.intro), element("p", "text-body-secondary small", state.hint));
  for (const running of [false, true]) {
    const entries = running ? state.open : state.closed;
    const active = entries.filter(entry => entry.enabled).length;
    const list = element("ol", "list-group list-group-numbered mb-3");
    let dragged = null;
    card.append(element("h2", "h6", running ? state.openLabel : state.closedLabel), list);
    entries.forEach((entry, index) => {
      const row = element("li", "list-group-item d-flex align-items-center gap-2");
      const blocked = entry.enabled && active === 1;
      const check = toggle(entry.label, entry.enabled, enabled => send({ type: "action_button_toggle", running, index, enabled }), blocked);
      check.classList.remove("mb-2");
      if (blocked) check.title = state.keepOne;
      const move = (from, to) => send({ type: "action_button_move", running, from, to });
      const handle = element("span", "action-drag-handle", "⠿");
      handle.draggable = true;
      handle.addEventListener("dragstart", event => {
        dragged = index;
        event.dataTransfer.setData("text/plain", String(index));
      });
      handle.addEventListener("dragend", () => { dragged = null; });
      row.addEventListener("dragover", event => { if (dragged !== null) event.preventDefault(); });
      row.addEventListener("drop", event => {
        event.preventDefault();
        if (dragged !== null && dragged !== index) move(dragged, index);
        dragged = null;
      });
      const reorder = element("div", "plugin-actions ms-auto");
      reorder.append(icon("↑", "Move " + entry.label + " up", () => move(index, index - 1), index === 0),
        icon("↓", "Move " + entry.label + " down", () => move(index, index + 1), index === entries.length - 1));
      row.append(handle, check, reorder);
      list.append(row);
    });
  }
  rows.append(card);
}
function addMaps() {
  const action = (type, key, data = {}) => send({ type: "add_maps", action: { type, key, ...data } });
  if (state.menu_disabled || state.menu_busy) {
    const lock = element("div", "menu-lock");
    lock.id = "menu-lock";
    lock.setAttribute("role", "alertdialog");
    lock.setAttribute("aria-modal", "true");
    const card = element("div", "menu-lock-card");
    const enable = button(state.menu_busy ? state.menu_lock.busy : state.menu_lock.enable,
      () => send({ type: "add_maps", action: { type: "enable_menu" } }), state.menu_busy, "primary");
    card.append(element("h2", "h6", state.menu_lock.title), element("p", "", state.menu_lock.body), enable);
    lock.append(card);
    document.body.append(lock);
    rows.inert = true;
  }
  const local = state.local;
  const labels = local.labels;
  const editor = panel();
  editor.append(element("h2", "h6", labels.title), element("p", "text-body-secondary", labels.intro));
  if (!local.details) {
    editor.append(element("h3", "h6", labels.file), element("p", "small text-body-secondary", labels.types));
    if (local.done) editor.append(element("p", "text-success", local.done));
    editor.append(button(labels.choose, () => action("local", "file"), false, "primary"));
  } else {
    editor.append(element("h3", "h6", labels.details), element("p", "small text-break", local.file));
    const fields = ["name", "author", "description"].map((key, index) => {
      const label = element("label", "d-grid gap-1 mb-2");
      const input = element(index === 2 ? "textarea" : "input", "form-control form-control-sm");
      input.value = local.fields[index];
      label.append(element("span", "", labels[key]), input);
      editor.append(label);
      return input;
    });
    const call = key => action("local", key, { fields: fields.map(field => field.value) });
    const controls = element("div", "d-flex gap-2 flex-wrap align-items-center");
    controls.append(button(labels.vdf, () => call("vdf")), button(labels.image, () => call("image")),
      element("span", "small text-break", local.banner || labels.optional));
    if (local.banner) controls.append(button(labels.clear, () => call("clear")));
    editor.append(controls);
    const actions = element("div", "d-flex gap-2 mt-3");
    const submit = button(labels.import, () => call("import"), !fields[0].value.trim(), "primary");
    fields[0].addEventListener("input", () => { submit.disabled = !fields[0].value.trim(); });
    actions.append(button(labels.back, () => call("back")), submit);
    editor.append(actions);
  }
  if (local.notice) editor.append(element("p", "small text-body-secondary mt-2", local.notice));
  if (local.error) editor.append(element("p", "text-warning mt-2", local.error));
  rows.append(editor);
}

function replayCommandEditor(timeline, call) {
  const selected = timeline.selected;
  const row = element("div", "d-flex align-items-center gap-2 flex-wrap mb-2");
  const edit = state.ready && !state.playing;
  const choices = [["", "+ Add command"], ...timeline.commands.map((command, index) => [index, command])];
  const add = loadoutSelect("Add command", choices, "", value => { if (value !== "") call("add_command", { index: Number(value) }); });
  add.disabled = !edit;
  row.append(element("strong", "", "Keyframe · Frame " + selected.frame), add,
    button("Delete keyframe", () => call("delete_key"), !edit, "outline-danger"));
  rowsForCommands(row, selected.commands, timeline, edit, call);
  return row;
}

function rowsForCommands(container, commands, timeline, edit, call) {
  commands.forEach(({ label, command }, index) => {
    const line = element("div", "d-flex align-items-center gap-2 w-100 border-top py-2");
    line.append(element("span", "", label));
    const [kind, value] = Object.entries(command)[0];
    const change = value => call("command_value", { index, value });
    if (kind === "Pov") {
      const players = timeline.players.map(([name, key]) => [key, name + " (" + key + ")"]);
      if (!players.some(([key]) => String(key) === value.focus)) players.unshift([value.focus, value.focus]);
      const select = loadoutSelect(label, players, value.focus, change);
      select.disabled = !edit;
      line.append(select, element("span", "small text-body-secondary", "Player View"));
    } else if (kind === "Hud") {
      line.append(toggle("Visible", value, change, !edit));
    } else {
      const number = element("input", "form-control form-control-sm w-auto");
      number.type = "number";
      number.min = kind === "Seek" ? "1" : kind === "Pause" ? ".1" : "0";
      number.max = String(kind === "Seek" ? timeline.max : kind === "Pause" ? 3600 : 16);
      number.step = kind === "Seek" ? "1" : kind === "Pause" ? ".1" : ".05";
      number.value = String(value);
      number.disabled = !edit;
      number.setAttribute("aria-label", label);
      number.addEventListener("change", () => { if (number.value !== "") change(Number(number.value)); });
      line.append(number);
      if (kind === "Speed" && value === 0) line.append(element("span", "small text-body-secondary", "0 freezes replay progression"));
    }
    const remove = button("Remove", () => call("remove_command", { index }), !edit, "outline-danger");
    remove.classList.add("ms-auto");
    line.append(remove);
    container.append(line);
  });
}

let replayDrag = null;
function replayTrack(timeline, call) {
  const panelNode = panel();
  const span = timeline.total / timeline.zoom;
  const zoomLimit = Math.max(1, timeline.max / 4);
  const setView = (zoom, start) => call("view", { zoom, start });
  const bar = element("div", "d-flex align-items-center gap-2 flex-wrap");
  bar.append(element("strong", "", "Frame " + timeline.frame + " / " + timeline.max),
    element("span", "text-body-secondary", timeline.clock),
    icon("−", "Zoom out", () => setView(Math.max(1, timeline.zoom / 1.6), timeline.start)),
    icon("+", "Zoom in", () => setView(Math.min(zoomLimit, timeline.zoom * 1.6), timeline.start)),
    button("Fit", () => setView(1, 0)),
    button("Follow playhead", () => setView(timeline.zoom, timeline.playhead - span / 2)),
    element("span", "small text-body-secondary", timeline.zoom.toFixed(1) + "× zoom · G goal · S save · blue/orange team · gold pause"));
  panelNode.append(bar);
  if (timeline.zoom > 1) {
    const slider = element("input", "form-range mt-2");
    slider.type = "range";
    slider.min = "0";
    slider.step = "any";
    slider.max = String(timeline.total - span);
    slider.value = String(timeline.start);
    slider.setAttribute("aria-label", "Pan timeline");
    slider.addEventListener("change", () => setView(timeline.zoom, Number(slider.value)));
    panelNode.append(slider);
  }
  const track = element("div", "replay-track");
  track.tabIndex = 0;
  track.setAttribute("aria-label", "Replay timeline");
  const fraction = at => (at - timeline.start) / span;
  const place = (item, at) => { item.style.left = Math.max(0, Math.min(1, fraction(at))) * 100 + "%"; track.append(item); };
  const wanted = Math.max(1, span * timeline.fps / 9);
  const base = 10 ** Math.floor(Math.log10(wanted));
  const step = [1, 2, 5, 10].map(value => value * base).find(value => value >= wanted);
  const first = Math.max(0, Math.ceil(timeline.start * timeline.fps / step) * step);
  for (let index = first; index <= (timeline.start + span) * timeline.fps; index += step) {
    const seconds = index / timeline.fps;
    const tick = element("span", "replay-track-tick");
    tick.append(element("small", "", String(index + 1)),
      element("small", "", Math.floor(seconds / 60) + ":" + String(Math.floor(seconds % 60)).padStart(2, "0")));
    place(tick, seconds);
  }
  for (const key of timeline.keys) {
    if (key.hold > 0 && fraction(key.at) < 1 && fraction(key.at + key.hold) > 0) {
      const hold = element("span", "replay-hold");
      hold.style.width = (Math.min(1, fraction(key.at + key.hold)) - Math.max(0, fraction(key.at))) * 100 + "%";
      place(hold, key.at);
    }
    if (fraction(key.at) < 0 || fraction(key.at) > 1) continue;
    const marker = icon("◆", "Frame " + key.frame + " · " + key.count + " commands · " + key.hold.toFixed(1) + "s pause",
      () => call("pick_key", { frame: key.frame }));
    marker.classList.add("replay-key-marker");
    place(marker, key.at);
  }
  for (const marker of timeline.markers) {
    if (fraction(marker.at) < 0 || fraction(marker.at) > 1) continue;
    const label = marker.event === "Goal" ? "G" : marker.event.toLowerCase().includes("save") ? "S" : "•";
    const team = marker.team === 0 ? "Blue" : marker.team === 1 ? "Orange" : "Unknown";
    const item = icon(label, [marker.event, marker.player, team + " team", "Frame " + marker.frame].join(" · "),
      () => call("scrub", { frame: marker.frame }), !state.ready);
    item.classList.add("replay-event-marker", "team-" + team.toLowerCase());
    place(item, marker.at);
  }
  if (fraction(timeline.playhead) >= 0 && fraction(timeline.playhead) <= 1) {
    place(element("span", "replay-playhead"), timeline.playhead);
  }
  track.addEventListener("pointerdown", event => {
    if (!state.ready || event.button !== 0 || event.target.closest("button")) return;
    const bounds = track.getBoundingClientRect();
    replayDrag = { bounds, start: timeline.start, span, fps: timeline.fps, max: timeline.max, previous: null, call };
    scrubReplayPointer(event);
  });
  track.addEventListener("wheel", event => {
    event.preventDefault();
    const bounds = track.getBoundingClientRect();
    const at = Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width));
    const zoom = Math.max(1, Math.min(zoomLimit, timeline.zoom * Math.exp(-event.deltaY * .005)));
    setView(zoom, timeline.start + at * (span - timeline.total / zoom));
  }, { passive: false });
  panelNode.append(track);
  return panelNode;
}

function scrubReplayPointer(event) {
  if (!replayDrag || state?.page !== "replay-library" || !state.ready) return;
  const drag = replayDrag;
  const at = Math.max(0, Math.min(1, (event.clientX - drag.bounds.left) / drag.bounds.width));
  const frame = Math.max(1, Math.min(drag.max, 1 + Math.round((drag.start + at * drag.span) * drag.fps)));
  if (drag.previous !== frame) {
    drag.previous = frame;
    drag.call("scrub", { frame });
  }
}
window.addEventListener("pointermove", scrubReplayPointer);
window.addEventListener("pointerup", () => { replayDrag = null; });
window.addEventListener("pointercancel", () => { replayDrag = null; });
window.addEventListener("keydown", event => {
  if (state?.page !== "replay-library" || !state.ready || state.capturing ||
    event.target.closest?.("input, select, textarea, [contenteditable]")) return;
  const offset = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
  if (offset && state.timeline) {
    event.preventDefault();
    send({ type: "replay_library", action: { type: "scrub", frame: state.timeline.frame + offset } });
  }
});

function replayLibrary() {
  const call = (type, data = {}) => send({ type: "replay_library", action: { type, ...data } });
  const header = panel();
  if (state.error) header.append(element("p", "text-warning", state.error));
  if (state.scan !== null) {
    const progress = element("progress", "w-100");
    progress.max = 1;
    progress.value = Math.min(1, Math.max(0, state.scan));
    progress.setAttribute("aria-label", "Scanning replay events · 16×");
    header.append(element("p", "small mb-1", "Scanning replay events · 16×"), progress,
      button("Cancel scan", () => call("cancel_scan")));
  }
  if (state.scanFailed) header.append(button("Retry event scan", () => call("retry_scan"), !state.ready));
  const controls = element("div", "d-flex align-items-center gap-2 flex-wrap");
  controls.append(element("span", state.connected ? "text-success" : "text-body-secondary",
    state.connected ? "Stats API connected" : "Stats API disconnected"),
    button("Detect current replay", () => call("detect")),
    element("span", "", "Play/pause key"), element("strong", "", state.capturing ? "Press a key…" : state.keybind || "Not set"),
    button("Set", () => call("capture"), state.capturing),
    button("Clear", () => call("clear"), !state.keybind && !state.capturing));
  header.append(controls, element("p", "small mt-2 mb-0", state.status));
  const editor = panel();
  editor.append(element("h2", "h6", "Playback & Keyframes"));
  const model = state.timeline;
  if (!state.full || !model) {
    editor.append(element("h3", "h6", "Waiting for a replay"),
      element("p", "text-body-secondary", "Open a replay in Rocket League, then return here. Timeline controls become available while you are watching it."));
    if (state.full) editor.append(element("p", "text-body-secondary", "Could not match this replay to a local file. Check your configured replay folder, then detect again."));
  } else {
    const tools = element("div", "d-flex gap-2 flex-wrap my-2");
    tools.append(button(state.playing ? "Pause" : "Play timeline", () => call("toggle"), !state.ready, "primary"),
      button("← Frame", () => call("scrub", { frame: model.frame - 1 }), !state.ready),
      button("Frame →", () => call("scrub", { frame: model.frame + 1 }), !state.ready),
      button("Frame 1", () => call("scrub", { frame: 1 }), !state.ready),
      button("+ Keyframe", () => call("add_key"), !state.ready || state.playing));
    const keys = element("div", "d-flex gap-2 flex-wrap align-items-center mb-2");
    keys.append(element("span", "", "Keyframes"));
    model.keys.forEach(({ frame }) => keys.append(button("Frame " + frame, () => call("pick_key", { frame }),
      false, model.selected?.frame === frame ? "primary" : "outline-secondary")));
    editor.append(element("strong", "", model.title + " · " + model.max + " frames · " + model.fps + " fps"),
      tools, keys, model.selected ? replayCommandEditor(model, call) :
        element("p", "small text-body-secondary", "Select a frame and add a keyframe, or select a diamond to edit its commands."));
  }
  rows.append(header, editor);
  if (model) rows.append(replayTrack(model, call));
}

function gameMods() {
  for (const mod of state.rows) {
    const card = panel();
    const update = (key, value = null) => send({ type: "game_mod", id: mod.id, key, value });
    card.append(toggle(mod.label, mod.enabled, value => update("enabled", value), mod.busy),
      element("p", "text-body-secondary small", mod.description));
    if (mod.output) card.append(element("p", "text-body-secondary small text-break", mod.output));
    if (mod.keys) {
      const line = element("label", "d-flex align-items-center gap-2 mb-2");
      const select = loadoutSelect(mod.keyboardLabel, mod.keys.map(key => [key, key]), mod.keyboard,
        key => update("keyboard", key));
      select.disabled = mod.busy;
      line.append(element("span", "", mod.keyboardLabel), select);
      const pad = element("div", "d-flex flex-wrap align-items-center gap-2");
      const waiting = state.capturing === mod.id;
      pad.append(element("strong", "", state.labels.pad),
        element("span", "small text-body-secondary", waiting ? state.labels.press : mod.pad || state.labels.none),
        button(state.labels.bind, () => update("bind"), mod.busy || !!state.capturing));
      if (mod.pad) pad.append(button(state.labels.clear, () => update("pad", ""), mod.busy || waiting));
      card.append(line, pad);
    }
    if (mod.id === "instant-suite") {
      const options = element("div", "d-flex gap-3 flex-wrap");
      options.append(toggle(mod.labels.private, mod.private, value => update("private", value)),
        toggle(mod.labels.tournaments, mod.tournaments, value => update("tournaments", value)));
      const mode = element("label", "d-flex align-items-center gap-2");
      mode.append(element("span", "", mod.labels.mode), loadoutSelect(mod.labels.mode, mod.modes, mod.mode,
        value => update("mode", value)));
      card.append(options, mode);
      if (mod.maps) {
        if (!mod.maps.enabled) card.append(element("p", "text-warning small mt-2", mod.labels.needsMenu));
        const map = element("div", "d-flex gap-2 align-items-center mt-2");
        map.append(element("span", "", mod.labels.map),
          loadoutSelect(mod.labels.map, [["", mod.labels.pick], ...mod.maps.items], mod.map, value => {
            if (value) update("map", value);
          }), button(mod.labels.reload, () => update("reload")));
        card.append(map);
      }
    }
    if (mod.busy || mod.status) card.append(element("p", "small text-body-secondary mt-2 mb-0", mod.status || "Working…"));
    window.patchProgress(card, !!mod.busy, mod.progress, mod.status || mod.label);
    rows.append(card);
  }
}

let multiplayerSearch = "";
let ownSlotsOpen = false;
let blockedPlayersOpen = false;
function multiplayerIcon() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 64 48");
  svg.setAttribute("aria-hidden", "true");
  svg.classList.add("multiplayer-icon");
  for (const [tag, attributes] of [
    ["circle", {cx: 43, cy: 14, r: 8, opacity: .55}],
    ["path", {d: "M29 40a14 14 0 0 1 28 0Z", opacity: .55}],
    ["circle", {cx: 24, cy: 11, r: 8}],
    ["path", {d: "M10 37a14 14 0 0 1 28 0Z"}],
  ]) {
    const node = document.createElementNS(svg.namespaceURI, tag);
    for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
    svg.append(node);
  }
  return svg;
}

function multiplayer() {
  const action = (type, data = {}) => send({ type: "multiplayer", action: { type, ...data } });
  const { connection, options, labels } = state;
  const help = button(labels.help, () => action("help"));
  const preferences = element("div", "d-grid mt-2");
  const preference = (key, label) => toggle(label, options[key], enabled => action("preference", { key, enabled }));
  preferences.append(preference("close", "Disconnect when Hebnix closes"), preference("toasts", labels.toasts));
  if (!connection.started) {
    const intro = panel();
    intro.classList.add("multiplayer-intro", "multiplayer-centered");
    const privacy = preference("relay", labels.relay);
    privacy.title = labels.relayHint;
    intro.append(multiplayerIcon(), element("h2", "h4 mb-3", "Workshop Multiplayer"), element("p", "text-body-secondary", labels.intro), privacy,
      button(labels.connect, () => action("connect"), connection.stopping, "primary"), " ", help, preferences);
    if (connection.status) intro.append(element("p", "small", connection.status));
    rows.append(intro);
    return;
  }
  const layout = element("div", "multiplayer-layout");
  const setup = panel();
  setup.classList.add("multiplayer-centered");
  setup.append(element("h2", "h6", labels.setup), element("p", "", connection.setup));
  if (connection.checking) setup.append(element("p", "text-body-secondary small", labels.checking));
  if (connection.close) setup.append(button(labels.closeGame, () => action("close_game")));
  if (connection.host) setup.append(element("p", "small", labels.password));
  if (connection.detected) setup.append(element("p", "small", connection.detected));
  const sync = button("Sync Map Slots", () => action("sync"), !connection.ready);
  sync.title = "Prepare your map slots now and load the maps other players are hosting.";
  const setupActions = element("div", "d-grid gap-2");
  setupActions.append(sync, button(labels.stop, () => action("disconnect")), help);
  setup.append(setupActions, preferences);

  const players = panel();
  const heading = element("header", "multiplayer-heading multiplayer-centered");
  heading.append(multiplayerIcon());
  players.append(heading);
  heading.append(element("h2", "h5", !connection.ready ? connection.setup : state.rows.length ? state.heading : labels.empty));
  if (connection.ready && !state.rows.length) heading.append(element("p", "text-body-secondary", "Invite another Hebnix player to connect to Workshop Multiplayer."));
  const list = element("div", "multiplayer-players");
  const filter = value => {
    list.replaceChildren();
    const term = value.trim().toLowerCase();
    const visible = state.rows.filter(peer => peer.name.toLowerCase().includes(term));
    for (const peer of visible) {
      const card = element("article", "tracker-detail p-3 multiplayer-centered");
      const heading = element("div", "d-flex justify-content-center flex-wrap gap-2");
      heading.append(element("strong", "", peer.name));
      if (peer.latency !== null) heading.append(element("span", "small", Math.round(peer.latency) + " ms"));
      card.append(heading);
      if (peer.map) {
        const line = element("div", "d-flex justify-content-center flex-wrap align-items-center gap-2");
        line.append(element("span", "", peer.map.label));
        if (peer.map.have) line.append(element("span", "text-success", labels.have));
        else line.append(button(labels.download, () => action("download", { peer: peer.peer, map: peer.map.id }), state.busy));
        card.append(line);
      }
      if (peer.peer) card.append(button(labels.block, () => action("block", { peer: peer.peer })));
      list.append(card);
    }
    if (!visible.length && state.rows.length) list.append(element("p", "text-body-secondary small", labels.noMatch));
  };
  if (state.search) {
    const search = element("input", "form-control form-control-sm mb-2");
    search.type = "search";
    search.value = multiplayerSearch;
    search.placeholder = labels.search;
    search.setAttribute("aria-label", labels.search);
    search.addEventListener("input", () => { multiplayerSearch = search.value; filter(search.value); });
    players.append(search);
  } else multiplayerSearch = "";
  filter(multiplayerSearch);
  players.append(list);
  if (state.own.length) {
    const slots = element("details", "mt-2");
    slots.open = ownSlotsOpen;
    slots.addEventListener("toggle", () => { ownSlotsOpen = slots.open; });
    slots.append(element("summary", "", "Your map slots"));
    for (const slot of state.own) {
      const row = element("p", "small mb-1", slot.label);
      const status = element("span", slot.issues.length ? "text-warning" : "text-success", " · " + slot.status);
      status.title = slot.issues.join("\n");
      if (slot.status) row.append(status);
      slots.append(row);
    }
    players.append(slots);
  }
  if (state.blocked.length) {
    const blocked = element("details", "mt-2");
    blocked.open = blockedPlayersOpen;
    blocked.addEventListener("toggle", () => { blockedPlayersOpen = blocked.open; });
    blocked.append(element("summary", "", state.blocked.length + " blocked players"),
      element("p", "small text-body-secondary", labels.blockHint));
    for (const [peer, name] of state.blocked) {
      const row = element("div", "d-flex align-items-center gap-2");
      row.append(element("span", "", name), button(labels.unblock, () => action("unblock", { peer })));
      blocked.append(row);
    }
    players.append(blocked);
  }
  if (state.progress) {
    const bar = element("progress", "w-100");
    bar.max = state.progress.total;
    bar.value = state.progress.done;
    const message = "Downloading from a player: " + Math.floor(bar.value / 1e6) + " / " + Math.floor(bar.max / 1e6) + " MB";
    bar.setAttribute("aria-label", message);
    players.append(element("p", "mb-1 small", message), bar);
  } else if (state.transfer) players.append(element("p", "small", state.transfer));
  if (connection.status || connection.progress) players.append(element("p", "small", connection.progress || connection.status));
  layout.append(setup, players);
  rows.append(layout);

  if (state.warning) {
    const dialog = element("dialog", "tracker-detail loadout-dialog");
    const title = element("h2", "h5", "Install from another player?");
    title.id = "peer-install-title";
    dialog.setAttribute("aria-labelledby", title.id);
    const choose = accept => dismissDialog(dialog, () => action("confirm", { accept }));
    dialog.append(title, element("p", "", state.warning), element("p", "", labels.warning),
      button(labels.cancel, () => choose(false)), " ", button(labels.confirm, () => choose(true), false, "primary"));
    openLoadoutModal(dialog, () => choose(false));
  }
}

let menuMapSearch = "";
let carPanelOpen = false;
let cameraPanelOpen = false;
function menuMap() {
  const { labels, maps, car, camera } = state;
  const call = (type, data = {}) => send({ type: "menu_map", action: { type, ...data } });
  const layout = element("div", "menu-map-layout");
  const main = element("div", "d-grid gap-2");
  const selection = panel();
  selection.append(element("p", "", labels.description), element("p", "small text-body-secondary", labels.close));
  if (state.loading) selection.append(element("p", "small text-body-secondary", labels.loading));
  selection.append(button(labels.arenas, () => call("source", { workshop: false }), !maps.editable,
    maps.workshop ? "outline-secondary" : "primary"), " ",
    button(labels.workshop, () => call("source", { workshop: true }), !maps.editable,
      maps.workshop ? "primary" : "outline-secondary"));
  const search = element("input", "form-control form-control-sm mt-2");
  search.type = "search";
  search.value = menuMapSearch;
  search.placeholder = labels.search;
  search.setAttribute("aria-label", labels.search);
  search.disabled = !maps.editable;
  const options = element("div", "d-grid gap-1 my-2 menu-map-options");
  const update = () => {
    const term = menuMapSearch.trim().toLowerCase();
    options.replaceChildren(...maps.choices.filter(([, name]) => name.toLowerCase().includes(term)).map(([id, name]) =>
      button(name, () => call("select", { id }), !maps.editable, id === maps.selected ? "primary" : "outline-secondary")));
  };
  search.addEventListener("input", () => { menuMapSearch = search.value; update(); });
  update();
  selection.append(search, options,
    button(labels.apply, () => call("apply"), !maps.editable || !maps.selected, "primary"), " ",
    button(labels.restore, () => call("restore"), !state.ready || !maps.active));
  if (maps.active) selection.append(element("p", "mt-2 mb-0", maps.active), element("p", "small text-body-secondary", labels.restoreFirst));
  if (maps.workshop || !state.loading && !maps.choices.length) {
    selection.append(element("p", "small text-body-secondary mt-2 mb-0", maps.workshop
      ? maps.choices.length ? labels.workshopNote : labels.workshopEmpty : labels.empty));
  }
  main.append(selection);

  const details = (title, description, open, onToggle) => {
    const panel = element("details", "tracker-detail p-3");
    panel.open = open;
    panel.append(element("summary", "fw-semibold", title), element("p", "small text-body-secondary mt-2", description));
    panel.addEventListener("toggle", () => onToggle(panel.open));
    return panel;
  };
  const numeric = (title, value, sendValue, disabled, min, max, step) => {
    const row = element("label", "appearance-row");
    const input = element("input", "form-control form-control-sm w-auto");
    input.type = "number";
    input.value = String(value);
    input.step = step;
    if (min !== undefined) input.min = String(min);
    if (max !== undefined) input.max = String(max);
    input.disabled = disabled;
    input.addEventListener("change", () => { if (input.value !== "") sendValue(Number(input.value)); });
    row.append(element("span", "", title), input);
    return row;
  };
  const offsets = (section, tweak, change, disabled) => {
    ["X", "Y", "Z"].forEach((axis, index) => section.append(numeric(labels.move + " " + axis, tweak.offset[index], value => {
      const offset = [...tweak.offset];
      offset[index] = value;
      change({ ...tweak, offset });
    }, disabled, undefined, undefined, "any")));
  };
  const position = coords => coords.map((value, index) => ["X", "Y", "Z"][index] + " " + value.toFixed(0)).join("  ");
  const carSection = details(labels.carTitle, labels.carDescription, carPanelOpen, value => { carPanelOpen = value; });
  if (!car) carSection.append(element("p", "text-body-secondary", labels.carNone));
  else {
    const change = tweak => call("car", { tweak });
    const disabled = !car.on || !state.ready;
    carSection.append(toggle(labels.carEnable, car.on, enabled => call("car_toggle", { enabled })),
      element("p", "small text-body-secondary", labels.original + ": " + position(car.position) + " · " + labels.yaw + " " + car.yaw.toFixed(1) + "°"));
    offsets(carSection, car.tweak, change, disabled);
    carSection.append(numeric(labels.yaw, car.tweak.yaw, yaw => change({ ...car.tweak, yaw }), disabled, -180, 180, ".5"),
      button(labels.reset, () => change({ offset: [0, 0, 0], yaw: 0 }), disabled));
  }
  main.append(carSection);

  const cameras = details(labels.cameraTitle, labels.cameraDescription, cameraPanelOpen, value => { cameraPanelOpen = value; });
  if (camera.error || !camera.rows.length) cameras.append(element("p", "text-body-secondary", camera.error || labels.cameraNone));
  else {
    const disabled = !camera.on || !state.ready;
    cameras.append(toggle(labels.cameraEnable, camera.on, enabled => call("camera_toggle", { enabled })));
    const play = camera.rows.find(item => item.play);
    const others = camera.rows.filter(item => !item.play);
    const selected = camera.rows.find(item => item.id === camera.selected);
    const onPlay = selected?.play;
    const cameraSelect = id => call("camera_select", { id });
    if (play) cameras.append(button(labels.play, () => cameraSelect(play.id), disabled, onPlay ? "primary" : "outline-secondary"), " ");
    cameras.append(button(labels.other, () => cameraSelect(camera.other || others[0].id), disabled || !others.length,
      onPlay ? "outline-secondary" : "primary"));
    if (!onPlay && others.length) {
      const dropdown = loadoutSelect(labels.other, others.map(item => [item.id, item.label]), camera.selected, cameraSelect);
      dropdown.disabled = disabled;
      cameras.append(dropdown);
    }
    if (selected) {
      const defaults = { camera: selected.id, offset: [0, 0, 0], pitch: 0, yaw: 0, fov: null };
      const tweak = camera.tweak || defaults;
      const change = values => call("camera", { tweak: { ...tweak, ...values } });
      cameras.append(element("p", "small text-body-secondary", labels.original + ": " + position(selected.position) +
        " · " + labels.pitch + " " + selected.rotation[0].toFixed(1) + "° · " + labels.yaw + " " + selected.rotation[1].toFixed(1) + "°"));
      offsets(cameras, tweak, change, disabled);
      cameras.append(numeric(labels.pitch, tweak.pitch, pitch => change({ pitch }), disabled, -89, 89, ".5"),
        numeric(labels.yaw, tweak.yaw, yaw => change({ yaw }), disabled, -180, 180, ".5"));
      if (selected.fov !== null) cameras.append(toggle(labels.fov, tweak.fov !== null,
        enabled => change({ fov: enabled ? selected.fov : null }), disabled),
        numeric(labels.fov, tweak.fov ?? selected.fov, fov => change({ fov }), disabled || tweak.fov === null, 10, 170, ".5"));
      cameras.append(button(labels.reset, () => change(defaults), disabled), element("p", "small text-body-secondary mb-0", labels.each));
    }
  }
  main.append(cameras);
  const actions = panel();
  if (maps.active) {
    const reapply = button(labels.reapply, () => call("reapply"), !maps.canReapply);
    reapply.title = labels.reapplyHint;
    actions.append(reapply, " ");
  }
  actions.append(button(labels.scan, () => call("scan"), state.busy || state.loading));
  if (state.status) actions.append(element("p", "small mt-2 mb-0", state.status));
  main.append(actions);

  const plugin = panel();
  const importButton = button(labels.import, () => call("import"), state.busy);
  importButton.title = labels.importHint;
  plugin.append(element("h2", "h6", labels.pluginTitle), element("p", "text-body-secondary", labels.pluginDescription),
    button(labels.plugin, () => call("plugin")), " ", importButton);
  layout.append(main, plugin);
  rows.append(layout);
}

function workshopMenu() {
  const sendMenu = (type, data = {}) => send({ type: "workshop_menu", action: { type, ...data } });
  const columns = element("div", "freeplay-columns");
  const left = element("div", "d-grid gap-2");
  const right = panel();
  if (state.freeplay) rows.append(element("p", "small text-body-secondary",
    "The active practice session uses the Freeplay tab. LAN settings below apply to Workshop and LAN matches."));
  if (!state.lanOnly) {
    const menu = panel();
    menu.append(element("h2", "h6", "In-game Workshop menu"),
      element("p", "small text-body-secondary", "Adds Workshop to Rocket League’s offline Play menu. Close Rocket League before changing it."),
      toggle("Enable in-game menu", state.enabled === true, enabled => sendMenu("enable", { enabled }), state.busy || state.enabled === null),
      button("Update Workshop patch", () => sendMenu("update"), state.busy || state.enabled !== true));
    if (state.error) menu.append(element("p", "text-warning", state.error));
    if (state.busy || state.status) menu.append(element("p", "small mt-2 mb-0", state.status || "Working…"));
    window.patchProgress(menu, !!state.busy, state.progress, state.status || "Updating Workshop in-game menu");
    const downloader = panel();
    downloader.append(element("h2", "h6", "Map downloader"),
      toggle("Enable map downloader", state.downloader, enabled => sendMenu("downloader", { enabled }), state.busy || state.enabled !== true),
      element("p", "small text-body-secondary mb-0", "With the downloader enabled, press L3 in the Workshop menu to browse maps."));
    const maps = panel();
    maps.append(element("h2", "h6", "Local maps"), element("p", "small text-break", state.folder),
      button("Rescan", () => sendMenu("rescan")), element("p", "small mt-2 mb-0", state.count + " maps found."));
    left.append(menu, downloader, maps);
  }
  const match = panel();
  match.append(element("h2", "h6", "LAN match"),
    element("p", "small text-body-secondary", "Used when you host a map from the in-game Workshop menu."));
  const selectRow = (label, choices, value, change) => {
    const row = element("label", "freeplay-setting");
    row.append(element("span", "", label), loadoutSelect(label, choices, value, pick => change(Number(pick))));
    return row;
  };
  match.append(selectRow("Team size", [1, 2, 3, 4].map(size => [size, size + "v" + size]), state.team,
    value => sendMenu("team_size", { value })),
    selectRow("Bots", state.botChoices.map((bot, index) => [index, bot]), state.bots, value => sendMenu("bots", { value })));
  for (const status of state.offers) match.append(element("p", "small mb-1", "Peer map: " + status));
  const access = panel();
  const name = element("input", "form-control form-control-sm mb-2");
  const password = element("input", "form-control form-control-sm mb-2");
  name.placeholder = "Game default";
  name.setAttribute("aria-label", "Match name");
  password.placeholder = "No password";
  password.setAttribute("aria-label", "Password");
  password.type = "password";
  name.maxLength = password.maxLength = 32;
  name.value = state.name;
  password.value = state.password;
  let custom = state.custom;
  name.disabled = password.disabled = !custom;
  access.append(element("h2", "h6", "LAN match access"), toggle("Use custom LAN credentials", custom, checked => {
    custom = checked;
    name.disabled = password.disabled = !checked;
  }), name, password, button("Save LAN match access", () => sendMenu("access", { custom, name: name.value, password: password.value })));
  left.append(match, access);
  right.append(element("h2", "h6", "LAN Mutators"),
    element("p", "small text-body-secondary", "These settings are used when hosting a LAN match. Heatseeker needs two Soccar goals."));
  if (state.lanOnly && state.status) right.append(element("p", "small", state.status));
  for (const { label, options, pick, index } of state.rows) {
    right.append(selectRow(label, options.map((name, index) => [index, name]), pick,
      value => sendMenu("mutator", { index, value })));
  }
  if (state.extra) right.append(element("p", "small text-body-secondary", "Other tags kept: " + state.extra));
  right.append(button("Reset to defaults", () => sendMenu("reset")));
  columns.append(left, right);
  rows.append(columns);
}

function freeplay() {
  const setup = panel();
  const ball = panel();
  const rules = panel();
  const columns = element("div", "freeplay-columns");
  setup.append(toggle("Enable Freeplay Mutators", state.checked,
    enabled => send({ type: "freeplay_toggle", enabled }), !state.canToggle));
  if (state.patchMissing) setup.append(element("p", "text-warning",
    "TAGame.upk no longer contains the current Freeplay patch. Close Rocket League and enable Freeplay Mutators again."));
  ball.append(element("h2", "h6 mb-3", "Ball"));
  rules.append(element("h2", "h6 mb-3", "Cars and Rules"));
  const panels = { mode: setup, ball, rules };
  const settings = [...state.rows].sort((a, b) => a.label.localeCompare(b.label));
  for (const setting of settings) {
    const row = element("label", "freeplay-setting");
    const select = element("select", "form-select form-select-sm");
    setting.options.forEach((name, index) => select.append(new Option(name, index)));
    select.value = String(setting.pick);
    select.disabled = !state.enabled;
    select.addEventListener("change", () => send({
      type: "freeplay_pick", index: setting.index, value: Number(select.value),
    }));
    row.append(element("span", "", setting.label), select);
    panels[setting.group].append(row);
  }
  if (state.heatseeker) setup.append(element("p", "small text-body-secondary",
    "Use a Soccar arena and pick a team when the Heatseeker practice lobby opens."));
  if (state.lanSession) setup.append(element("p", "small text-body-secondary",
    "This session uses Maps > In-game menu controls."));
  setup.append(button("Reset to defaults", () => send({ type: "freeplay_reset" }), !state.enabled));
  if (state.status) setup.append(element("p", "small text-body-secondary mt-2 mb-0", state.status));
  columns.append(ball, rules);
  rows.append(setup, columns);
}

const rankView = { draft: null };
function playerRank() {
  const card = panel();
  card.append(element("p", "mb-1", "Choose the rank, division and MMR shown locally in Rocket League."),
    element("p", "small text-body-secondary", "Close Rocket League before applying. Untick a playlist and apply to restore its display."));
  const grid = element("div", "rank-playlist-grid");
  for (const playlist of state.rows) {
    const tile = element("div", "rank-playlist-tile" + (playlist.enabled ? " is-enabled" : ""));
    const choose = button("", () => { rankView.draft = { ...playlist }; render(); }, state.busy);
    choose.className = "rank-playlist-open";
    choose.setAttribute("aria-label", "Edit " + playlist.name + " rank");
    const icon = element("img", "rank-playlist-icon");
    icon.src = `/rank-icons/playlist-${playlist.icon}.png`; icon.alt = "";
    choose.append(icon, element("span", "rank-playlist-name", playlist.name));
    const details = element("span", "rank-playlist-details");
    const division = playlist.rank > 0 && playlist.rank < 22
      ? ["I", "II", "III", "IV"][playlist.division] : "—";
    for (const [label, value] of [["Rank", state.ranks[playlist.rank] || "—"], ["Div", division], ["MMR", playlist.mmr || "—"]]) {
      const row = element("span", "rank-playlist-detail");
      row.append(element("span", "text-body-secondary", label), element("span", "rank-playlist-value", value));
      details.append(row);
    }
    choose.append(details);
    const enabled = element("input", "form-check-input rank-playlist-enable");
    enabled.type = "checkbox"; enabled.checked = playlist.enabled; enabled.disabled = state.busy;
    enabled.setAttribute("aria-label", "Enable " + playlist.name + " rank override");
    enabled.addEventListener("change", () => send({ type: "player_rank", id: playlist.id, enabled: enabled.checked }));
    tile.append(choose, enabled); grid.append(tile);
  }
  card.append(grid);
  const actions = element("div", "d-flex justify-content-center gap-2 mt-3");
  actions.append(button(state.apply, () => send({ type: "player_rank_apply", restore: false }), !state.ready, "primary"),
    button(state.restore, () => send({ type: "player_rank_apply", restore: true }), !state.ready));
  card.append(actions);
  if (state.status) card.append(element("p", "small mt-2 mb-0", state.status));
  window.patchProgress(card, !!state.busy, state.progress, state.status || "Updating ranks");
  rows.append(card);
  if (rankView.draft) rankDialog();
}
function rankDialog() {
  const draft = rankView.draft;
  const dialog = element("dialog", "tracker-detail loadout-dialog rank-picker-dialog");
  const close = () => dismissDialog(dialog, () => { rankView.draft = null; dialog.remove(); });
  const heading = element("h2", "h5", draft.name); heading.id = "rank-picker-title";
  dialog.setAttribute("aria-labelledby", heading.id);
  const header = element("div", "d-flex align-items-center justify-content-between mb-3");
  const dismiss = button("×", close); dismiss.setAttribute("aria-label", "Close rank picker"); header.append(heading, dismiss);
  const grid = element("div", "rank-tier-grid");
  const rankButtons = [];
  const division = loadoutSelect("Division", ["Division I", "Division II", "Division III", "Division IV"].map((name, id) => [id, name]), draft.division,
    value => { draft.division = Number(value); });
  const mmr = element("input", "form-control form-control-sm");
  mmr.type = "number"; mmr.min = "100"; mmr.max = "9999"; mmr.step = "1"; mmr.value = draft.mmr;
  mmr.setAttribute("aria-label", "Custom MMR"); mmr.addEventListener("input", () => { draft.mmr = mmr.value; });
  const recommendation = element("p", "small text-body-secondary mb-0");
  const update = () => {
    rankButtons.forEach((tile, id) => { tile.classList.toggle("is-selected", draft.rank === id); tile.setAttribute("aria-pressed", String(draft.rank === id)); });
    division.disabled = draft.rank === 0 || draft.rank === 22;
    recommendation.textContent = "Recommended MMR: " + state.rankDefaults[draft.rank] + ". You can enter a custom value.";
  };
  state.ranks.forEach((name, id) => {
    const tile = button("", () => {
      draft.rank = id; draft.mmr = String(state.rankDefaults[id]); mmr.value = draft.mmr;
      if (id === 0 || id === 22) { draft.division = 0; division.value = "0"; }
      update();
    });
    tile.className = "rank-tier-tile";
    const image = element("img", ""); image.src = `/rank-icons/tier-${id}.png`; image.alt = "";
    tile.append(image, element("span", "", name)); rankButtons.push(tile); grid.append(tile);
  });
  const fields = element("div", "rank-picker-fields mt-3");
  const divisionLabel = element("label", "", "Division"); divisionLabel.append(division);
  const mmrLabel = element("label", "", "MMR"); mmrLabel.append(mmr); fields.append(divisionLabel, mmrLabel);
  const save = button("Save", () => {
    if (!mmr.reportValidity()) return;
    dismissDialog(dialog, () => {
      rankView.draft = null; dialog.remove();
      send({ type: "player_rank", id: draft.id, rank: draft.rank, division: draft.division, mmr: String(draft.mmr) });
    });
  }, state.busy, "primary");
  const footer = element("div", "d-flex justify-content-end gap-2 mt-3"); footer.append(button("Cancel", close), save);
  dialog.append(header, grid, fields, recommendation, footer); update(); openLoadoutModal(dialog, close);
}

const titleSearch = { library: "", target: "" };

function titles() {
  const action = (type, data = {}) => send({ type: "titles", action: { type, ...data } });
  const top = panel();
  if (state.inventory.error) top.append(element("p", "text-warning small", state.inventory.error));
  if (state.busy) top.append(element("p", "small text-body-secondary", "Title patch operation in progress. Editing will resume when it finishes."));
  if (top.childElementCount) rows.append(top);

  const flatten = value => String(value || "").replace(/^#/, "");
  const colour = rgb => rgb.map(byte => byte.toString(16).padStart(2, "0")).join("");
  const colorCss = value => /^[a-f\d]{6}$/i.test(flatten(value)) ? "#" + flatten(value) : "";
  function titleArt(value, textColour, glow) {
    const preview = element("span", "title-art");
    preview.style.color = colorCss(textColour);
    if (colorCss(glow)) preview.style.textShadow = "0 0 .6rem " + colorCss(glow);
    for (const part of value.split(/(\{[^{}]+\})/g)) {
      if (Object.hasOwn(state.icons, part)) {
        const icon = element("img", "title-symbol");
        icon.src = state.icons[part];
        icon.alt = part;
        preview.append(icon);
      } else preview.append(part);
    }
    return preview;
  }
  const search = (part, label, update) => {
    const input = element("input", "form-control form-control-sm mb-2");
    input.type = "search";
    input.value = titleSearch[part];
    input.placeholder = label;
    input.setAttribute("aria-label", label);
    input.addEventListener("input", () => {
      titleSearch[part] = input.value;
      update(input.value.trim().toLowerCase());
    });
    return input;
  };
  const columns = element("div", "title-editor-columns");
  const library = panel();
  const list = element("div", "title-editor-list");
  const showLibrary = query => {
    list.replaceChildren();
    const matches = state.catalog.filter(item => (!state.owned || item.owned) &&
      (item.text.toLowerCase().includes(query) || item.id.toLowerCase().includes(query)));
    for (const item of matches) {
      const line = element("div", "title-list-row");
      line.append(titleArt(item.text, item.color, item.glow),
        button("Copy", () => action("copy", { id: item.id }), state.busy));
      list.append(line);
    }
    if (!matches.length && !state.loading) list.append(element("p", "small text-body-secondary", "No matching titles."));
  };
  library.append(element("h2", "h6", "Copy Title"), element("p", "small text-body-secondary", "Browse titles and copy one to edit."),
    search("library", "Search titles", showLibrary),
    loadoutSelect("Title ownership", [["owned", "Owned Items"], ["all", "All Titles"]],
      state.owned ? "owned" : "all", value => action("owned", {enabled: value === "owned"})), list);
  if (state.catalogError) library.append(element("p", "text-warning small", state.catalogError),
    button("Refresh title library", () => action("reload"), state.loading));
  if (state.loading) library.append(element("p", "small text-body-secondary", "Loading title library…"));
  showLibrary(titleSearch.library.trim().toLowerCase());

  const editor = panel();
  const draft = state.draft;
  const text = element("input", "form-control form-control-sm mb-2");
  text.value = draft.text;
  text.disabled = state.busy;
  text.setAttribute("aria-label", "Custom title");
  const length = element("span", "small text-body-secondary", [...text.value].length + " / 64");
  const art = element("div", "title-editor-preview");
  const refreshPreview = () => {
    length.textContent = [...text.value].length + " / 64";
    art.replaceChildren(titleArt(text.value, colour(draft.color), draft.glow ? colour(draft.glow_color) : null));
  };
  text.addEventListener("input", () => {
    if ([...text.value].length > 64) text.value = [...text.value].slice(0, 64).join("");
    refreshPreview();
  });
  text.addEventListener("change", () => action("text", { text: text.value }));
  const paint = (name, glow) => {
    const row = element("label", "d-flex align-items-center gap-2");
    const input = element("input", "form-control form-control-color");
    input.type = "color";
    input.setAttribute("aria-label", glow ? "Glow colour" : "Text colour");
    input.value = "#" + colour(glow ? draft.glow_color : draft.color);
    input.disabled = state.busy || glow && !draft.glow;
    input.addEventListener("change", () => action("colour", {
      glow, rgb: input.value.slice(1).match(/.{2}/g).map(value => parseInt(value, 16)),
    }));
    row.append(element("span", "", name), input);
    return row;
  };
  const colours = element("div", "title-colour-controls mb-2");
  const tokens = Object.keys(state.icons).sort();
  const symbols = loadoutSelect("Add Symbol", [["", "Add Symbol"], ...tokens.map(token => [token, token])], "", token => {
    if (token && [...text.value + token].length <= 64) action("text", { text: text.value + token });
  });
  symbols.disabled = state.busy || !tokens.length;
  const textPaint = paint("Text colour", false);
  const glowPaint = paint("", true);
  const glow = toggle("Glow", draft.glow, enabled => action("glow", {enabled}), state.busy);
  glow.classList.remove("mb-2");
  colours.append(...textPaint.children, symbols, glow, glowPaint.lastElementChild);
  const targets = element("div", "title-target-options");
  const target = element("details", "mb-2");
  const current = state.catalog.find(item => item.id === draft.target);
  target.append(element("summary", "btn btn-sm btn-outline-secondary", current?.text || draft.target || (state.targetsOwned ? "Choose an owned title" : "All titles (fallback)")));
  const showTargets = query => {
    targets.replaceChildren();
    if (!state.targetsOwned) targets.append(button("All titles (fallback)", () => action("target"), state.busy));
    for (const item of state.catalog) {
      if (state.targetsOwned && !item.owned || !item.text.toLowerCase().includes(query)) continue;
      const choice = button("", () => action("target", { id: item.id }), state.busy,
        item.id === draft.target ? "primary" : "outline-secondary");
      choice.append(titleArt(item.text, item.color, item.glow));
      targets.append(choice);
    }
  };
  target.addEventListener("toggle", () => { if (target.open) showTargets(titleSearch.target.trim().toLowerCase()); });
  target.append(search("target", "Search replacement titles", showTargets), targets);
  const commit = button(state.editing ? "Edit Title" : "Add Title", () => {
    if (text.value !== draft.text) action("text", { text: text.value });
    action("save");
  }, false, "primary");
  const blocked = () => { commit.disabled = state.busy || !text.value.trim() || state.targetsOwned && !current?.owned || !state.editing && state.rules.length >= 64; };
  blocked();
  text.addEventListener("input", blocked);
  refreshPreview();
  editor.append(element("h2", "h6", "Edit Title"), element("span", "", "Custom title"), " ", length,
    text, colours, art, element("p", "mt-2 mb-1", "Title to replace"), target, commit);

  const applied = panel();
  const records = element("div", "title-editor-list");
  state.rules.forEach(({ title, target, warning }, index) => {
    const row = element("div", "border-bottom pb-2");
    row.append(titleArt(title.text, title.color, title.glow ? title.glow_color || title.color : null),
      element("p", "small text-body-secondary mb-1", target),
      button("Edit", () => action("edit", { index }), state.busy), " ",
      button("Remove", () => action("remove", { index }), state.busy, "outline-danger"));
    if (warning) row.append(element("p", "text-warning small mb-0", warning));
    records.append(row);
  });
  if (!state.rules.length) records.append(element("p", "small text-body-secondary", "No title replacements added yet"));
  const actions = element("div", "d-flex gap-2 mt-2");
  actions.append(button("Apply", () => action("apply"), state.busy || !state.ready || !!state.validation, "primary"),
    button("Restore", () => action("restore", { key: "open" }), state.busy || !state.ready));
  applied.append(element("h2", "h6", "Applied Titles"),
    element("p", "small text-body-secondary", "Apply your list to update this client."), records, actions);
  if (state.applied !== null) applied.append(element("p", state.applied ? "text-success small" : "text-warning small",
    state.applied ? "Applied" : "Changes pending — apply to update"));
  if (state.status || !state.ready || state.rules.length && state.validation) {
    applied.append(element("p", "small", state.status || (!state.ready ? "Close Rocket League to Apply or Restore." : state.validation)));
  }
  if (state.admin) applied.append(button("Restart as administrator for title patching", () => action("admin")));
  window.patchProgress(applied, !!state.busy, state.progress, state.status || "Updating titles");
  columns.append(library, editor, applied);
  rows.append(columns);
  if (state.restore) {
    const dialog = element("dialog", "tracker-detail loadout-dialog");
    const heading = element("h2", "h5", "Restore title changes?");
    heading.id = "restore-titles-title";
    dialog.setAttribute("aria-labelledby", heading.id);
    const dismiss = key => dismissDialog(dialog, () => action("restore", { key }));
    dialog.append(heading, element("p", "", "Restore the original title display on this client? Your replacement list will be kept."),
      button("Cancel", () => dismiss("cancel")), " ",
      button("Restore titles", () => dismiss("confirm"), state.busy || !state.ready, "danger"));
    openLoadoutModal(dialog, () => dismiss("cancel"));
  }
}

function profilePictures() {
  const intro = element("p", "", "Choose a picture to share with other Hebnix players.");
  const hint = element("p", "small text-body-secondary mb-2", "Click a picture under Recent pictures to upload it as your avatar.");
  const columns = element("div", "live-columns");
  const account = panel();
  const recent = panel();
  const set = (key, value = null) => send({ type: "profile_picture", key, value });
  account.append(toggle("Show player profile pictures", state.enabled, value => set("enabled", value), !state.canToggle));
  if (state.accounts.length) {
    const options = state.account === null ? [["", "Waiting for Rocket League…"], ...state.accounts] : state.accounts;
    const select = loadoutSelect("Rocket League account", options, state.account, value => set("account", value));
    select.disabled = state.running || state.busy;
    account.append(select);
  } else {
    account.append(element("p", "small", state.owner || "Open Rocket League to upload"));
  }
  if (state.update) account.append(button("Update avatar support", () => set("update", true), !state.canToggle));
  if (state.retry) account.append(button("Retry connection", () => set("retry")));
  if (state.current) {
    const picture = element("img", "profile-current");
    picture.src = state.current;
    picture.alt = "Current picture";
    account.append(element("h2", "h6 mt-3", "Current picture"), picture);
  }
  recent.append(element("h2", "h6", "Recent pictures"), button("Import picture…", () => set("import"), state.busy));
  const gallery = element("div", "profile-gallery mt-3");
  for (const entry of state.rows) {
    const tile = element("div", "profile-tile");
    const active = entry.id === state.active;
    const upload = button("", () => set("upload", entry.id), state.busy || !state.owner, active ? "primary" : "outline-secondary");
    upload.setAttribute("aria-label", active ? "Upload current picture again" : "Upload picture");
    const picture = element("img", "");
    picture.src = entry.image;
    picture.alt = "";
    upload.append(picture);
    tile.append(upload, icon("×", "Remove from recent pictures", () => set("remove", entry.id), state.busy));
    gallery.append(tile);
  }
  recent.append(gallery);
  columns.append(account, recent);
  rows.append(intro, hint, columns);
  if (state.status || state.busy) rows.append(element("p", state.error ? "text-warning" : "text-body-secondary", state.status || "Working…"));
}

const loadoutView = { car: null, page: 0, search: "", draft: null, modalKey: "", name: "", previews: "", openSlot: null, itemSearch: "",
  importOpen: false, importSubmitted: false, palette: null };

function loadoutAction(type, data = {}) {
  send({ type: "loadouts", action: { type, ...data } });
}

function loadoutSelect(label, options, current, change) {
  const select = element("select", "form-select form-select-sm w-auto");
  select.setAttribute("aria-label", label);
  options.forEach(([id, name]) => select.append(new Option(name, id)));
  select.value = String(current ?? "");
  select.addEventListener("change", () => change(select.value));
  return select;
}

function loadoutPreview(url, product) {
  if (product === 0) {
    const empty = element("span", "loadout-preview loadout-empty-preview", "×");
    empty.setAttribute("role", "img");
    empty.setAttribute("aria-label", "No item selected");
    return empty;
  }
  const image = element("img", "loadout-preview");
  image.src = url || "/tracker-logo.png";
  image.alt = "";
  return image;
}

function loadoutImport(card, labels) {
  if (!state.closed) card.append(element("p", "text-warning small", labels.closed));
  if (state.account === null) card.append(element("p", "text-warning small", "Select a Rocket League save from the settings cog before importing."));
  if (!state.import) {
    card.append(element("p", "text-body-secondary small", labels.hint));
    const form = element("form", "input-group");
    const input = element("input", "form-control");
    input.value = loadoutView.draft ?? state.input;
    input.placeholder = "Paste a code here";
    input.setAttribute("aria-label", "Loadout code");
    input.addEventListener("input", () => { loadoutView.draft = input.value; });
    const submit = element("button", "btn btn-primary", "Import");
    submit.type = "submit";
    submit.disabled = !state.closed || state.account === null;
    form.addEventListener("submit", event => {
      event.preventDefault();
      if (!submit.disabled) {
        loadoutView.draft = null;
        loadoutView.importSubmitted = true;
        loadoutAction("import", { code: input.value });
      }
    });
    form.append(input, submit);
    card.append(form);
    return;
  }
  const plan = state.import;
  card.append(element("p", "mb-2", plan.summary));
  if (!state.rows.length) {
    card.append(element("p", "text-body-secondary", labels.empty));
    return;
  }
  const options = state.rows.map(row => [row.position, row.name]);
  const update = change => {
    const { base, target, replace, name } = plan;
    loadoutAction("plan", { base, target, replace, name, ...change });
  };
  const base = element("label", "d-flex align-items-center gap-2 mb-2");
  base.append(labels.base, loadoutSelect(labels.base, options, plan.base, value => update({ base: Number(value) })));
  const mode = element("div", "d-flex align-items-center gap-2 mb-2");
  mode.append(button(labels.new, () => update({ replace: false }), false, plan.replace ? "outline-secondary" : "primary"),
    button(labels.replace, () => update({ replace: true }), false, plan.replace ? "primary" : "outline-secondary"));
  const target = element("label", "d-flex align-items-center gap-2 mb-2");
  if (plan.replace) {
    target.append(labels.target, loadoutSelect(labels.target, options, plan.target, value => update({ target: Number(value) })));
  } else {
    const name = element("input", "form-control form-control-sm w-auto");
    name.value = plan.name;
    name.addEventListener("change", () => update({ name: name.value }));
    target.append(labels.name, name);
  }
  card.append(base, element("p", "small text-body-secondary", labels.baseHint), mode, target);
  if (plan.preview?.rows) {
    const table = element("table", "table table-sm");
    const head = element("thead", "");
    const heading = element("tr", "");
    heading.append(...labels.plan.map(label => element("th", "", label)));
    head.append(heading);
    const body = element("tbody", "");
    for (const cells of plan.preview.rows) {
      const row = element("tr", "");
      row.append(...cells.map(label => element("td", "", label)));
      body.append(row);
    }
    table.append(head, body);
    card.append(element("strong", "", plan.preview.name), table, element("p", "small text-body-secondary", plan.preview.note));
  } else if (plan.preview?.error) {
    card.append(element("p", "text-warning", plan.preview.error));
  }
  const actions = element("div", "d-flex flex-wrap gap-2");
  const blocked = !state.closed || !plan.preview?.rows;
  const save = apply => {
    loadoutView.importSubmitted = true;
    loadoutAction("save_import", { apply });
  };
  actions.append(button(labels.apply, () => save(true), blocked, "primary"),
    button(labels.importOnly, () => save(false), blocked));
  card.append(actions);
}

function dismissDialog(dialog, complete) {
  if (dialog.classList.contains("is-closing")) return;
  dialog.classList.add("is-closing");
  const animation = dialog.getAnimations().find(animation => animation.animationName === "popup-close");
  if (animation) animation.finished.then(complete, complete);
  else complete();
}

function closeLoadoutImport(dialog) {
  dismissDialog(dialog, () => {
    loadoutView.importOpen = false;
    loadoutView.importSubmitted = false;
    if (state.import) loadoutAction("cancel_import");
    render();
  });
}

function openLoadoutModal(dialog, dismiss) {
  dialog.addEventListener("cancel", event => { event.preventDefault(); dismiss(); });
  dialog.addEventListener("click", event => {
    if (event.target !== dialog) return;
    const box = dialog.getBoundingClientRect();
    if (event.clientX < box.left || event.clientX > box.right || event.clientY < box.top || event.clientY > box.bottom) dismiss();
  });
  rows.append(dialog);
  dialog.showModal();
}

function loadoutDialog(labels) {
  const model = state.dialog;
  if (!model) {
    loadoutView.modalKey = "";
    return;
  }
  const dialog = element("dialog", "tracker-detail loadout-dialog");
  dialog.setAttribute("aria-labelledby", "loadout-modal-title");
  const heading = element("h2", "h5", labels[model.kind]);
  heading.id = "loadout-modal-title";
  dialog.append(heading);
  const actions = element("div", "d-flex gap-2 justify-content-end mt-3");
  const cancel = () => dismissDialog(dialog, () => loadoutAction("close"));
  actions.append(button(labels.cancel, cancel));
  const commit = data => dismissDialog(dialog, () => loadoutAction("confirm", data));
  let confirm = () => commit({});
  let submitLabel = labels.confirm;
  const key = model.kind + ":" + model.position;
  if (model.kind === "delete") {
    dialog.append(element("p", "", model.message));
    submitLabel = labels.delete;
  } else if (model.kind === "move") {
    const destinations = state.rows.map(row => [row.position, (row.position + 1) + " · " + row.name]);
    const position = loadoutSelect(labels.position, destinations, model.target, () => {});
    dialog.append(element("p", "", model.name), element("p", "small mb-1", labels.position), position);
    confirm = () => commit({ destination: Number(position.value) });
  } else {
    if (key !== loadoutView.modalKey) {
      loadoutView.name = model.name;
      loadoutView.openSlot = null;
      loadoutView.itemSearch = "";
    }
    const name = element("input", "form-control mb-3");
    name.value = loadoutView.name;
    name.setAttribute("aria-label", labels.name);
    name.addEventListener("input", () => { loadoutView.name = name.value; });
    dialog.append(name, loadoutSelect("Team", [[0, labels.blue], [1, labels.orange]], model.team,
      value => loadoutAction("team", { team: Number(value) })));
    confirm = () => { if (name.value.trim()) commit({ name: name.value }); else name.focus(); };
    if (model.palette.loading) dialog.append(element("p", "small", "Loading colours…"));
    if (model.palette.error) dialog.append(element("p", "text-warning", model.palette.error));
    const colours = element("div", "d-flex gap-2 mt-3");
    for (const colourKey of ["primary", "secondary"]) {
      if (!model.palette[colourKey]) continue;
      const choose = button(labels[colourKey], () => {
        loadoutView.palette = colourKey;
        loadoutColourDialog(model, labels);
      });
      const swatch = element("span", "loadout-current-colour");
      swatch.style.backgroundColor = "rgb(" + (model.palette[colourKey][model[colourKey]] || [0,0,0]).join(",") + ")";
      choose.prepend(swatch); colours.append(choose);
    }
    dialog.append(colours);
    dialog.append(element("p", "small text-body-secondary mt-3", labels.owned));
    for (const slot of model.slots) {
      const list = element("details", "loadout-slot mb-2");
      list.open = loadoutView.openSlot === slot.slot;
      const summary = element("summary", "", slot.name + " · " + slot.current);
      summary.prepend(loadoutPreview(slot.image, slot.product));
      list.append(summary);
      const search = element("input", "form-control form-control-sm my-2");
      search.placeholder = labels.itemSearch;
      if (list.open) search.value = loadoutView.itemSearch;
      search.setAttribute("aria-label", labels.itemSearch + " · " + slot.name);
      const choices = element("div", "loadout-choices");
      const filter = () => {
        const term = search.value.trim().toLowerCase();
        const owned=slot.choices.map(([label,option,selected])=>({label,selected,choose:()=>loadoutAction("item",{team:model.team,slot:slot.slot,option})}));
        for(const engine of slot.engines || []) if(!owned.some(item=>item.label===engine.label)) owned.push({label:engine.label,selected:slot.product===engine.product,choose:()=>loadoutAction("stock_engine",{team:model.team,product:engine.product})});
        owned.sort((a,b)=>a.label.localeCompare(b.label));
        choices.replaceChildren(...owned.filter(item=>item.label.toLowerCase().includes(term)).map(item=>button(item.label,item.choose,false,item.selected?"primary":"outline-secondary")));
      };
      search.addEventListener("input", () => {
        loadoutView.itemSearch = search.value;
        filter();
      });
      list.addEventListener("toggle", () => {
        if (list.open) {
          if (loadoutView.openSlot !== slot.slot) loadoutView.itemSearch = search.value;
          loadoutView.openSlot = slot.slot;
          filter();
        } else if (loadoutView.openSlot === slot.slot) {
          loadoutView.openSlot = null;
        }
      });
      list.append(search, choices);
      dialog.append(list);
    }
  }
  loadoutView.modalKey = key;
  actions.append(button(submitLabel, confirm, !state.closed, model.kind === "delete" ? "danger" : "primary"));
  dialog.append(actions);
  openLoadoutModal(dialog, cancel);
  if (loadoutView.palette && model.kind === "edit") loadoutColourDialog(model, labels);
}

function loadoutColourDialog(model, labels, select) {
  const key = loadoutView.palette;
  if (!key || !model.palette[key]) return;
  const dialog = element("dialog", "tracker-detail loadout-dialog loadout-colour-dialog");
  dialog.classList.toggle("secondary-palette", key === "secondary");
  dialog.style.setProperty("--palette-columns", key === "secondary" ? 15 : 10);
  const close = () => dismissDialog(dialog, () => { loadoutView.palette = null; dialog.remove(); });
  const header = element("div", "d-flex align-items-center justify-content-between gap-3 mb-3");
  header.append(element("h2", "h5 mb-0", labels[key]), button("×", close));
  const palette = element("div", "loadout-palette");
  model.palette[key].forEach(([r,g,b], index) => {
    const colour = icon("", labels[key] + " " + (index + 1), () => dismissDialog(dialog, () => {
      loadoutView.palette = null; dialog.remove();
      if (select) select(key, index);
      else loadoutAction("colour", { team: model.team, primary: key === "primary", index });
    }));
    colour.style.backgroundColor = `rgb(${r},${g},${b})`;
    colour.setAttribute("aria-pressed", String(index === model[key]));
    palette.append(colour);
  });
  // any colour: the app appends it to the garage palette and picks that cell
  const current = model.palette[key][model[key]] || [255, 255, 255];
  const custom = element("div", "d-flex align-items-center gap-2 mt-3");
  const picker = element("input", "form-control form-control-color");
  picker.type = "color";
  picker.value = "#" + current.map(value => value.toString(16).padStart(2, "0")).join("");
  picker.setAttribute("aria-label", "Custom " + labels[key]);
  custom.append(element("span", "", "Any colour"), picker, button("Use colour", () => dismissDialog(dialog, () => {
    loadoutView.palette = null; dialog.remove();
    const rgb = picker.value.slice(1).match(/.{2}/g).map(value => parseInt(value, 16));
    loadoutAction("custom_colour", { team: model.team, primary: key === "primary", rgb });
  }), !state.closed, "primary"));
  const note = element("p", "small text-body-secondary mt-2", "A custom colour is added to your garage palette, so Rocket League must be closed. Restart it to see the colour. For rainbow or live colours, use the Colours page.");
  dialog.append(header, palette, custom, note, button("Close", close));
  openLoadoutModal(dialog, close);
}

const creatorView = { step: 0, name: "", search: "", copy: false, pending: null, replacement: "", requested: "", initialized: false, itemPage: 0 };
function creatorAction(type, data = {}) { loadoutAction("creator", { action: { type, ...data } }); }
function presetCreator() {
  const view = creatorView, draft = state.creator;
  view.resize?.disconnect();
  document.body.classList.add("preset-creator-page");
  const root = element("section", "preset-creator");
  if (!draft) {
    const start = element("div", "preset-creator-start");

    if (view.createdUntil > Date.now()) start.append(element("p", "text-success creator-created", "Loadout Created"));
    else if (state.message && state.message !== "Loadout Created") start.append(element("p", "text-warning", state.message));
    const begin = copy => { view.step = 0; view.name = ""; view.initialized = false; view.pending = null; view.search = ""; creatorAction("begin", { copy }); };
    const options = element("div", "preset-creator-start-actions");
    options.append(button("Create New Preset", () => begin(null), !state.rows.length || !state.closed, "primary"),
      button("Copy Existing Loadout", () => { view.copy = true; render(); }, !state.rows.length || !state.closed));
    start.append(options);
    if (!state.closed) start.append(element("p","small text-warning mt-2 mb-0","Rocket League must be closed."));
    if (!state.rows.length) start.append(element("p", "small text-body-secondary", "Select an account with a Rocket League loadout from quick settings first."));
    if (view.copy && state.rows.length) {
      const choose = loadoutSelect("Loadout to copy", state.rows.map(row => [row.position, row.name]), state.rows[0].position, () => {});
      const copy = element("div", "d-flex align-items-center justify-content-center gap-2 mt-3");
      copy.append(choose, button("Continue", () => { view.copy = false; begin(Number(choose.value)); }, !state.closed, "primary"), button("Cancel", () => { view.copy = false; render(); })); start.append(copy);
    }
    root.append(start); rows.append(root); return;
  }
  if (!view.initialized) { view.name = draft.name || ""; view.initialized = true; }
  const steps = [{name:"Name"},{name:"Body",slot:0},{name:"Decal",slot:1},{name:"Colours",colours:true},
    {name:"Wheels",slot:2},{name:"Boost",slot:3},{name:"Antenna",slot:4},{name:"Topper",slot:5},
    {name:"Trail",slot:14},{name:"Goal Explosion",slot:15},{name:"Engine Audio",slot:13},{name:"Review"}];
  view.step = Math.min(view.step, steps.length - 1);
  const step = steps[view.step];
  const navigate = index => { if (!view.name.trim() && index > 0) { document.getElementById("creator-name")?.focus(); return; } view.step = index; view.search = ""; view.itemPage = 0; view.pending = null; view.requested = ""; if (steps[index].name === "Review") creatorAction("review"); if (steps[index].slot !== undefined) creatorAction("slot", {slot:steps[index].slot}); render(); };
  const rail = element("nav", "tracker-detail preset-creator-steps");
  steps.forEach((step,index) => rail.append(button(`${index+1}. ${step.name}`, () => navigate(index), false, index === view.step ? "primary" : "outline-secondary")));
  const content = panel(); content.classList.add("preset-creator-content");
  const heading = element("div", "d-flex align-items-center justify-content-between gap-3 mb-3");
  heading.append(element("h2", "h5 mb-0", step.name));

  content.append(heading);
  if (state.message && state.message !== "Loadout Created") content.append(element("p", "text-warning", state.message));
  if (view.step === 0) {
    const name = element("input", "form-control"); name.id="creator-name"; name.maxLength=64; name.placeholder="Name your preset"; name.value=view.name;
    name.setAttribute("aria-label","Preset name"); name.addEventListener("input",()=>{view.name=name.value;});
    content.append(element("p", "text-body-secondary", "Give this preset a name. Your original loadout stays unchanged."), name);
  } else if (step.colours) {
    if (draft.palette.loading) content.append(element("p","","Loading colours…"));
    if (draft.palette.error) content.append(element("p","text-warning",draft.palette.error));
    const actions=element("div","preset-creator-colours");
    for (const team of draft.colours || []) {
      const group=element("section","creator-team-colours");
      group.append(element("h3","h6",team.team===0?"Blue team":"Orange team"));
      for (const key of ["primary","secondary"]) if (team.palette?.[key]) {
        const pick=button(key === "primary" ? "Primary Colour" : "Secondary Colour",()=>{loadoutView.palette=key;loadoutColourDialog(team,state.labels,(key,index)=>creatorAction("colour",{team:team.team,primary:key==="primary",index}));});
        const swatch=element("span","loadout-current-colour");swatch.style.backgroundColor=`rgb(${(team.palette[key][team[key]]||[0,0,0]).join(",")})`;pick.prepend(swatch);group.append(pick);
      }
      actions.append(group);
    }
    content.append(actions);
  } else if (step.slot !== undefined) {
    const selected=draft.slots.find(slot=>slot.slot===step.slot);
    if (step.slot === 1) {
      const body = draft.slots.find(slot => slot.slot === 0);
      if (body?.product) content.append(element("p", "text-body-secondary", "Choose a decal for " + body.label + "."));
    }
    content.append(element("p","text-body-secondary", "Selected: " + (selected?.label || "Default")));
    const search=element("input","form-control form-control-sm mb-3"); search.id="creator-search";search.type="search";search.placeholder="Search all items";search.value=view.search;
    const grid=element("div","preset-creator-items");
    const pages=element("nav","d-flex justify-content-center gap-2 align-items-center my-3");
    const draw=()=>{
      if(draft.slot !== step.slot) { grid.textContent="Loading items…"; return; }
      const items=(draft.catalog.items||[]).filter(item=>item.label.toLowerCase().includes(view.search.trim().toLowerCase()));
      grid.replaceChildren();
      const optional = ![0,2,3,14,15].includes(step.slot);
      const defaultTile = optional ? button(step.slot===13?"Default":"None",()=>creatorAction("pick",{product:0,replacement:null})) : null;
      if(defaultTile) {if(step.slot!==13){const empty=element("span","creator-empty-icon","×");empty.setAttribute("aria-hidden","true");defaultTile.prepend(empty);}defaultTile.className="swapper-tile"+(!selected?.product?" is-selected":"");defaultTile.setAttribute("aria-pressed",String(!selected?.product));grid.append(defaultTile);}
      const columns=Math.max(1,Math.floor((grid.clientWidth+10)/160));
      grid.style.setProperty("--creator-columns",columns);
      const pageSize=Math.max(1,columns*3-(defaultTile?1:0));
      const pageCount=Math.max(1,Math.ceil(items.length/pageSize));view.itemPage=Math.min(view.itemPage,pageCount-1);
      const visible=items.slice(view.itemPage*pageSize,(view.itemPage+1)*pageSize);
      pages.replaceChildren(button("Previous",()=>{view.itemPage--;draw();},view.itemPage===0),element("span","",`${view.itemPage+1} / ${pageCount}`),button("Next",()=>{view.itemPage++;draw();},view.itemPage+1>=pageCount));
      const ids=visible.map(item=>item.id), signature=JSON.stringify([step.slot,ids]);
      if(view.requested!==signature || ids.some(id=>!draft.preview_ids.includes(id))) {view.requested=signature;creatorAction("previews",{ids});}
      for(const item of visible){
        const tile=button("",()=>{if(item.owned)creatorAction("pick",{product:item.product,replacement:null});else{view.pending=item;view.replacement="";creatorReplacement(draft);}},false, selected?.product===item.product?"primary":"outline-secondary");
        tile.className="swapper-tile"+(selected?.product===item.product?" is-selected":"");
        tile.setAttribute("aria-pressed",String(selected?.product===item.product));
        const art=element("span","swapper-tile-art"), preview=draft.catalog.previews?.[item.id];
        if(preview?.image){const img=element("img","");img.src=preview.image;img.alt="";art.append(img);}else art.textContent=preview?.status||"";
        tile.append(art,element("span","swapper-tile-caption",item.label),element("small","text-body-secondary",item.owned?"Owned":"Swap required"));grid.append(tile);
      }
    };
    search.addEventListener("input",()=>{view.search=search.value;view.itemPage=0;draw();});content.append(search,grid,pages);
    requestAnimationFrame(()=>{if(grid.isConnected){draw();view.resize=new ResizeObserver(()=>{const cols=Math.max(1,Math.floor((grid.clientWidth+10)/160));if(grid.style.getPropertyValue("--creator-columns")!==String(cols))draw();});view.resize.observe(grid);}});
  } else {
    content.append(element("h3","h4",view.name),element("p","text-body-secondary","Review each team's selections before creating your preset. Owned replacement items will also change wherever they are used."));
    for(const slot of draft.slots)content.append(element("p","mb-1",slot.name+": "+slot.label));
    content.append(element("h3","h6 mt-4","Affected loadouts"));
    if(!draft.affected?.length)content.append(element("p","text-body-secondary","No existing loadouts use these replacement items."));
    for(const loadout of draft.affected || []) {
      const card=element("section","tracker-detail p-3 mb-2");card.append(element("h4","h6",loadout.name));
      for(const reason of loadout.changes)card.append(element("p","small mb-1",reason));content.append(card);
    }
  }
  const footer=element("footer","preset-creator-footer");
  footer.append(button("Cancel",()=>{view.copy=false;creatorAction("cancel");}),button("← Back",()=>navigate(view.step-1),view.step===0));
  if(view.step+1<steps.length)footer.append(button("Next →",()=>navigate(view.step+1),false,"primary"));
  else {
    const missing = draft.missing_required;
    footer.append(button("Create Preset",()=>{view.creating=true;view.attempt=state.creatorAttempt;render();requestAnimationFrame(()=>requestAnimationFrame(()=>creatorAction("finish",{name:view.name})));},!state.closed || missing.length > 0,"primary"));
    if (missing.length) footer.append(element("span", "text-warning small", "Required: " + missing.join(", ")));
  }
  if(!state.closed)footer.append(element("span","text-warning small","Close Rocket League before creating the preset."));
  content.append(footer);root.append(rail,content);rows.append(root);
  if(view.creating){root.inert=true;const busy=element("div","creator-busy");busy.setAttribute("role","status");busy.setAttribute("aria-live","polite");busy.append(element("span","spinner-border text-primary"),element("strong","","Creating loadout…"),element("span","small text-body-secondary","Applying item swaps and saving your loadout."));rows.append(busy);}
  if(view.pending)creatorReplacement(draft);
}
function creatorReplacement(draft){
  const view=creatorView,item=view.pending;if(!item)return;
  const dialog=element("dialog","tracker-detail loadout-dialog");
  const close=()=>dismissDialog(dialog,()=>{view.pending=null;dialog.remove();});
  const body = draft.slots.find(slot => slot.slot === 0);
  const hint = draft.slot === 1 && body?.equipped
    ? "Choose an owned decal compatible with " + (body.equipped_label || "the body you'll equip in Rocket League") + "."
    : "Choose the owned item you'll equip in Rocket League.";
  dialog.append(element("h2","h5","Choose an owned item to replace"),
    element("p","",hint + " It will appear as " + item.label + "."),
    element("p","small text-warning","This also changes other loadouts that use the same owned item."));
  const choose=loadoutSelect("Owned replacement",[["","Choose an owned item"],...draft.replacements.map(item=>[item.index,item.label])],view.replacement,value=>{view.replacement=value;apply.disabled=value==="";});
  const apply=button("Use Replacement",()=>{if(choose.value==="")return;dismissDialog(dialog,()=>{view.pending=null;dialog.remove();creatorAction("pick",{product:item.product,replacement:Number(choose.value)});});},view.replacement==="","primary");
  dialog.append(choose,element("p","small text-body-secondary mt-2",draft.replacements.length?"":"No compatible owned replacements are available."),button("Cancel",close),apply);openLoadoutModal(dialog,close);
}

function loadouts() {
  const labels = state.labels;
  if (state.catalogLoading) controls.append(element("span", "text-body-secondary small", "Loading item catalogs…"));
  if (state.catalogError) controls.append(element("span", "text-warning small", state.catalogError),
    button("Reload catalogs", () => send({ type: "loadouts_reload_catalogs" })));
  const save = panel();

  if (!state.accounts.length) save.append(element("p", "small text-body-secondary mt-2 mb-0", labels.noSave));
  if (!loadoutView.importOpen && state.message) {
    const warning=element("div","d-flex justify-content-between align-items-start gap-3");
    warning.setAttribute("role","status");
    warning.append(element("p","small mb-0",state.message),icon("×","Dismiss warning",()=>loadoutAction("dismiss_message")));
    save.append(warning);
  }
  const layout = element("div", "loadout-layout");
  const filters = panel();
  filters.classList.add("loadout-car-panel");
  filters.append(element("h2", "h6", labels.cars));
  const counts = new Map();
  for (const row of state.rows) {
    const item = counts.get(row.body) || { car: row.car, count: 0 };
    item.count++;
    counts.set(row.body, item);
  }
  const content = panel();
  const heading = element("div", "d-flex justify-content-between align-items-center gap-2 mb-3");
  const search = element("input", "form-control form-control-sm w-auto");
  search.type = "search";
  search.value = loadoutView.search;
  search.placeholder = labels.search;
  search.setAttribute("aria-label", labels.search);
  heading.append(element("h2", "h6 mb-0", labels.list), search,
    pluginAction("zip", "Import", () => { loadoutView.importOpen = true; render(); }));
  const grid = element("div", "loadout-grid");
  const paging = element("nav", "d-flex gap-2 justify-content-center align-items-center mt-3");
  const update = () => {
    const term = loadoutView.search.toLowerCase().trim();
    const matching = state.rows.filter(row => (loadoutView.car === null || row.body === loadoutView.car) && row.name.toLowerCase().includes(term));
    const columns = Math.max(1, Math.floor((grid.clientWidth + 10) / 195));
    grid.style.setProperty("--loadout-columns", columns);
    const pageSize = columns * 3;
    const pages = Math.max(1, Math.ceil(matching.length / pageSize));
    loadoutView.page = Math.min(pages - 1, loadoutView.page);
    grid.replaceChildren();
    const visible = matching.slice(loadoutView.page * pageSize, (loadoutView.page + 1) * pageSize);
    const positions = visible.map(row => row.position);
    const previewKey = JSON.stringify(positions);
    if (loadoutView.previews !== previewKey || positions.some(position => !(state.preview_positions || []).includes(position))) {
      loadoutView.previews = previewKey;
      loadoutAction("previews", { positions });
    }
    for (const row of visible) {
      const tile = element("article", "tracker-detail loadout-tile");
      const copy = element("details", "loadout-copy");
      const summary = element("summary", "btn btn-sm btn-outline-secondary ui-icon-button", "⧉");
      summary.title = labels.copy;
      summary.setAttribute("aria-label", labels.copy);
      const menu = element("div", "loadout-copy-menu");
      menu.append(button(labels.lds, () => { copy.open = false; loadoutAction("copy", { position: row.position, bakkes: false }); }),
        button(labels.bakkes, () => { copy.open = false; loadoutAction("copy", { position: row.position, bakkes: true }); }));
      copy.append(summary, menu);
      const actions = element("div", "d-flex gap-1");
      actions.append(copy);
      for (const [kind, symbol] of [["edit", "✎"], ["move", "↕"]]) {
        actions.append(icon(symbol, labels[kind], () => loadoutAction("open", { position: row.position, kind }),
          !state.closed));
      }
      const remove = icon("×", labels.delete, () => loadoutAction("open", { position: row.position, kind: "delete" }),
        !state.closed, "outline-danger");
      remove.classList.add("loadout-delete");
      const info = element("div", "loadout-tile-info");
      info.append(element("strong", "", row.name), element("small", "text-body-secondary", row.car), actions);
      tile.append(loadoutPreview(row.image, row.body), info, remove);
      grid.append(tile);
    }
    paging.hidden = pages < 2;
    paging.replaceChildren(button("Previous", () => { loadoutView.page--; update(); }, loadoutView.page === 0),
      element("span", "", (loadoutView.page + 1) + " / " + pages),
      button("Next", () => { loadoutView.page++; update(); }, loadoutView.page + 1 >= pages));
  };
  const choices = element("div", "d-grid gap-1 loadout-car-list");
  const filter = (body, name) => {
    const option = button(name, () => { loadoutView.car = body; loadoutView.page = 0; render(); },
      false, loadoutView.car === body ? "primary" : "outline-secondary");
    choices.append(option);
  };
  filter(null, "All (" + state.rows.length + ")");
  [...counts].sort(([, a], [, b]) => a.car.localeCompare(b.car)).forEach(([body, item]) => filter(body, item.car + " (" + item.count + ")"));
  search.addEventListener("input", () => { loadoutView.search = search.value; loadoutView.page = 0; update(); });
  filters.append(choices);
  content.append(heading, grid, paging);
  layout.append(filters, content);
  if (save.childElementCount) rows.append(save);
  rows.append(layout);
  loadoutView.resize?.disconnect();
  loadoutView.resize = new ResizeObserver(update);
  loadoutView.resize.observe(grid);
  loadoutView.carResize?.disconnect();
  loadoutView.carResize = new ResizeObserver(() => { filters.style.height = content.offsetHeight + "px"; });
  loadoutView.carResize.observe(content);
  update();
  if (loadoutView.importOpen) {
    const dialog = element("dialog", "tracker-detail loadout-dialog");
    dialog.setAttribute("aria-labelledby", "import-loadout-title");
    const heading = element("h2", "h5", labels.import);
    heading.id = "import-loadout-title";
    const header = element("div", "d-flex align-items-center justify-content-between gap-3 mb-3");
    heading.classList.add("mb-0");
    const close = icon("×", "Close import", () => closeLoadoutImport(dialog));
    header.append(heading, close);
    dialog.append(header);
    loadoutImport(dialog, labels);
    if (state.message) dialog.append(element("p", "small mt-2 mb-0", state.message));
    openLoadoutModal(dialog, () => closeLoadoutImport(dialog));
  } else {
    loadoutDialog(labels);
  }
}
function studioActive() {
  const header = panel();
  const actions = element("div", "d-flex align-items-center justify-content-between gap-2");
  const restore = (kind, target) => send({ type: "studio_restore", kind, target });
  actions.append(element("span", "", state.count),
    button(state.all, () => restore(), !state.rows.length, "outline-warning"));
  header.append(actions);
  rows.append(header);
  if (!state.rows.length) {
    rows.append(element("p", "text-body-secondary", state.empty));
    return;
  }
  const grid = element("div", "plugin-catalog-grid");
  for (const patch of state.rows) {
    const card = element("article", "tracker-detail plugin-card");
    const art = element("div", "studio-active-art");
    if (!patch.images.length) art.append(element("span", "text-body-secondary", state.missing));
    for (const source of patch.images) {
      const image = element("img", "studio-active-" + patch.kind);
      image.src = source;
      image.alt = "";
      art.append(image);
    }
    const info = element("div", "plugin-card-body text-center");
    info.append(element("h2", "h6 mb-1", patch.name), element("p", "small text-body-secondary", patch.description),
      button(state.restore, () => restore(patch.kind, patch.target)));
    card.append(art, info);
    grid.append(card);
  }
  rows.append(grid);
}

function studioPage() {
  const content = element('div', 'd-grid gap-2');
  rows.append(content);
  return content;
}

const studioAssetView = { key: "", search: "", page: 0, requested: "", applied: false, downloaded: false, previews: new Map() };
const historyView = { query: "", expanded: new Set() };
const artworkView = { key: "", query: "", page: 0, owned: false, patched: false, names: false, requested: "", draft: null };
function itemArtwork() {
  const content = studioPage();
  const view = artworkView;
  const names = state.kind === "names";
  const { labels } = state;
  const cmd = (key, value = null) => send({ type: "item_artwork", key, value });
  const key = [state.kind, state.category, state.language, state.owned, state.patched].join(":");
  if (key !== view.key) Object.assign(view, { key, page: 0, requested: "", draft: null });
  const unavailable = state.busy || !state.closed;
  const head = panel();
  const controls = element("div", "d-flex flex-wrap align-items-center gap-2");
  if (names) controls.append(loadoutSelect("Item language", state.languages.map(file => [file, file.replace(/^Coalesced_|[.]bin$/g, "")]),
    state.language, file => cmd("language", file)));
  controls.append(loadoutSelect("Category", state.categories, state.category, category => cmd("category", category)),
    button("Refresh", () => cmd("refresh"), state.busy),
    button(labels.restoreAll, () => cmd("restore-all"), unavailable || !state.active, "outline-danger"));
  head.append(element("p", "small text-body-secondary mb-2", labels.description), controls);
  if (state.error || state.status || state.busy) head.append(element("p",
    "small mb-0 mt-2 " + (state.error ? "text-danger" : "text-body-secondary"),
    state.error || (state.busy ? "Working…" : state.status)));
  const filters = element("div", "d-flex flex-wrap gap-3 mt-2");
  view.owned = !!state.globalOwnedOnly;
  if (!names && state.owned !== state.globalOwnedOnly) cmd("owned", !!state.globalOwnedOnly);
  for (const filter of ["patched"]) {
    filters.append(toggle(labels[filter], names ? view[filter] : state[filter], value => {
      if (names) {
        view[filter] = value;
        view.page = 0;
        cmd("cancel");
        draw();
      } else cmd(filter, value);
    }, state.busy));
  }
  if (names) filters.append(toggle(labels.showNames, view.names, value => { view.names = value; draw(); }));
  head.append(filters);
  content.append(head);
  function preview(parent, image, missing = "No preview") {
    if (image) {
      const img = element("img", "object-fit-contain");
      img.src = image;
      img.alt = "";
      parent.append(img);
    } else parent.append(element("span", "text-body-secondary small", missing));
  }
  const selected = state.selection;
  if (selected) {
    const editor = panel();
    editor.append(element("h2", "h6", selected.original || selected.name));
    const buttons = element("div", "d-flex flex-wrap gap-2");
    if (names) {
      const draftId = JSON.stringify(selected);
      if (view.draft?.id !== draftId) view.draft = { id: draftId, name: selected.name, artist: selected.artist };
      const save = button(labels.apply, () => cmd("save", { name: view.draft.name, artist: view.draft.artist }), unavailable || !view.draft.name.trim(), "primary");
      for (const field of ["name", "artist"]) {
        if (selected[field] === null) continue;
        const label = element("label", "d-block mb-2", labels[field]);
        const input = element("input", "form-control form-control-sm");
        input.maxLength = 128;
        input.value = view.draft[field];
        input.disabled = state.busy;
        input.addEventListener("input", () => {
          view.draft[field] = input.value;
          save.disabled = unavailable || !view.draft.name.trim();
        });
        label.append(input);
        editor.append(label);
      }
      buttons.append(save);
      if (selected.fields.length) buttons.append(button(labels.restore, () => cmd("restore"), unavailable));
      if (selected.artist !== null) {
        for (const [field, label] of [["Label", labels.restoreSong], ["Artist", labels.restoreArtist]]) {
          if (selected.fields.includes(field)) buttons.append(button(label, () => cmd("restore-field", field), unavailable));
        }
      }
    } else {
      const previews = element("div", "d-flex gap-3 mb-2");
      for (const [label, image] of [[labels.current, selected.image], [labels.replacement, selected.replacement]]) {
        if (label === labels.replacement && !image) continue;
        const frame = element("div", "studio-active-art");
        frame.style.width = "9rem";
        frame.setAttribute("aria-label", label);
        frame.title = label;
        preview(frame, image);
        previews.append(frame);
      }
      editor.append(previews);
      buttons.append(button(labels.choose, () => cmd("choose"), state.busy),
        button(labels.apply, () => cmd("save"), unavailable || !selected.image || !selected.replacement, "primary"));
      if (selected.export !== null) {
        const file = selected.id.slice(0, selected.id.lastIndexOf("|"));
        buttons.append(button(labels.restore, () => cmd("restore", file + "|" + selected.export), unavailable));
      }
    }
    buttons.append(button(labels.cancel, () => cmd("cancel"), state.busy));
    editor.append(buttons);
    content.append(editor);
  } else view.draft = null;
  const search = element("input", "form-control form-control-sm");
  search.type = "search";
  search.placeholder = labels.search;
  search.setAttribute("aria-label", labels.search);
  search.value = view.query;
  const activeThumbnails = !names && state.patched;
  const list = element("div", activeThumbnails ? "d-grid gap-2" : "plugin-catalog-grid studio-asset-grid");
  const paging = element("div", "d-flex align-items-center justify-content-center gap-2");
  content.append(search, list, paging);
  function draw() {
    const term = view.query.trim().toLowerCase();
    const items = state.rows.filter(item => (!names || (!view.owned || item.owned) && (!view.patched || item.patched)) &&
      [item.name, item.current, item.artist, item.currentArtist, item.file, item.id].filter(Boolean).join(" ").toLowerCase().includes(term));
    const columns = Math.max(1, Math.floor((list.clientWidth + 16) / 191));
    const size = activeThumbnails ? 36 : Math.min(36, columns * (names ? 3 : 2));
    const pages = Math.max(1, Math.ceil(items.length / size));
    view.page = Math.min(view.page, pages - 1);
    const visible = items.slice(view.page * size, (view.page + 1) * size);
    const ids = visible.map(item => item.id);
    const signature = JSON.stringify(ids);
    if (signature !== view.requested) {
      view.requested = signature;
      send({ type: "studio_asset_previews", ids });
    }
    list.replaceChildren();
    for (const item of visible) {
      const card = element("article", "tracker-detail " + (activeThumbnails ? "plugin-installed-row p-2" : "plugin-card"));
      const art = element("div", "studio-active-art");
      preview(art, item.image, item.error || "No preview");
      const body = element("div", "plugin-card-body");
      const title = element("h2", "h6 text-break", names && view.names ? item.current : item.name);
      title.title = item.file || item.id;
      body.append(title);
      const artist = view.names ? item.currentArtist : item.artist;
      if (artist) body.append(element("p", "small text-body-secondary mb-1", artist));
      if (item.patched && names) body.append(element("p", "small text-body-secondary mb-1", "Patched"));
      body.append(activeThumbnails ? button(labels.restore, () => cmd("restore", item.id), unavailable) :
        button(labels.edit, () => cmd("select", item.id), state.busy || (!names && !item.image)));
      card.append(art, body);
      list.append(card);
    }
    if (!visible.length) list.append(element("p", "small text-body-secondary", labels.empty));
    paging.hidden = pages < 2;
    paging.replaceChildren(button("Previous", () => { view.page--; draw(); }, view.page === 0),
      element("span", "", view.page + 1 + " / " + pages),
      button("Next", () => { view.page++; draw(); }, view.page + 1 >= pages));
  }
  search.addEventListener("input", () => { view.query = search.value; view.page = 0; draw(); });
  draw();
}

const anthemView = { query: "", page: 0, owned: false, patched: false, requested: "" };
function anthems() {
  const content = studioPage();
  const cmd = (key, id = null, value = null) => send({ type: "anthem", key, id, value });
  if (state.error || state.status) content.append(element("p", state.error ? "small text-danger mb-0" : "small text-body-secondary mb-0", state.error || state.status));
  const editor = state.editor;
  if (editor) {
    const card = panel();
    const header = element("div", "d-flex gap-3 align-items-center my-2");
    if (editor.image) {
      const art = element("img", "object-fit-contain");
      art.width = 85; art.height = 75; art.src = editor.image; art.alt = "";
      header.append(art);
    }
    const title = element("div", "");
    title.append(element("h2", "h5 mb-0", editor.name), element("p", "text-body-secondary small mb-0", editor.file));
    header.append(title);
    card.append(button("‹ All anthems", () => cmd("back"), state.busy), header,
      button("Import Audio", () => cmd("import"), state.busy),
      element("p", "small text-body-secondary mt-2 mb-1", editor.source || "Import audio to replace the anthem."));
    const playback = element("div", "d-flex flex-wrap gap-2 mt-2");
    if (editor.region) {
      card.append(element("p", "small", "All " + editor.regions + " regions · longest: " + editor.region + " (" + editor.length.toFixed(3) + " s)"));
      playback.append(button("Preview original " + editor.region, () => cmd("original"), state.busy || !editor.original));
      if (editor.previewType === "original") playback.append(button(editor.playing ? "Pause original" : "Resume original", () => cmd("original-toggle"), state.busy),
        button("Stop original", () => cmd("stop"), state.busy));
    } else {
      card.append(element("p", "text-danger small", editor.error || "Anthem clip timings could not be read."));
    }
    card.append(playback);
    if (editor.duration && editor.maxStart === null) card.append(element("p", "text-warning small mt-2", "The imported audio must cover the longest region."));
    if (editor.maxStart !== null) {
      const plot = element("canvas", "w-100 border rounded my-2");
      plot.width = 1250;
      plot.height = 140;
      const context = plot.getContext("2d");
      const draw = (start, cursor) => {
        const { width, height } = plot;
        const pixels = time => time / editor.duration * width;
        context.clearRect(0, 0, width, height);
        context.fillStyle = "#173754";
        context.fillRect(pixels(start), 0, pixels(editor.length), height);
        context.strokeStyle = "#83bbdf";
        context.beginPath();
        editor.peaks.forEach((peak, index) => {
          const x = index / editor.peaks.length * width;
          context.moveTo(x, height / 2 - peak * height / 2);
          context.lineTo(x, height / 2 + peak * height / 2);
        });
        context.stroke();
        context.fillStyle = "#d6efff";
        context.fillRect(pixels(cursor), 0, 2, height);
      };
      anthems.updateCursor = position => draw(editor.start, position);
      draw(editor.start, editor.position);
      const plotTime = event => {
        const rect = plot.getBoundingClientRect();
        return (event.clientX - rect.left) / rect.width * editor.duration;
      };
      let drag = null;
      plot.addEventListener("pointerdown", event => {
        if (state.busy || event.button !== 0) return;
        const time = plotTime(event);
        const threshold = 8 / plot.clientWidth * editor.duration;
        if (time >= editor.start - threshold && time <= editor.start + editor.length + threshold) {
          drag = { original: time, offset: time - editor.start, start: editor.start };
          plot.setPointerCapture(event.pointerId);
        } else cmd("seek", null, time);
      });
      plot.addEventListener("pointermove", event => {
        if (!drag) return;
        drag.start = Math.max(0, Math.min(editor.maxStart, plotTime(event) - drag.offset));
        draw(drag.start, drag.start);
      });
      plot.addEventListener("pointerup", event => {
        if (!drag) return;
        const time = plotTime(event);
        if (Math.abs(time - drag.original) > 0.002) cmd("start", null, drag.start);
        else cmd("seek", null, time);
        drag = null;
      });
      plot.addEventListener("pointercancel", () => { drag = null; draw(editor.start, editor.position); });
      card.append(plot);
      const commands = element("div", "d-flex flex-wrap align-items-center gap-2");
      const start = element("input", "form-control form-control-sm w-auto");
      start.type = "number"; start.min = 0; start.max = editor.maxStart; start.step = 0.001;
      start.value = editor.start.toFixed(3); start.disabled = state.busy;
      start.addEventListener("change", () => cmd("start", null, start.valueAsNumber));
      const startLabel = element("label", "d-flex align-items-center gap-2", "Start (s)");
      startLabel.append(start);
      commands.append(button(editor.playing && editor.previewType === "imported" ? "Pause" : "Play", () => cmd("play"), state.busy),
        button("Stop", () => cmd("stop"), state.busy), startLabel,
        element("span", "small text-body-secondary", "Stop " + (editor.start + editor.length).toFixed(3) + " s"));
      const volume = element("input", "form-range");
      volume.type = "range"; volume.min = 0; volume.max = 100; volume.value = Math.round(editor.volume * 100);
      volume.disabled = state.busy;
      volume.addEventListener("change", () => cmd("volume", null, volume.valueAsNumber / 100));
      const volumeLabel = element("label", "d-block mt-3", "Song volume");
      volumeLabel.append(volume);
      card.append(commands, volumeLabel, button("Save and patch anthem", () => cmd("save"), state.busy || !state.closed, "primary"));
    }
    content.append(card);
    return;
  }
  anthems.updateCursor = null;
  const controls = panel();
  anthemView.owned = !!state.globalOwnedOnly;
  const tools = element("div", "d-flex flex-wrap gap-2 align-items-center");
  tools.append(button("Refresh", () => cmd("refresh"), state.busy),
    button("Restore all anthems", () => cmd("restore-all"), state.busy || !state.closed || !state.active, "outline-danger"));
  const filter = element("input", "form-control form-control-sm");
  filter.type = "search";
  filter.value = anthemView.query;
  filter.placeholder = "Search anthems";
  filter.setAttribute("aria-label", filter.placeholder);
  const filters = element("div", "d-flex flex-wrap gap-3 mt-2");
  for (const [key, label] of [["patched", "Show patched anthems"]]) {
    filters.append(toggle(label, anthemView[key], value => { anthemView[key] = value; anthemView.page = 0; draw(); }));
  }
  const grid = element("div", "plugin-catalog-grid studio-asset-grid");
  const paging = element("div", "d-flex justify-content-center align-items-center gap-2");
  controls.append(tools, filters, filter);
  content.append(controls, grid, paging);
  function draw() {
    const term = anthemView.query.trim().toLowerCase();
    const items = state.rows.filter(item => (!anthemView.owned || item.owned) && (!anthemView.patched || item.active) &&
      (item.name + " " + item.file).toLowerCase().includes(term));
    const count = Math.min(36, Math.max(1, Math.floor((grid.clientWidth + 16) / 191)) * 3);
    const pages = Math.max(1, Math.ceil(items.length / count));
    anthemView.page = Math.min(anthemView.page, pages - 1);
    const visible = items.slice(anthemView.page * count, (anthemView.page + 1) * count);
    const ids = visible.map(item => item.id);
    const signature = JSON.stringify(ids);
    if (signature !== anthemView.requested) {
      anthemView.requested = signature;
      send({ type: "studio_asset_previews", ids });
    }
    grid.replaceChildren();
    for (const item of visible) {
      const card = element("article", "tracker-detail plugin-card");
      const art = element("div", "studio-active-art");
      if (item.image) {
        const img = element("img", ""); img.src = item.image; img.alt = "";
        art.append(img);
      } else art.append(element("span", "small text-body-secondary", "No preview"));
      const body = element("div", "plugin-card-body text-center");
      const title = element("h2", "h6 mb-2", item.name);
      title.title = item.file;
      body.append(title, button("Select", () => cmd("select", item.id), state.busy));
      if (item.patched) body.append(toggle("Patch", item.active, value => cmd("patch", item.id, value), state.busy || !state.closed));
      card.append(art, body);
      grid.append(card);
    }
    if (!items.length) grid.append(element("p", "small text-body-secondary", state.busy ? "Loading…" : "No installed anthems match this filter."));
    paging.hidden = pages < 2;
    paging.replaceChildren(button("Previous", () => { anthemView.page--; draw(); }, anthemView.page === 0),
      element("span", "", anthemView.page + 1 + " / " + pages),
      button("Next", () => { anthemView.page++; draw(); }, anthemView.page + 1 >= pages));
  }
  filter.addEventListener("input", () => { anthemView.query = filter.value; anthemView.page = 0; draw(); });
  draw();
}

function upkHistory() {
  const content = element("div", "d-grid gap-3");
  rows.append(content);
  const { labels } = state;
  const cmd = (key, index = null, value = null) => send({ type: "upk_history", key, index, value });
  const filter = element("input", "form-control form-control-sm");
  filter.type = "search";
  filter.placeholder = labels.search;
  filter.setAttribute("aria-label", labels.search);
  filter.value = historyView.query;
  const toolbar = element("div", "d-flex gap-2");
  toolbar.append(filter, button(labels.refresh, () => cmd("refresh"), state.busy));
  const controls = panel();
  controls.append(toolbar);
  if (state.running || state.busy || state.status) {
    controls.append(element("p", "small mb-0 mt-2", state.running ? labels.close : state.busy ? "Working…" : state.status));
  }
  const list = element("div", "d-grid gap-2");
  content.append(controls, list);
  const check = (title, id) => {
    const group = element("details", "mt-2");
    group.open = historyView.expanded.has(id);
    group.append(element("summary", "", title));
    group.addEventListener("toggle", () => {
      if (group.open) historyView.expanded.add(id);
      else historyView.expanded.delete(id);
    });
    return group;
  };
  function point(row) {
    const box = element("div", "border-top mt-2 pt-2");
    box.append(element("p", "small text-body-secondary mb-1", [row.file, row.date].filter(Boolean).join(" · ")),
      element("p", "mb-1", row.label));
    if (row.colour || row.alignment) box.append(element("p", "small text-body-secondary mb-2", [row.colour, row.alignment].filter(Boolean).join(" · ")));
    if (row.linked) return box;
    const disabled = state.busy || state.running || row.disabled;
    const actions = element("div", "d-flex gap-2");
    actions.append(button(row.restore, () => cmd(row.command, row.index), disabled));
    if (row.after) actions.append(button(labels.after, () => cmd("after", row.index), disabled));
    box.append(actions);
    if (!row.edit) return box;
    const edit = check(row.edit.alignment ? labels.edit : labels.editColour, "edit:" + row.index);
    const paint = element("select", "form-select form-select-sm w-auto");
    paint.setAttribute("aria-label", "Paint");
    for (const [label, value] of state.paints) paint.append(new Option(label, JSON.stringify(value)));
    const custom = element("input", "form-control form-control-color form-control-sm");
    custom.type = "color";
    custom.setAttribute("aria-label", "Custom paint colour");
    const rgb = row.edit.paint.Custom || (row.edit.paint === "Red" ? [255, 0, 0] : [255, 255, 255]);
    custom.value = "#" + rgb.map(component => component.toString(16).padStart(2, "0")).join("");
    paint.value = JSON.stringify(row.edit.paint.Custom || row.edit.paint === "Red" ? {Custom: [255, 255, 255]} : row.edit.paint);
    const showCustom = () => { custom.hidden = !JSON.parse(paint.value).Custom; };
    paint.addEventListener("change", showCustom);
    showCustom();
    const fields = element("div", "d-flex gap-2 my-2");
    fields.append(paint, custom);
    edit.append(fields);
    const alignment = {};
    if (row.edit.alignment) {
      for (const [key, label, max] of [["camber", "Camber", 30], ["vertical", "Up / down", 15], ["lateral", "Track width", 15]]) {
        const wrap = element("label", "d-flex gap-2 align-items-center mb-2");
        const slider = element("input", "form-range mb-0");
        slider.type = "range";
        slider.min = -max;
        slider.max = max;
        slider.step = 0.25;
        slider.value = row.edit.alignment[key];
        const display = element("output", "", slider.value);
        slider.addEventListener("input", () => { display.value = slider.value; });
        wrap.append(element("span", "text-nowrap", label), slider, display);
        alignment[key] = { slider, display };
        edit.append(wrap);
      }
      edit.append(button(labels.reset, () => {
        for (const { slider, display } of Object.values(alignment)) slider.value = display.value = 0;
      }, disabled));
    }
    edit.append(button(labels.apply, () => {
      const colour = JSON.parse(paint.value);
      if (colour.Custom) colour.Custom = custom.value.slice(1).match(/../g).map(part => parseInt(part, 16));
      cmd("edit", row.index, { paint: colour, alignment: row.edit.alignment ?
        Object.fromEntries(Object.entries(alignment).map(([key, { slider }]) => [key, slider.valueAsNumber])) : null });
    }, disabled, "primary"));
    box.append(edit);
    return box;
  }
  function draw() {
    const term = filter.value.trim().toLowerCase();
    list.replaceChildren();
    for (const group of state.groups) {
      if (!group.name.toLowerCase().includes(term) && !group.rows.some(row => (row.file + " " + row.label).toLowerCase().includes(term))) continue;
      const card = panel();
      card.append(element("h2", "h6 mb-0", group.name), point(group.rows[0]));
      if (group.rows.length > 1) {
        const checkpoints = check(labels.checkpoints, "group:" + group.rows[0].index);
        checkpoints.append(...group.rows.slice(1).map(point));
        card.append(checkpoints);
      }
      list.append(card);
    }
    if (!list.children.length) list.append(element("p", "text-body-secondary", term ? "No matching checkpoints." : labels.empty));
  }
  filter.addEventListener("input", () => { historyView.query = filter.value; draw(); });
  draw();
}

function maintenance() {
  const cmd = (key, enabled = null) => send({ type: "maintenance", key, enabled });
  const { labels } = state;
  const scope = panel();
  scope.append(button("‹ " + labels.back, () => cmd("back")), element("h2", "h6 mt-3", labels.remove));
  for (const [key, label, enabled] of state.options) scope.append(toggle(label, enabled, value => cmd(key, value)));
  const paths = panel();
  paths.append(element("p", "mb-2", labels.note), element("code", "text-break", state.base));
  const files = element("details", "mt-2");
  files.append(element("summary", "", labels.files));
  for (const path of state.paths) files.append(element("code", "d-block text-break", path));
  paths.append(files);
  const actions = panel();
  if (state.error) actions.append(element("p", "text-danger", state.error));
  actions.append(button(labels.remove, () => cmd("confirm"), !state.selected, "outline-danger"));
  rows.append(scope, paths, actions);
  if (state.confirm) {
    const dialog = element("dialog", "tracker-detail loadout-dialog");
    const heading = element("h2", "h5", labels.remove);
    heading.id = "maintenance-confirm";
    dialog.setAttribute("aria-labelledby", heading.id);
    const close = key => dismissDialog(dialog, () => cmd(key));
    dialog.append(heading, element("p", "", labels.confirm),
      button(labels.cancel, () => close("cancel")), " ",
      button(labels.confirmButton, () => close("remove"), false, "danger"));
    openLoadoutModal(dialog, () => close("cancel"));
  }
}

function menuTextures() {
  const content = studioPage();
  const action = (key, value = null) => send({ type: "menu_textures", key, value });
  const top = panel();
  const packageSelect = element("select", "form-select form-select-sm w-auto");
  packageSelect.setAttribute("aria-label", "Menu package");
  packageSelect.append(new Option("Choose a menu package", ""));
  packageSelect.options[0].disabled = true;
  for (const file of state.packages) packageSelect.append(new Option(file, file));
  packageSelect.value = state.file || "";
  packageSelect.disabled = state.busy;
  packageSelect.addEventListener("change", () => action("package", packageSelect.value));
  const toolbar = element("div", "d-flex flex-wrap align-items-center gap-2");
  toolbar.append(packageSelect, button("Reload", () => action("reload"), !state.file || state.busy),
    button("Restore All (" + state.active + ")", () => action("restore-all"), !state.active || state.busy || !state.closed, "outline-danger"));
  top.append(element("p", "small text-body-secondary mb-2", "Close Rocket League to replace textures in its menu packages."), toolbar);
  if (state.message || state.error) top.append(element("p", "small mb-0 mt-2", state.error || state.message));
  content.append(top);

  const source = panel();
  const replacementControls = element("div", "d-flex flex-wrap align-items-center gap-2");
  replacementControls.append(button("Image file…", () => action("image"), state.busy),
    button("Minecraft resource pack…", () => action("pack"), state.busy),
    toggle("Tile (4×)", state.tile, value => action("tile", value), state.busy),
    toggle("Keep original alpha", state.keepAlpha, value => action("alpha", value), state.busy));
  function textureButton(name, image, detail, clicked, disabled, selected = false) {
    const tile = button("", clicked, disabled, selected ? "primary" : "outline-secondary");
    tile.classList.add("d-flex", "flex-column", "align-items-center", "text-break");
    if (image) {
      const preview = element("img", "object-fit-contain mb-1");
      preview.src = image;
      preview.alt = "";
      preview.width = 70;
      preview.height = 64;
      tile.append(preview);
    }
    tile.append(element("span", "", name), element("small", "", detail));
    return tile;
  }
  source.append(element("h2", "h6", "Replacement"), replacementControls);
  if (state.replacement) {
    const selection = element("div", "mt-2");
    const preview = textureButton(state.replacement.name, state.replacement.image, "", () => {}, true);
    selection.append(preview);
    source.append(selection);
  }
  if (state.packCount) {
    const search = element("input", "form-control form-control-sm my-2");
    search.type = "search";
    search.placeholder = "Filter " + state.packCount + " resource pack textures";
    search.setAttribute("aria-label", "Filter resource pack textures");
    search.value = state.packSearch;
    search.addEventListener("input", () => action("pack-search", search.value));
    const pack = element("div", "d-flex flex-wrap gap-2");
    for (const item of state.pack) pack.append(textureButton(item.name, item.image, "", () => action("pack-select", item.name), state.busy));
    source.append(search, pack);
  }
  content.append(source);
  if (!state.file) return;
  const textures = panel();
  textures.append(element("h2", "h6", state.busy ? "Patching…" : state.loading ? "Loading textures…" : "Package textures"));
  const selected = state.rows.filter(row => row.selected).length;
  const tools = element("div", "d-flex flex-wrap align-items-center gap-2 mb-2");
  tools.append(element("span", "small", state.rows.length + " textures, " + selected + " selected"),
    button("Select all", () => action("select-all"), state.busy || !state.rows.length),
    button("Clear", () => action("clear"), state.busy || !selected),
    button("Apply to selected", () => action("apply"), state.busy || !state.closed || !selected || !state.replacement, "primary"),
    button("Restore selected (" + state.restoreCount + ")", () => action("restore"), state.busy || !state.closed || !state.restoreCount));
  const grid = element("div", "d-flex flex-wrap gap-2");
  for (const item of state.rows) {
    const tile = textureButton(item.name, item.image,
      item.error ? "Unreadable" : item.size.join(" × ") + (item.patched ? " · Patched" : ""),
      () => action("select", item.id), state.busy || Boolean(item.error), item.selected);
    tile.title = item.error || item.name + " · export #" + item.id;
    tile.setAttribute("aria-pressed", String(item.selected));
    grid.append(tile);
  }
  textures.append(tools, grid);
  content.append(textures);
}

function studioAsset() {
  const content = studioPage();
  const view = studioAssetView;
  const key = state.kind + ":" + state.custom;
  if (view.key !== key) {
    Object.assign(view, { key, page: 0, requested: "" });
    view.previews.clear();
  }
  const { labels } = state;
  const decal = state.kind === "decal";
  const action = (command, id = null, value = null) => send(decal ?
    { type: "studio_decal", key: command, id, value } : { type: "studio_asset", key: command, id });
  const controls = panel();
  const actions = element("div", "d-flex flex-wrap gap-2 mb-2");
  actions.append(button("Catalog", () => action("source", "catalog"), false, state.custom ? "outline-secondary" : "primary"),
    button("Custom", () => action("source", "custom"), false, state.custom ? "primary" : "outline-secondary"),
    button(labels.refresh, () => action("refresh")), button(labels.import, () => action("import"), state.busy),
    button(labels.restore, () => action("restore"), !state.active || state.busy || (decal && Boolean(state.error))));
  if (state.kind === "boost") {
    const ring = button(labels.ring, () => action("ring"), state.busy);
    ring.title = labels.ringHint;
    actions.append(ring);
  }
  const search = element("input", "form-control form-control-sm");
  search.type = "search";
  search.value = view.search;
  search.placeholder = labels.search;
  search.setAttribute("aria-label", labels.search);
  const filter = state.custom ? "applied" : "downloaded";
  controls.append(actions, toggle(labels[filter], view[filter], enabled => {
    view[filter] = enabled;
    view.page = 0;
    draw();
  }), search);
  const grid = element("div", "plugin-catalog-grid studio-asset-grid");
  const paging = element("div", "d-flex justify-content-center align-items-center gap-2");
  content.append(controls);
  if (decal) {
    const split = element("div", "studio-decal-panes");
    const browse = element("div", "d-grid gap-2");
    browse.append(grid, paging);
    const active = panel();
    active.classList.add("studio-decal-active");
    active.append(element("h2", "h6", labels.active + " · " + state.activeDecals.length));
    if (!state.activeDecals.length) active.append(element("p", "small text-body-secondary", labels.empty));
    for (const row of state.activeDecals) {
      const item = element("div", "d-flex gap-2 align-items-center border-top py-2");
      if (row.image) {
        const img = element("img", "object-fit-contain");
        img.width = 42; img.height = 42; img.src = row.image; img.alt = "";
        item.append(img);
      }
      const text = element("div", "flex-grow-1");
      text.append(element("strong", "d-block small", row.name), element("span", "small text-body-secondary", row.target));
      item.append(text, button("Restore", () => action("restore", row.id), state.busy));
      active.append(item);
    }
    split.append(browse, active);
    content.append(split);
    if (state.catalogError && state.custom) {
      const retry = panel();
      retry.append(element("p", "text-danger small mb-2", state.catalogError), button(labels.retry, () => action("reload")));
      content.append(retry);
    }
    if (state.busy) {
      const working = element("div", "text-body-secondary small", state.status || "Working…");
      window.patchProgress(working, true, state.progress, state.status || "Applying patch");
      content.append(working);
    }
  } else content.append(grid, paging);
  if (state.catalog?.status) content.append(element("p", "text-body-secondary small", state.catalog.status));
  if (state.error) content.append(element("p", "text-danger small", state.error));
  function draw() {
    const term = view.search.trim().toLowerCase();
    const items = (state.custom ? state.rows : state.catalog?.rows || []).filter(item =>
      (item.name + " " + (item.author || "")).toLowerCase().includes(term) &&
      (state.custom ? !view.applied || item.active : !view.downloaded || item.downloaded));
    const pageSize = Math.min(36, Math.max(1, Math.floor((grid.clientWidth + 16) / 191)) * 3);
    const pages = Math.max(1, Math.ceil(items.length / pageSize));
    view.page = Math.min(view.page, pages - 1);
    const visible = items.slice(view.page * pageSize, (view.page + 1) * pageSize);
    const ids = visible.map(item => item.id);
    const signature = JSON.stringify(ids);
    if (signature !== view.requested) {
      view.requested = signature;
      send({ type: "studio_asset_previews", ids });
    }
    grid.replaceChildren();
    for (const item of visible) {
      const card = element("article", "tracker-detail plugin-card");
      const image = element("div", "studio-active-art");
      const length = item.images.length;
      const current = (view.previews.get(item.id) || 0) % Math.max(1, length);
      const images = state.custom ? item.images : item.images.slice(current, current + 1);
      if (!images.some(Boolean)) image.append(element("span", "small text-body-secondary", "No preview"));
      for (const source of images.filter(Boolean)) {
        const artwork = element("img", "");
        artwork.src = source;
        artwork.alt = "";
        image.append(artwork);
      }
      if (!state.custom && length > 1) {
        const previous = icon("‹", "Previous preview", () => { view.previews.set(item.id, (current + length - 1) % length); draw(); });
        const next = icon("›", "Next preview", () => { view.previews.set(item.id, (current + 1) % length); draw(); });
        previous.classList.add("studio-preview-prev");
        next.classList.add("studio-preview-next");
        image.append(previous, next);
      }
      if (item.pack) image.append(element("span", "badge text-bg-danger studio-pack-tag", "PACK"));
      const info = element("div", "plugin-card-body text-center");
      info.append(element("h2", "h6 mb-1", item.name));
      if (!state.custom) {
        if (item.author) info.append(element("p", "small mb-1", item.author));
        info.append(element("p", "small text-body-secondary mb-2", item.downloads + " downloads"),
          button(item.busy ? "Downloading…" : item.downloaded ? "Import" : "Download",
            () => action("download", item.id), item.busy || state.busy, "primary"));
      } else {
        if (item.subtitle) info.append(element("p", "small text-body-secondary mb-2", item.subtitle));
        if (decal) {
          if (item.chassis) info.append(element("p", "small text-body-secondary mb-2", "Chassis textures skipped"));
          const apply = button(labels.apply, () => action("apply", item.id), state.busy || Boolean(state.error) ||
            !item.targets.some(([id, , patched]) => id === item.target && !patched), "primary");
          if (!item.body) {
            const body = loadoutSelect("Select body", [["", "Select body…"], ...state.bodies], "", value => action("body", item.id, value));
            body.options[0].disabled = true;
            info.append(body);
          } else {
            const filter = element("input", "form-control form-control-sm mb-1");
            filter.type = "search"; filter.placeholder = "Filter decals";
            filter.setAttribute("aria-label", filter.placeholder);
            if (!view.targetFilters) view.targetFilters = new Map();
            filter.value = view.targetFilters.get(item.id) || "";
            const target = element("select", "form-select form-select-sm");
            target.setAttribute("aria-label", labels.target);
            target.disabled = state.busy;
            const update = () => {
              const query = filter.value.trim().toLowerCase();
              target.replaceChildren(new Option(labels.target, ""));
              target.options[0].disabled = true;
              for (const [id, label, patched] of item.targets) {
                if (id !== item.target && !(label + " " + id).toLowerCase().includes(query)) continue;
                const option = new Option(label + (patched ? " (patched)" : ""), id);
                option.disabled = patched;
                target.append(option);
              }
              target.value = item.target || "";
            };
            filter.addEventListener("input", () => { view.targetFilters.set(item.id, filter.value); update(); });
            target.addEventListener("change", () => action("target", item.id, target.value));
            update();
            info.append(filter, target);
          }
          if (state.speed) {
            const label = element("label", "small mt-2", "Speed multiplier");
            const speed = element("input", "form-range");
            speed.type = "range"; speed.min = 0.1; speed.max = 20; speed.step = 0.05; speed.value = item.speed;
            speed.disabled = state.busy;
            const display = element("output", "ms-2", item.speed + "×");
            speed.addEventListener("input", () => { display.value = speed.value + "×"; });
            speed.addEventListener("change", () => action("speed", item.id, speed.valueAsNumber));
            label.append(display, speed);
            info.append(label);
          }
          const buttons = element("div", "d-flex flex-wrap gap-2 justify-content-center pt-2 mt-auto");
          buttons.append(apply, button(labels.delete, () => action("delete", item.id), state.busy || !item.delete, "outline-danger"));
          info.append(buttons);
          card.append(image, info);
          grid.append(card);
          continue;
        }
        const restoring = item.restore ?? item.active;
        const apply = button(restoring ? labels.restore : labels.apply,
          () => action(restoring ? "restore" : "apply", item.id), state.busy || (!restoring && Boolean(item.error)),
          restoring ? "outline-secondary" : "primary");
        if (item.error) info.append(element("p", "small text-danger mb-2", item.error));
        const actions = element("div", "studio-asset-actions mt-auto pt-2");
        actions.append(apply, button(labels.delete, () => action("delete", item.id), state.busy || !item.delete, "outline-danger"));
        info.append(actions);
      }
      card.append(image, info);
      grid.append(card);
    }
    if (!items.length) grid.append(element("p", "text-body-secondary", state.catalog?.loading ? "Loading…" : "No matching items."));
    paging.hidden = pages < 2;
    paging.replaceChildren(button("Previous", () => { view.page--; draw(); }, view.page === 0),
      element("span", "", view.page + 1 + " / " + pages),
      button("Next", () => { view.page++; draw(); }, view.page + 1 >= pages));
  }
  search.addEventListener("input", () => { view.search = search.value; view.page = 0; draw(); });
  draw();
  if (state.confirmRestore) {
    const dialog = element("dialog", "tracker-detail loadout-dialog");
    const title = element("h2", "h5", labels.restoreTitle);
    title.id = "studio-restore-title";
    dialog.setAttribute("aria-labelledby", title.id);
    const choose = confirm => dismissDialog(dialog, () => action(confirm ? "restore-all" : "cancel"));
    dialog.append(title, element("p", "", "Restore all " + state.activeDecals.length + " patched decal targets?"),
      button(labels.no, () => choose(false)), " ", button(labels.yes, () => choose(true), state.busy, "danger"));
    openLoadoutModal(dialog, () => choose(false));
  }
  if (state.confirm) {
    const dialog = element("dialog", "tracker-detail loadout-dialog");
    const title = element("h2", "h5", labels.confirm);
    title.id = "studio-delete-title";
    dialog.setAttribute("aria-labelledby", title.id);
    const choose = confirm => dismissDialog(dialog, () => action(confirm ? "confirm" : "cancel"));
    dialog.append(title, element("p", "", "Delete " + state.confirm + "?"),
      button(labels.no, () => choose(false)), " ", button(labels.yes, () => choose(true), state.busy, "danger"));
    openLoadoutModal(dialog, () => choose(false));
  }
}

function ballAppearance() {
  const content = studioPage();
  const { labels } = state;
  const update = (key, value = null) => send({ type: "ball_appearance", key, value });
  const editor = panel();
  const chooser = element("div", "d-flex flex-wrap gap-2 mb-3");
  const models = state.balls.map((label, index) => [index, label]);
  const original = loadoutSelect(labels.source, models, state.source, index => update("source", Number(index)));
  const donor = loadoutSelect(labels.donor, models, state.donor, index => update("donor", Number(index)));
  original.disabled = donor.disabled = state.busy;
  donor.options[state.source].disabled = true;
  chooser.append(element("span", "", labels.source), original, element("span", "", labels.donor), donor);
  editor.append(element("p", "", labels.description), element("p", "small text-body-secondary", labels.hint), chooser,
    toggle(labels.colour, state.colours, value => update("colours", value), state.busy || !state.canColour),
    element("p", "small text-body-secondary", labels.colourHint));
  if (state.applied) editor.append(element("p", "", state.applied), element("p", "small", labels.restoreFirst));
  if (state.ballError) editor.append(element("p", "text-warning", state.ballError));
  editor.append(button(labels.apply, () => update("apply"), !state.canApply || state.busy, "primary"), " ",
    button(labels.restore, () => update("restore"), !state.canRestore || state.busy));
  const status = panel();
  status.append(element("h2", "h6", "Status"), element("p", state.error ? "text-warning" : "small",
    state.status || (state.busy ? "Working…" : "")));
  content.append(editor, status);
}

function liveGoalExplosion() {
  const settings = structuredClone(state.live.settings);
  const section = panel();
  section.append(element("h2", "h6", "Goal explosion scale"),
    element("p", "small text-body-secondary", "Changes apply to the next goal explosion. Default preserves the game's original size."));
  const controls = element("div", "live-alignment-controls");
  const slider = element("input", "form-range");
  slider.type = "range"; slider.min = "0.1"; slider.max = "10"; slider.step = "0.1";
  slider.value = settings.goal_explosion_scale || 1;
  slider.disabled = !settings.goal_explosion_scale;
  slider.setAttribute("aria-label", "Goal explosion scale");
  const output = element("output", "small text-nowrap", slider.disabled ? "Default" : slider.value + "×");
  let timer;
  const save = () => { clearTimeout(timer); timer = setTimeout(() => send({type: "live_cosmetics", settings}), 100); };
  section.append(toggle("Custom scale", !!settings.goal_explosion_scale, on => {
    slider.disabled = !on;
    settings.goal_explosion_scale = on ? Number(slider.value) : 0;
    output.textContent = on ? slider.value + "×" : "Default";
    save();
  }));
  slider.addEventListener("input", () => { settings.goal_explosion_scale = Number(slider.value); output.textContent = slider.value + "×"; save(); });
  controls.append(slider, output); section.append(controls);
  return section;
}

function liveCosmeticEditor(kind, settings = structuredClone(state.live.settings), scheduleSave) {
  const section = panel();
  section.classList.add("live-cosmetics");
  let timer;
  const save = scheduleSave || (() => {
    clearTimeout(timer);
    timer = setTimeout(() => send({type: "live_cosmetics", settings}), 100);
  });
  section.append(element("h2", "h6", kind === "alignment" ? "Live Wheel Alignment · All local cars" : "Live Colours"),
    element("p", "small text-body-secondary", "Changes are saved automatically and update in Rocket League. Values remain available while Hebnix is closed."),
    element("p", "small text-body-secondary live-setup-status", "Live cosmetics: " + (state.live.status || "Starting…")));
  if (kind === "alignment") {
    for (const [key, label, max, unit] of [["camber", "Camber", 30, "°"], ["vertical", "Up / Down", 15, ""], ["lateral", "Track width", 15, ""]]) {
      const line = element("label", "appearance-row");
      const controls = element("div", "live-alignment-controls");
      const slider = element("input", "form-range");
      const output = element("output", "small text-nowrap");
      slider.type = "range";
      slider.min = -max; slider.max = max; slider.step = "0.5";
      slider.value = settings.alignment[key];
      slider.setAttribute("aria-label", label);
      output.textContent = slider.value + unit;
      slider.addEventListener("input", () => { settings.alignment[key] = Number(slider.value); output.textContent = slider.value + unit; save(); });
      controls.append(slider, output);
      line.append(element("span", "", label), controls);
      section.append(line);
    }
    section.append(button("Reset alignment", () => {
      settings.alignment = {camber: 0, vertical: 0, lateral: 0};
      save();
      document.activeElement?.blur();
    }));
  } else {
    for (const [key, label] of [["primary", "Primary paint"], ["secondary", "Secondary paint"], ["trim", "Body / trim"], ["decal", "Decal"], ["wheels", "Wheels"], ["antenna", "Antenna"], ["topper", "Topper"], ["boost", "Boost"], ["trail", "Trail"], ["goal_explosion", "Goal explosion"], ["banner", "Player banner"], ["avatar_border", "Avatar border"], ["heatseeker", "Heatseeker ball"]]) {
      if (key === "primary") section.append(element("p", "small text-body-secondary", "Colours apply to your currently equipped items."), element("h3", "h6 mt-3 mb-3", "Car Colours"));
      if (key === "boost") section.append(element("hr", "my-4"), element("h3", "h6 mb-3", "Effects"));
      if (key === "banner") section.append(element("hr", "my-4"), element("h3", "h6 mb-3", "Player Colours"));
      if (key === "heatseeker") section.append(element("hr", "my-4"), element("h3", "h6 mb-3", "Heatseeker Ball"));
      const row = element("div", "live-colour-row mb-3");
      const input = element("input", "form-control form-control-color");
      input.type = "color";
      input.value = "#" + settings[key].rgb.map(b => b.toString(16).padStart(2, "0")).join("");
      input.setAttribute("aria-label", label);
      const rainbow = key !== "goal_explosion";
      if (!rainbow) settings[key].rainbow = false;
      input.disabled = !!settings[key].rainbow;
      input.addEventListener("input", () => { settings[key].rgb = input.value.slice(1).match(/../g).map(b => parseInt(b, 16)); save(); });
      row.append(toggle(label, settings[key].enabled, on => { settings[key].enabled = on; save(); }), input);
      if (rainbow) row.append(toggle("Rainbow", !!settings[key].rainbow, on => { settings[key].rainbow = on; input.disabled = on; save(); }));
      section.append(row);
    }
  }
  return section;
}

function gameColours() {
  const content = state.live ? rows : studioPage();
  const { labels } = state;
  const settings = structuredClone(state.settings);
  const liveSettings = state.live && structuredClone(state.live.settings);
  if (liveSettings) for (const key of ["stadium", "hud"]) {
    settings[key + "_colours"] = liveSettings[key].enabled;
    settings[key + "_blue"] = liveSettings[key].blue.slice();
    settings[key + "_orange"] = liveSettings[key].orange.slice();
  }
  let liveTimer;
  const saveLiveColours = () => {
    clearTimeout(liveTimer);
    liveTimer = setTimeout(() => send({type: "live_cosmetics", settings: liveSettings}), 100);
  };
  if (liveSettings) content.append(liveCosmeticEditor("colours", liveSettings, saveLiveColours));
  const saveTeamColours = key => {
    if (!liveSettings) return;
    liveSettings[key] = { enabled: settings[key + "_colours"], blue: settings[key + "_blue"].slice(), orange: settings[key + "_orange"].slice() };
    saveLiveColours();
  };
  const controls = element("div", "d-grid gap-2");
  if (liveSettings) controls.classList.add("live-cosmetics");
  const field = (label, rgb, onChange) => {
    const row = element("label", "d-flex gap-2 align-items-center mb-2");
    const input = element("input", "form-control form-control-color");
    const hex = element("span", "small font-monospace");
    input.type = "color";
    input.value = "#" + rgb.map(byte => byte.toString(16).padStart(2, "0")).join("");
    hex.textContent = input.value.toUpperCase();
    input.addEventListener("input", () => {
      onChange(input.value.slice(1).match(/.{2}/g).map(byte => parseInt(byte, 16)));
      hex.textContent = input.value.toUpperCase();
    });
    row.append(element("span", "", label), input, hex);
    return row;
  };
  const group = (title, label, hint, key, children) => {
    const section = panel();
    section.append(element("h2", "h6", title), toggle(label, settings[key], enabled => {
      settings[key] = enabled;
      if (key === "stadium_colours" || key === "hud_colours") saveTeamColours(key.split("_")[0]);
      draw();
    }), element("p", "small text-body-secondary", hint));
    if (settings[key]) children(section);
    return section;
  };
  function teamSection(section, key) {
    const blue = key + "_blue", orange = key + "_orange";
    section.append(field(labels.blue, settings[blue], rgb => { settings[blue] = rgb; saveTeamColours(key); }),
      field(labels.orange, settings[orange], rgb => { settings[orange] = rgb; saveTeamColours(key); }));
    if (liveSettings) section.append(element("p", "small text-body-secondary", "Live · changes are saved automatically. Disable to use the game's colours."));
    const options = element("div", "d-flex gap-2");
    if (key === "hud") options.append(button(labels.match, () => {
      settings.hud_blue = settings.stadium_blue.slice();
      settings.hud_orange = settings.stadium_orange.slice();
      saveTeamColours("hud");
      draw();
    }));
    options.append(button(labels.defaults, () => {
      settings[blue] = state.defaults[blue].slice();
      settings[orange] = state.defaults[orange].slice();
      saveTeamColours(key);
      draw();
    }), button(labels.swap, () => {
      [settings[blue], settings[orange]] = [settings[orange], settings[blue]];
      saveTeamColours(key);
      draw();
    }));
    section.append(options);
  }
  function draw() {
    const teamGroups = [
      group(labels.stadium, labels.stadiumOn, labels.stadiumHint, "stadium_colours", section => teamSection(section, "stadium")),
      group(labels.hud, labels.hudOn, labels.hudHint, "hud_colours", section => teamSection(section, "hud")),
    ];
    if (liveSettings) {
      controls.replaceChildren(...teamGroups);
      return;
    }
    controls.replaceChildren(
      ...(state.liveManaged ? [] : teamGroups),
      group(labels.palette, labels.paletteOn, labels.paletteHint, "extended_palette", () => {}),
      group(labels.car, labels.carOn, labels.carHint, "car_colours", section => {
        settings.custom_colours.forEach((rgb, index) => {
          const row = element("div", "d-flex gap-2");
          row.append(field(labels.custom, rgb, next => { settings.custom_colours[index] = next; }),
            icon("×", "Remove colour", () => { settings.custom_colours.splice(index, 1); draw(); }));
          section.append(row);
        });
        section.append(icon("+", "Add colour", () => { settings.custom_colours.push([255, 0, 0]); draw(); }));
      }),
      ...(!state.liveManaged ? [group(labels.heatseeker, labels.heatseekerOn, labels.heatseekerHint, "heatseeker_glow",
        section => section.append(field(labels.maxSpeed, settings.heatseeker_max_speed, rgb => { settings.heatseeker_max_speed = rgb; })))] : []));
  }
  draw();
  if (liveSettings) {
    content.append(controls);
    return;
  }
  const status = panel();
  const apply = restore => send({ type: "game_colours", settings, restore });
  status.append(element("h2", "h6", "Status"),
    button(labels.apply, () => apply(false), state.busy, "primary"), " ",
    button(labels.restore, () => apply(true), state.busy || !state.canRestore),
    element("p", state.error ? "text-warning mt-2" : "small mt-2", state.status || (state.busy ? "Working…" : "")),
    element("p", "small text-body-secondary mb-0", state.backup));
  content.append(element("p", "text-body-secondary mb-0", labels.description), controls, status);
}

function wheelAlignment() {
  const content = studioPage();
  const { labels } = state;
  const editor = panel();
  const search = element("input", "form-control form-control-sm w-auto");
  search.type = "search";
  search.placeholder = labels.filter;
  search.setAttribute("aria-label", labels.filter);
  const picker = loadoutSelect("Car", [], "", index => send({ type: "wheel_select", index: Number(index) }));
  picker.disabled = state.busy || !state.models.length;
  search.disabled = picker.disabled;
  const update = () => {
    const term = search.value.trim().toLowerCase();
    picker.replaceChildren(...state.models.map((name, index) => [name, index])
      .filter(([name]) => name.toLowerCase().includes(term)).map(([name, index]) => new Option(name, index)));
    picker.value = String(state.selected);
  };
  search.addEventListener("input", update);
  update();
  const chooser = element("div", "d-flex gap-2 flex-wrap mb-3");
  chooser.append(search, picker);
  editor.append(chooser);
  if (state.catalogLoading || state.catalogError || !state.models.length) {
    editor.append(element("p", "small text-body-secondary", state.catalogError || (state.catalogLoading ? "Loading…" : labels.noCatalog)));
  }
  const values = { ...state.settings };
  const inputs = {};
  if (state.models.length) {
    for (const [key, max, step] of [["camber", 30, ".5"], ["vertical", 15, ".5"], ["lateral", 15, ".5"]]) {
      const line = element("label", "appearance-row");
      const controls = element("div", "d-flex gap-2");
      const slider = element("input", "form-range w-auto");
      const output = element("output", "small");
      slider.type = "range";
      slider.min = String(-max);
      slider.max = String(max);
      slider.value = String(values[key]);
      slider.step = step;
      slider.disabled = state.busy;
      output.textContent = slider.value;
      slider.addEventListener("input", () => {
        values[key] = Number(slider.value);
        output.textContent = slider.value;
      });
      inputs[key] = { slider, output };
      controls.append(slider, output);
      line.append(element("span", "", labels[key]), controls);
      editor.append(line, element("p", "text-body-secondary small", labels[key + "Hint"]));
    }
    const apply = button(labels.apply, () => send({ type: "wheel_apply", settings: values }), state.busy || state.running, "primary");
    if (state.running) apply.title = labels.close;
    editor.append(apply);
  }
  const revert = panel();
  revert.append(element("h2", "h6", labels.revert), button(labels.reset, () => {
    for (const [key, { slider, output }] of Object.entries(inputs)) {
      values[key] = 0;
      slider.value = output.textContent = "0";
    }
  }, state.busy), element("p", "small text-body-secondary mt-2", state.editedLabel));
  const list = element("div", "menumap-options d-grid gap-1");
  for (const row of state.edited) {
    const line = element("div", "d-flex gap-2 align-items-center");
    line.append(button(row.label, () => send({ type: "wheel_select", index: row.index }), state.busy || row.index === null,
      row.index === state.selected ? "primary" : "outline-secondary"),
    icon("×", "Restore " + row.label, () => send({ type: "wheel_apply", remove: row.file }),
      state.busy || state.running, "outline-danger"));
    list.append(line);
  }
  if (!state.edited.length) list.append(element("p", "text-body-secondary small", labels.empty));
  revert.append(list);
  const status = panel();
  status.append(element("h2", "h6", "Status"), element("p", "small mb-0", state.status || (state.busy ? "Working…" : "")));
  content.append(editor, revert, status);
}

function goalExplosion() {
  const content = studioPage();
  state.panels.forEach((model, index) => {
    const card = panel();
    const row = element("div", "d-flex align-items-center gap-2 flex-wrap mb-2");
    const slider = element("input", "form-range w-auto");
    const input = element("input", "form-control form-control-sm w-auto");
    slider.type = "range";
    input.type = "number";
    input.min = String(model.min);
    input.max = String(model.max);
    input.step = model.log ? "0.01" : "1";
    input.value = String(model.value);
    slider.min = model.log ? "0" : input.min;
    slider.max = model.log ? "100" : input.max;
    slider.step = "1";
    slider.disabled = input.disabled = model.busy;
    slider.setAttribute("aria-label", model.title);
    input.setAttribute("aria-label", model.title + " value");
    const mute = element("span", "text-body-secondary small", model.mute);
    const logarithm = Math.log(model.max / model.min);
    const sync = () => {
      const value = Math.max(model.min, Math.min(model.max, Number(input.value)));
      slider.value = String(model.log ? 100 * Math.log(value / model.min) / logarithm : value);
      mute.hidden = !model.mute || value > model.min;
    };
    sync();
    input.addEventListener("input", sync);
    slider.addEventListener("input", () => {
      input.value = model.log ? (model.min * Math.exp(logarithm * Number(slider.value) / 100)).toFixed(2) : slider.value;
      sync();
    });
    row.append(slider, input, element("span", "", model.suffix), mute);
    const command = restore => send({ type: "goal_explosion", volume: true, value: Number(input.value), restore });
    card.append(element("h2", "h6", model.title), element("p", "text-body-secondary", model.description),
      row, button(model.apply, () => command(false), model.busy, "primary"), " ",
      button(model.restore, () => command(true), model.busy || !model.active));
    if (model.status || model.busy) card.append(element("p", "small mt-2 mb-0", model.status || "Working…"));
    window.patchProgress(card, !!model.busy, model.progress, model.status || model.title);
    content.append(card);
  });
}

function speedPatch() {
  const labels = state.labels;
  const section = studioPage();
  const content = panel();
  content.append(element("p", "text-body-secondary", labels.description),
    toggle(labels.items, state.items, enabled => send({ type: "speed_items", enabled })),
    element("p", "small text-body-secondary", labels.itemsHint),
    element("hr", ""), element("h2", "h6", labels.oneoff),
    element("p", "small text-body-secondary", labels.oneoffHint));
  const chooser = element("div", "d-flex gap-2 align-items-center mb-3");
  chooser.append(button(labels.choose, () => send({ type: "speed_choose" })),
    element("span", "text-break", state.package || labels.noPackage));
  const sliderRow = element("label", "d-flex flex-wrap align-items-center gap-2 mb-3");
  const slider = element("input", "form-range w-auto");
  const number = element("input", "form-control form-control-sm w-auto");
  slider.type = "range";
  number.type = "number";
  slider.min = number.min = String(state.min);
  slider.max = number.max = String(state.max);
  slider.step = number.step = "0.25";
  slider.value = number.value = String(state.speed);
  slider.setAttribute("aria-label", labels.multiplier);
  number.setAttribute("aria-label", labels.multiplier + " value");
  slider.addEventListener("input", () => { number.value = slider.value; });
  number.addEventListener("input", () => { slider.value = number.value; });
  const preset = value => { slider.value = number.value = String(value); };
  sliderRow.append(element("span", "", labels.multiplier), slider, number,
    button("1x", () => preset(1)), button("5x", () => preset(5)));
  const action = restore => send({ type: "speed_run", speed: Number(number.value), restore });
  const actions = element("div", "d-flex gap-2");
  actions.append(button(labels.patch, () => action(false), !state.package || state.busy, "primary"),
    button(labels.restore, () => action(true), !state.package || state.busy));
  content.append(chooser, sliderRow, actions);
  if (state.status || state.busy) content.append(element("p", state.error ? "text-warning mt-2" : "text-body-secondary mt-2", state.status || "Working…"));
  section.append(content);
}

function filesSettings() {
  const labels = state.labels;
  const update = (key, value) => send({ type: "files", key, value });
  const card = panel();
  const row = (title, controls) => {
    const line = element("div", "appearance-row");
    const values = element("div", "d-flex align-items-center gap-2");
    values.append(...controls);
    line.append(element("span", "", title), values);
    return line;
  };
  card.append(element("p", "text-body-secondary small", labels.intro));
  for (const [key, label, value] of state.paths.slice(0, 2)) {
    const field = element("input", "form-control form-control-sm");
    field.readOnly = true;
    field.value = value;
    field.title = value;
    field.setAttribute("aria-label", label);
    card.append(row(label, [field, button(labels.browse, () => update(key))]));
  }
  for (const [key, label, value, suggested] of state.ini) {
    const field = element("input", "form-control form-control-sm w-auto");
    field.inputMode = "numeric";
    field.placeholder = labels.missing;
    field.value = value || "";
    field.setAttribute("aria-label", label);
    field.addEventListener("input", () => { field.value = field.value.replace(/\D/g, ""); });
    field.addEventListener("change", () => {
      const port = Number(field.value);
      if (field.value && (key === "PacketSendRate" || port >= 1 && port <= 65535)) update(key, field.value);
      else field.value = value || "";
    });
    const controls = [field];
    const set = { PacketSendRate: labels.setRate, Port: labels.reset, WebPort: labels.setWebPort };
    if (value !== suggested) controls.push(button(set[key], () => update(key, suggested)));
    if (key === "Port") controls.push(button(labels.refresh, () => update("refresh")));
    card.append(row(label, controls));
  }
  card.append(element("p", "text-body-secondary small", labels.note));
  const [key, label, value, reset] = state.paths[2];
  const replay = element("input", "form-control form-control-sm");
  replay.value = value;
  replay.title = value;
  replay.readOnly = true;
  replay.setAttribute("aria-label", label);
  const actions = [replay, button(labels.browse, () => update(key))];
  if (reset) actions.push(button(labels.reset, () => update("reset_replays")));
  card.append(row(label, actions));
  const launch = panel();
  launch.append(element("h2", "h6", labels.title), element("p", "text-body-secondary", labels.description),
    element("p", "", labels.status), button(labels.setup, () => update("setup")));
  rows.append(card, launch);
  if (state.setup) launchSetup(state.setup);
}

function launchSetup(model) {
  const labels = model.labels;
  const draft = { ...model.draft };
  const dialog = element("dialog", "tracker-detail loadout-dialog");
  const title = element("h2", "h5", labels.title);
  title.id = "launch-setup-title";
  dialog.setAttribute("aria-labelledby", title.id);
  const sendSetup = (key, candidate) => send({ type: "launch_setup", key, draft, candidate });
  const cancel = () => dismissDialog(dialog, () => sendSetup("cancel"));
  const fields = element("div", "d-grid gap-2 mb-3");
  const mode = loadoutSelect(labels.hint, model.modes, draft.mode, value => {
    draft.mode = value;
    refreshFields();
  });
  const input = (key, label) => {
    const row = element("label", "d-grid gap-1");
    const field = element("input", "form-control form-control-sm");
    field.value = draft[key];
    field.addEventListener("input", () => { draft[key] = field.value; });
    row.append(element("span", "", label), field);
    return row;
  };
  function refreshFields() {
    fields.replaceChildren();
    if (draft.mode === "steam_native") fields.append(input("steam_id", labels.steam));
    if (draft.mode === "steam_shortcut") {
      fields.append(element("p", "small text-warning mb-0", labels.warning),
        button(labels.scan, () => sendSetup("scan")));
      if (!model.candidates.length) fields.append(element("p", "small text-body-secondary mb-0", labels.empty));
      model.candidates.forEach((label, index) => fields.append(button(label, () => sendSetup("candidate", index))));
      fields.append(input("steam_id", labels.shortcut));
      if (draft.shortcut_exe) fields.append(element("p", "small text-break mb-0", draft.shortcut_exe));
    }
  }
  refreshFields();
  const actions = element("div", "d-flex justify-content-end gap-2");
  actions.append(button(labels.cancel, cancel), button(labels.save,
    () => dismissDialog(dialog, () => sendSetup("save")), false, "primary"));
  dialog.append(title, element("p", "text-body-secondary", labels.hint), mode, fields, actions);
  openLoadoutModal(dialog, cancel);
}

function appearanceSettings() {
  const labels = state.labels;
  const update = (key, value = null) => send({ type: "appearance", key, value });
  const card = panel();
  const legacy = toggle("Legacy UI", state.legacy, value => update("legacy", value));
  legacy.title = "Applies the next time Hebnix starts.";
  card.append(legacy);
  const line = (label, ...controls) => {
    const row = element("div", "appearance-row");
    row.append(element("span", "", label));
    const values = element("div", "d-flex flex-wrap align-items-center gap-2");
    values.append(...controls);
    row.append(values);
    return row;
  };
  const range = (title, key, value, min, max, step, format, disabled = false) => {
    const slider = element("input", "form-range w-auto");
    const number = element("output", "small");
    slider.type = "range";
    Object.assign(slider, { min, max, step, value, disabled });
    slider.setAttribute("aria-label", title);
    const display = () => { number.textContent = format(Number(slider.value)); };
    display();
    slider.addEventListener("input", display);
    slider.addEventListener("change", () => update(key, Number(slider.value)));
    return line(title, slider, number);
  };
  if (state.themePage) {
    card.append(line(labels.themeLabel, loadoutSelect(labels.themeLabel, state.themes.map(name => [name, name]),
      state.theme, value => update("theme", value)),
      button(labels.refresh, () => update("refresh_theme")),
      button(labels.import, () => update("import_theme")),
      button(labels.folder, () => update("theme_folder")),
      button(labels.fonts, () => update("fonts_folder"))));
    rows.append(card);
    return;
  }
  const general = state.general;
  const hotkey = element("input", "form-control form-control-sm w-auto");
  hotkey.value = general.capturing ? "…" : general.hotkey;
  hotkey.readOnly = true;
  hotkey.setAttribute("aria-label", labels.hotkey);
  card.append(line(labels.hotkey, hotkey, button(labels.capture, () => update("capture"), general.capturing)),
    line(labels.language, loadoutSelect(labels.language, general.languages, general.language, value => update("language", value))),
    range(labels.opacity, "opacity", general.opacity, .5, 1, .01, value => value.toFixed(2)),
    line(labels.toast, loadoutSelect(labels.toast, general.toasts.map(label => [label, label]), general.toast, value => update("toast", value))),
    line(labels.defaultTab, loadoutSelect(labels.defaultTab, general.tabs, general.defaultTab, value => update("default_tab", value))));
  const controller = panel();
  const cfg = state.panel;
  const side = loadoutSelect("Opens from", [["left", "Left"], ["right", "Right"]], cfg.left ? "left" : "right",
    value => update("panel_left", value === "left"));
  side.disabled = !cfg.enabled;
  controller.append(element("h2", "h6", "Controller panel"),
    toggle("Enable controller panel", cfg.enabled, value => update("panel_enabled", value)),
    element("p", "small text-body-secondary", "Open the panel beside the game with L3 + R3."),
    line("Opens from", side),
    range("Width", "panel_width", cfg.width, .2, .5, .01, value => Math.round(value * 100) + "%", !cfg.enabled),
    toggle("Slide in like a drawer", cfg.animate, value => update("panel_animate", value), !cfg.enabled),
    toggle("Show every page (Advanced mode)", cfg.advanced, value => update("panel_advanced", value), !cfg.enabled));
  rows.append(card, controller);
}

function systemSettings() {
  const backups = state.backups;
  const backupAction = key => send({ type: "system_backups", key });
  const command = key => send({ type: "system_command", key });
  const card = panel();

  if (backups.error) card.append(element("p", "text-warning", backups.error));
  const options = panel();
  for (const item of state.rows) {
    options.append(toggle(item.label, item.enabled, enabled => send({ type: "system_toggle", key: item.key, enabled })));
    if (item.note) options.append(element("p", "small text-body-secondary", item.note));
  }
  const maintenanceActions = element("div", "d-flex gap-2 flex-wrap align-items-center mt-3");
  maintenanceActions.append(button(state.repair, () => command("repair"), state.repairBusy),
    button("Uninstall", () => command("uninstall"), false, "outline-danger"),
    button(backups.label, () => backupAction("open"), false, "outline-danger"));
  const fps = element("input", "form-range w-auto");
  const display = element("output", "small");
  fps.type = "range";
  fps.min = "20";
  fps.max = "60";
  fps.step = "1";
  fps.value = String(state.fps);
  display.textContent = fps.value + " fps";
  fps.addEventListener("input", () => { display.textContent = fps.value + " fps"; });
  fps.addEventListener("change", () => send({ type: "system_fps", fps: Number(fps.value) }));
  const slider = element("label", "d-flex flex-wrap align-items-center gap-3 mt-3");
  slider.title = "How often the overlay and plugins update while Rocket League is open.";
  slider.append(element("span", "", "Overlay refresh rate"), fps, display);
  options.append(slider);
  const certificate = state.certificate;
  const proxy = panel();
  proxy.append(element("h2", "h6", "Account HTTPS proxy certificate"),
    element("p", "small text-body-secondary", "Name and Friends use the account HTTPS proxy while Rocket League is running. It routes only api.epicgames.dev and leaves Windows proxy settings unchanged. Administrator access and this certificate are required."));
  const commands = element("div", "d-flex align-items-center flex-wrap gap-2");
  commands.append(element("span", "", certificate.statusLabel + " " + certificate.status),
    button(certificate.action, () => command("certificate")),
    icon("↻", certificate.refresh, () => command("refresh_certificate")));
  proxy.append(commands);
  if (!certificate.admin) proxy.append(element("p", "small text-body-secondary mt-2", "Restart as Administrator to install or remove the certificate."));
  proxy.append(element("p", "mt-2 mb-0", certificate.proxyLabel + " " + certificate.proxy));
  options.append(maintenanceActions);
  window.patchProgress(options, !!state.repairBusy, null, state.repair);
  rows.append(options, proxy);
  if (backups.error) rows.append(card);
  if (backups.confirming) {
    const dialog = element("dialog", "tracker-detail loadout-dialog");
    const title = element("h2", "h5", backups.label);
    title.id = "backups-modal-title";
    dialog.setAttribute("aria-labelledby", title.id);
    const actions = element("div", "d-flex justify-content-end gap-2");
    const cancel = () => dismissDialog(dialog, () => backupAction("cancel"));
    actions.append(button(backups.cancel, cancel), button(backups.confirm,
      () => dismissDialog(dialog, () => backupAction("confirm")), false, "danger"));
    dialog.append(title, element("p", "", backups.note), actions);
    openLoadoutModal(dialog, cancel);
  }
}

function discordSettings() {
  const labels = state.labels;
  const card = panel();
  const set = (key, enabled) => send({ type: "discord_option", key, enabled });
  card.append(toggle(labels.enabled, state.enabled, value => set("enabled", value)),
    toggle(labels.only, state.only, value => set("only", value)),
    element("p", "mb-2 mt-3", labels.message),
    toggle(labels.game, state.game, value => set("game", value)));
  const gameOptions = element("div", "ms-3");
  const keys = ["score", "map", "mode"];
  const count = keys.filter(key => state[key]).length;
  for (const key of keys) {
    gameOptions.append(toggle(labels[key], state[key], value => set(key, value),
      !state.game || state[key] && count <= 1));
  }
  const custom = element("div", "d-flex gap-2 align-items-center");
  const message = element("input", "form-control form-control-sm w-auto");
  message.placeholder = labels.placeholder;
  message.setAttribute("aria-label", labels.custom);
  message.disabled = state.game;
  message.value = state.message;
  message.addEventListener("change", () => send({ type: "discord_message", message: message.value }));
  custom.append(toggle(labels.custom, !state.game, value => set("game", !value)), message);
  card.append(gameOptions, custom);
  if (state.game) card.append(element("p", "text-body-secondary small mb-0", labels.disabled));
  const link = state.link;
  const linkLabels = link.labels;
  const accounts = panel();
  const form = element("form", "d-flex flex-wrap align-items-center gap-2");
  const code = element("input", "form-control form-control-sm w-auto");
  code.value = link.code;
  code.maxLength = 64;
  code.disabled = link.busy;
  code.setAttribute("aria-label", linkLabels.code);
  const submit = element("button", "btn btn-sm btn-primary", linkLabels.button);
  submit.type = "submit";
  submit.disabled = link.busy;
  form.append(element("span", "", linkLabels.code), code, submit);
  form.addEventListener("submit", event => {
    event.preventDefault();
    if (!link.busy) send({ type: "discord_link", code: code.value });
  });
  accounts.append(element("h2", "h6", linkLabels.heading),
    element("p", "text-body-secondary small", linkLabels.instruction), form,
    element("p", "text-body-secondary small mt-2 mb-0", link.status));
  if (link.accounts.length) {
    accounts.append(element("h3", "h6 mt-3", linkLabels.accounts));
    const list = element("ul", "mb-0");
    list.append(...link.accounts.map(name => element("li", "", name)));
    accounts.append(list);
  }
  rows.append(card, accounts);
}

function presets() {
  const labels = state.labels;
  const sendPreset = (type, data = {}) => send({ type: "preset", action: { type, ...data } });
  const capture = panel();
  const form = element("div", "d-flex flex-wrap align-items-center gap-2");
  const name = element("input", "form-control form-control-sm w-auto");
  name.value = state.name;
  name.setAttribute("aria-label", labels.name);
  let patches = state.patches;
  form.append(element("span", "", labels.name), name, toggle(labels.patches, patches, checked => { patches = checked; }),
    button(labels.capture, () => sendPreset("capture", { name: name.value, patches }), false, "primary"));
  capture.append(element("p", "text-body-secondary", labels.intro), form,
    button(labels.refresh, () => sendPreset("refresh")), " ", button(labels.import, () => sendPreset("import_file")));

  if (!state.current) {
    capture.append(element("p", "small text-body-secondary mt-2 mb-0", labels.empty));
  } else {
    const selected = element("div", "d-flex flex-wrap gap-2 mt-3");
    selected.append(loadoutSelect("Preset", state.presets.map((label, index) => [index, label]), state.selected,
      index => sendPreset("select", { index: Number(index) })),
      button("Apply Preset", () => sendPreset("apply"), false, "primary"),
      button("Edit Preset", () => sendPreset("edit")),
      button(labels.delete, () => sendPreset("delete"), false, "outline-danger"));
    capture.append(selected);
  }
  rows.append(capture);

  const draft = state.editing;
  const preset = draft || state.current;
  if (preset) {
    const summary = panel();
    summary.append(element("h2", "h6", draft ? "Edit preset" : preset.name + (preset.include_patches ? " (with patches)" : " (swaps only)")));
    let editedName, editedPatches;
    const saveDraft = () => sendPreset("draft", { name: editedName.value, patches: editedPatches });
    if (draft) {
      editedName = element("input", "form-control form-control-sm w-auto");
      editedName.value = draft.name;
      editedName.setAttribute("aria-label", labels.name);
      editedPatches = draft.include_patches;
      summary.append(editedName, toggle(labels.patches, editedPatches, value => { editedPatches = value; }));
    }
    const listItem = (description, remove) => {
      const line = element("div", "d-flex gap-2 align-items-center justify-content-between border-top py-2");
      line.append(element("span", "text-break", description));
      if (draft) line.append(button("Remove", () => { saveDraft(); remove(); }, false, "outline-danger"));
      summary.append(line);
    };
    preset.swaps.forEach((swap, index) => {
      listItem((swap.source_name || "?") + " → " + (swap.target_name || "?"), () => sendPreset("remove_swap", { index }));
    });
    for (const [kind, value] of Object.entries(preset.patches || {})) {
      if (value === null) continue;
      if (draft && typeof value === "object" && !Array.isArray(value)) {
        for (const [key, item] of Object.entries(value)) {
          listItem(kind + " / " + key + ": " + JSON.stringify(item), () => sendPreset("remove_patch", { kind, key }));
        }
      } else {
        listItem(kind + ": " + JSON.stringify(value), () => sendPreset("remove_patch", { kind }));
      }
    }
    if (draft) summary.append(button("Save Changes", () => { saveDraft(); sendPreset("save"); }, false, "primary"),
      " ", button("Cancel", () => sendPreset("cancel")));
    rows.append(summary);
  }
  if (state.status || state.missing) {
    const status = panel();
    if (state.status) status.append(element("p", "mb-2", state.status));
    if (state.missing) status.append(button("Remove Missing Items from Preset", () => sendPreset("remove_missing")),
      " ", button("Ignore Missing Items", () => sendPreset("ignore_missing")));
    rows.append(status);
  }
  const share = panel();
  const key = element("textarea", "form-control mb-2");
  key.rows = 3;
  key.value = state.key;
  key.setAttribute("aria-label", "Preset Key");
  share.append(element("h2", "h6", "Preset Key"),
    element("p", "text-body-secondary", "Share or paste a base64 preset key. Imported items remain listed even when their local files are missing."), key,
    button("Show Preset Key", () => sendPreset("show_key"), !state.current),
    " ", button("Import Preset", () => sendPreset("import_key", { key: key.value })));
  rows.append(share);
}

function render() {
  const creatorFocus = document.activeElement?.id?.startsWith("creator-") ? document.activeElement : null;
  const creatorSelection = creatorFocus && creatorFocus.type !== "number" ? [creatorFocus.selectionStart, creatorFocus.selectionEnd] : null;
  const layout = rows.querySelector(".plugin-settings-layout");
  const samePlugin = state.page === "plugin-settings" && layout?.dataset.plugin === String(state.selected);
  const navigationScroll = layout?.firstElementChild.scrollTop || 0;
  const settingsScroll = samePlugin ? layout.lastElementChild.scrollTop : 0;
  const focused = samePlugin && layout.contains(document.activeElement) ? document.activeElement : null;
  const inputs = focused ? [...layout.querySelectorAll("input,select,textarea,button,summary")] : [];
  const focusIndex = inputs.indexOf(focused);
  const selection = focused && ["text", "password", "search"].includes(focused.type)
    ? [focused.selectionStart, focused.selectionEnd] : null;
  renderContent();
  // A page-level fallback covers every patch model with a busy flag. Cards
  // containing separate patch workers add their own bars below their controls.
  if (!rows.querySelector(".patch-progress, progress"))
    window.patchProgress(rows, !!state.busy, state.progress, state.status || "Applying patch");
  if (creatorFocus) { const input = document.getElementById(creatorFocus.id); input?.focus({preventScroll:true}); if (input && creatorSelection) input.setSelectionRange(...creatorSelection); }
  const next = rows.querySelector(".plugin-settings-layout");
  if (next) {
    if (focusIndex >= 0) {
      const control = next.querySelectorAll("input,select,textarea,button,summary")[focusIndex];
      control?.focus({preventScroll: true});
      if (selection && control?.setSelectionRange) control.setSelectionRange(...selection);
    }
    next.firstElementChild.scrollTop = navigationScroll;
    next.lastElementChild.scrollTop = settingsScroll;
  }
}

function renderContent() {
  const scroll = rows.querySelector(".loadout-dialog")?.scrollTop;
  document.body.classList.toggle("preset-creator-page", state.page === "preset-creator");
  controls.replaceChildren();
  rows.replaceChildren();
  rows.inert = false;
  document.getElementById("menu-lock")?.remove();
  controls.classList.add("mb-3");
  query.hidden = state.page !== "installed" && state.page !== "browse";
  field("plugin-search-row").hidden = query.hidden;
  field("plugin-refresh").replaceChildren();
  document.body.classList.toggle("plugin-settings-page", state.page === "plugin-settings");
  const term = query.value.trim().toLowerCase();
  const list = (state.rows || []).filter(plugin => [plugin.name, plugin.author, plugin.description]
    .some(value => typeof value === "string" && value.toLowerCase().includes(term)));
  status.textContent = state.error || (state.loading ? "Loading…" : !query.hidden && !list.length ? "No plugins found." : "");
  status.hidden = !status.textContent;

  rows.classList.remove("player-name-layout");
  if (state.page === "player-name") return playerName();
  if (state.page === "player-friends") return friendNames();
  if (state.page === "player-scoreboard") return scoreboard();
  if (state.page === "action-button") return actionButtons();
  if (state.page === "freeplay") return freeplay();
  if (state.page === "workshop-menu") return workshopMenu();
  if (state.page === "multiplayer") return multiplayer();
  if (state.page === "menu-map") return menuMap();
  if (state.page === "add-maps") return addMaps();
  if (state.page === "game-mods") return gameMods();
  if (state.page === "live-colours") return gameColours();
  if (state.page === "live-goal-explosion") return rows.append(liveGoalExplosion());
  if (state.page === "live-wheels") return rows.append(liveCosmeticEditor("alignment"));
  if (state.page === "replay-library") return replayLibrary();
  if (state.page === "presets") return presets();
  if (state.page === "discord-settings") return discordSettings();
  if (state.page === "system-settings") return systemSettings();
  if (state.page === "appearance-settings") return appearanceSettings();
  if (state.page === "files-settings") return filesSettings();
  if (state.page === "speed-patch") return speedPatch();
  if (state.page === "goal-explosion") return goalExplosion();
  if (state.page === "wheel-alignment") return wheelAlignment();
  if (state.page === "game-colours") return gameColours();
  if (state.page === "ball-appearance") return ballAppearance();
  if (state.page === "studio-active") return studioActive();
  if (state.page === "studio-asset") return studioAsset();
  if (state.page === "menu-textures") return menuTextures();
  if (state.page === "maintenance") return maintenance();
  if (state.page === "controller-menu") return controllerMenu();
  if (state.page === "plugin-window") return floatingPlugin();
  if (state.page === "upk-history") return upkHistory();
  if (state.page === "anthems") return anthems();
  if (state.page === "item-artwork") return itemArtwork();
  if (state.page === "plugin-settings") return pluginSettings();
  if (state.page === "player-rank") return playerRank();
  if (state.page === "profile-pictures") return profilePictures();
  if (state.page === "titles") return titles();
  if (state.page === "preset-creator") return presetCreator();
  if (state.page === "loadouts") {
    loadouts();
    const dialog = rows.querySelector(".loadout-dialog");
    if (dialog && scroll !== undefined) dialog.scrollTop = scroll;
    return;
  }
  if (state.page === "installed") {
    controls.append(pluginAction("folder", "Plugins Folder", () => command("folder")),
      pluginAction("zip", "Install from ZIP", () => command("zip")),
      pluginAction("reload", "Reload", () => command("reload")));
    const tools = [...controls.children];
    controls.replaceChildren();
    field("plugin-refresh").append(...tools);
    if (state.overlay.length) rows.append(overlayOrder(state.overlay));
    rows.append(...list.map(installedRow));
    return;
  }

  field("plugin-refresh").append(pluginAction("refresh", "Refresh", () => command("refresh"), state.loading));
  const pages = Math.max(1, Math.ceil(list.length / pageSize));
  page = Math.min(page, pages - 1);
  const visible = list.slice(page * pageSize, (page + 1) * pageSize);
  const ids = visible.map(plugin => plugin.id);
  const signature = JSON.stringify(ids);
  if (requested !== signature) {
    requested = signature;
    send({ type: "plugin_previews", ids });
  }
  const grid = element("div", "plugin-catalog-grid");
  grid.append(...visible.map(catalogCard));
  rows.append(grid);
  if (pages > 1) {
    const paging = element("nav", "d-flex align-items-center justify-content-center gap-2 mt-3");
    paging.append(button("Previous", () => { page--; render(); }, page === 0),
      element("span", "small", (page + 1) + " / " + pages),
      button("Next", () => { page++; render(); }, page + 1 >= pages));
    rows.append(paging);
  }
}

query.addEventListener("input", () => { page = 0; render(); });
window.showManager = data => {
  if (state?.page !== data.page) {
    query.value = "";
    query.placeholder = "Search";
    query.setAttribute("aria-label", "Search");
    page = 0;
    pendingRemoval = null;
    requested = "";
    loadoutView.previews = "";
    loadoutView.importOpen = false;
    loadoutView.importSubmitted = false;
  }
  if (data.page === "loadouts" && loadoutView.importSubmitted) {
    loadoutView.importSubmitted = false;
    if (!data.input && !data.import) {
      const dialog = rows.querySelector(".loadout-dialog");
      if (dialog) {
        state = data;
        closeLoadoutImport(dialog);
        return;
      }
      loadoutView.importOpen = false;
    }
  }
  const editingName = state?.page === "player-name" && data.page === state.page &&
    rows.contains(document.activeElement) && document.activeElement.matches("input[type=text]");
  const editingLive = state?.page === data.page && document.activeElement?.closest(".live-cosmetics") &&
    document.activeElement.matches("input");
  if (state?.page !== data.page && data.page === "anthems") anthemView.requested = "";
  if (state?.page !== data.page && data.page === "item-artwork") artworkView.key = "";
  if (state?.page !== data.page && data.page === "studio-asset") studioAssetView.requested = "";
  if (state?.page === "anthems" && data.page === "anthems" && state.editor && data.editor && anthems.updateCursor) {
    const { position: previousPosition, ...previousEditor } = state.editor;
    const { position, ...nextEditor } = data.editor;
    if (previousPosition !== position && state.busy === data.busy && state.status === data.status && state.error === data.error &&
        state.closed === data.closed && JSON.stringify(previousEditor) === JSON.stringify(nextEditor)) {
      state = data;
      anthems.updateCursor(position);
      return;
    }
  }
  if (data.page !== "player-rank") rankView.draft = null;
  const editingRank = state?.page === "player-rank" && data.page === "player-rank" && rankView.draft !== null;
  if (data.page === "loadouts" && state?.page === "loadouts" && state.input && !data.input) loadoutView.draft = null;
  if (creatorView.creating && data.creatorAttempt !== creatorView.attempt) creatorView.creating=false;
  if (state?.creator && !data.creator && data.page === "preset-creator" && data.message === "Loadout Created") {
    creatorView.createdUntil = Date.now() + 10000;
    clearTimeout(creatorView.createdTimer);
    creatorView.createdTimer = setTimeout(() => { if (state?.page === "preset-creator" && !state.creator) render(); }, 10000);
  }
  state = data;
  if (editingLive && data.live) for (const banner of rows.querySelectorAll(".live-setup-status")) {
    banner.textContent = "Live cosmetics: " + (data.live.status || "Starting…");
  }
  if (!editingName && !editingLive && !editingRank) render();
};
window.ipc.postMessage(JSON.stringify({ type: "manage_ready" }));
