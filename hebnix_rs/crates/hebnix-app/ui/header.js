const field = (id) => document.getElementById(id);
const send = (action) => window.ipc.postMessage(JSON.stringify({ type: "header", action }));
const body = field("panel-body");
let state = null;
let signature = "";
let notificationFilter = "";
let networkTab = "friends";
let codeDraft = "";
let confirmation = null;
const openDetails = new Set();
const selectedRanks = new Map();

const element = (tag, classes, label = "") => {
  const node = document.createElement(tag);
  node.className = classes;
  node.textContent = label;
  return node;
};
function button(label, run, disabled = false, classes = "btn btn-sm btn-outline-secondary") {
  const node = element("button", classes, label);
  node.type = "button";
  node.disabled = disabled;
  node.addEventListener("click", run);
  return node;
}
function panel(value) {
  const next = state?.panel === value ? null : value;
  const apply = () => send({ type: "panel", panel: next, action_inset: next === "actions" ? actionInset() : 0,
    action_width: next === "actions" ? actionWidth() : 180 });
  if (state?.panel) animatePanelClose(apply);
  else apply();
}
function animatePanelClose(complete) {
  const popup = field("panel");
  if (popup.classList.contains("is-closing")) return;
  popup.classList.add("is-closing");
  const animation = popup.getAnimations().find(animation => animation.animationName === "popup-close");
  if (animation) animation.finished.then(complete, complete);
  else complete();
}
function actionInset() {
  return Math.max(0, Math.round(document.documentElement.clientWidth - field("actions").getBoundingClientRect().right));
}
const actionMeasure = document.createElement("canvas").getContext("2d");
function actionWidth() {
  if (!actionMeasure || !state?.actions.length) return 180;
  const font = getComputedStyle(field("primary"));
  actionMeasure.font = font.fontWeight + " " + font.fontSize + " " + font.fontFamily;
  const label = Math.max(0, ...state.actions.slice(1).map(item => actionMeasure.measureText(item.label).width));
  return Math.min(720, Math.max(100, Math.ceil(label + 42)));
}
function syncActionAnchor() {
  if (state?.panel !== "actions") return;
  const inset = actionInset();
  const width = actionWidth();
  if (inset !== state.action_inset || width !== state.action_width) send({ type: "action_anchor", inset, width });
}
window.addEventListener("resize", syncActionAnchor);
function network(action) {
  send({ type: "network", action });
}
field("primary").addEventListener("click", () => {
  if (state?.actions.length) send({ type: "run", action: state.actions[0].action });
});
field("more").addEventListener("click", () => panel("actions"));
field("network").addEventListener("click", () => panel("network"));
field("notifications").addEventListener("click", () => panel("notifications"));
field("close").addEventListener("click", () => panel(null));
document.querySelector(".workspace-header").addEventListener("pointerdown", (event) => {
  if (state?.panel && !event.target.closest("button")) panel(null);
});
document.addEventListener("keydown", event => {
  if (event.key === "Escape" && state?.panel) panel(null);
});

function notifications() {
  const data = state.notifications;
  const toolbar = element("div", "d-flex gap-2 mb-2");
  const filter = element("select", "form-select form-select-sm");
  filter.setAttribute("aria-label", "Filter notifications");
  filter.append(new Option(data.all, ""), ...data.plugins.map(plugin => new Option(plugin.name + " (" + plugin.count + ")", plugin.slug)));
  if (!data.plugins.some(plugin => plugin.slug === notificationFilter)) notificationFilter = "";
  filter.value = notificationFilter;
  filter.addEventListener("change", () => { notificationFilter = filter.value; renderPanel(); });
  toolbar.append(filter, button(data.clear, () => send({ type: "clear_notifications" }), !data.entries.length));
  body.append(toolbar);
  const entries = data.entries.filter(entry => !notificationFilter || notificationFilter === entry.plugin);
  if (!entries.length) body.append(element("p", "text-body-secondary mb-0", data.empty));
  entries.forEach(entry => {
    const details = element("details", "border-top py-2");
    const key = "toast:" + entry.id;
    details.open = openDetails.has(key);
    details.addEventListener("toggle", () => updateOpen(key, details.open));
    details.append(element("summary", "fw-medium text-break", entry.name + ": " + entry.preview),
      element("div", "small text-body-secondary mt-2", entry.time),
      element("div", "notification-text mt-1 text-break", entry.text));
    body.append(details);
  });
}

function updateOpen(key, open) {
  if (open) openDetails.add(key); else openDetails.delete(key);
}

function networkPlayer(player) {
  const data = state.network;
  const details = element("details", "border-top py-2");
  const key = "friend:" + player.id;
  details.open = openDetails.has(key);
  details.addEventListener("toggle", () => { updateOpen(key, details.open); requestRanks(); });
  details.append(element("summary", "text-break", player.name + (player.status === "accepted" ? " · " + player.presence : "")));
  if (player.status === "accepted") {
    player.links.forEach(link => {
      const group = element("div", "my-2");
      group.append(element("div", "small text-body-secondary", [link.name, link.platform].filter(Boolean).join(" · ")));
      if (link.error) group.append(element("div", "small text-warning", link.error));
      else if (link.ranks.length) {
        const select = element("select", "form-select form-select-sm");
        select.setAttribute("aria-label", "Rank playlist");
        select.append(...link.ranks.map(rank => new Option(rank.mode + " · " + rank.rank, rank.mode)));
        const rankKey = key + ":" + link.index;
        select.value = selectedRanks.get(rankKey) || link.ranks[0].mode;
        select.addEventListener("change", () => selectedRanks.set(rankKey, select.value));
        group.append(select);
      } else group.append(element("small", "text-body-secondary", link.loaded ? "No ranked playlists" : "Ranks not loaded"));
      details.append(group);
    });
  }
  const controls = element("div", "d-flex gap-2 mt-2 flex-wrap");
  const operations = player.status === "accepted" ? [["remove", "Remove Friend"], ["block", "Block"]]
    : player.status === "blocked" ? [["unblock", "Unblock"]]
    : player.direction === "incoming" ? [["accept", "Accept"], ["deny", "Deny"], ["block", "Block"]] : [["block", "Block"]];
  if (player.status === "pending" && player.direction !== "incoming") controls.append(element("small", "text-body-secondary", "Request sent"));
  operations.forEach(([operation, name]) => {
    const needsConfirm = operation === "block" || operation === "remove";
    const pending = confirmation?.id === player.id && confirmation?.operation === operation;
    controls.append(button(pending ? "Confirm " + name.toLowerCase() : name, () => {
      if (needsConfirm && !pending) { confirmation = { id: player.id, operation }; renderPanel(); }
      else { confirmation = null; network({ type: "friend", id: player.id, operation }); }
    }, !data.enabled));
    if (pending) controls.append(button("Cancel", () => { confirmation = null; renderPanel(); }));
  });
  details.append(controls);
  return details;
}

function requestRanks() {
  if (state?.panel !== "network" || networkTab !== "friends" || !state.network?.enabled) return;
  for (const player of state.network.players) {
    if (player.status !== "accepted" || !openDetails.has("friend:" + player.id)) continue;
    const link = player.links.find(link => !link.loaded);
    if (link) { network({ type: "ranks", id: player.id, index: link.index }); return; }
  }
}

function networkPanel() {
  const data = state.network;
  if (!data) return;
  if (!data.signed_in && networkTab === "add") networkTab = "friends";
  const tabs = element("nav", "network-tabs mb-3");
  const choices = [["friends", "Friends"], ["pending", "Pending"], ["settings", "Settings"]];
  if (data.signed_in) choices.splice(2, 0, ["add", "Add Friend"]);
  choices.forEach(([value, label]) => {
    const tab = button(label, () => {
      networkTab = value; confirmation = null; renderPanel();
      if ((value === "settings" || value === "add") && data.signed_in && !data.friend_code && data.enabled) network({ type: "load_settings" });
    }, false, "btn btn-sm network-tab");
    tab.classList.toggle("active", value === networkTab);
    tab.setAttribute("aria-pressed", String(value === networkTab));
    tabs.append(tab);
  });
  body.append(tabs);
  if (data.error) body.append(element("p", "text-warning mb-2", data.error));
  if (networkTab === "settings") {
    if (data.signed_in && data.presence) {
      const label = element("label", "form-label d-block", "Presence");
      const select = element("select", "form-select form-select-sm");
      select.append(new Option("Online for Friends", "friends"), new Option("Offline", "offline"));
      select.value = data.presence === "offline" ? "offline" : "friends";
      select.disabled = !data.enabled;
      select.addEventListener("change", () => network({ type: "presence", value: select.value }));
      label.append(select); body.append(label);
    } else if (data.signed_in) body.append(button(data.busy ? "Loading settings…" : "Load Network settings", () => network({ type: "load_settings" }), !data.enabled));
    [["hide_match_users", "Don’t Notify About Hebnix Users In Game"], ["hide_presence", "Hide All Presence Notifications"]].forEach(([key, label]) => {
      const row = element("label", "d-flex align-items-center gap-2 my-2", label);
      const input = element("input", "form-check-input mt-0");
      input.type = "checkbox"; input.checked = data[key];
      input.addEventListener("change", () => network({ type: "notifications", hide_match_users: key === "hide_match_users" ? input.checked : data.hide_match_users, hide_presence: key === "hide_presence" ? input.checked : data.hide_presence }));
      row.prepend(input); body.append(row);
    });
    return;
  }
  if (networkTab === "add") {
    const label = element("label", "form-label d-block", "Enter Friend Code");
    const input = element("input", "form-control form-control-sm");
    input.maxLength = 8; input.value = codeDraft;
    const submit = button("Send friend request", () => { network({ type: "add", code: codeDraft }); codeDraft = ""; input.value = ""; submit.disabled = true; }, !data.enabled || codeDraft.length !== 8);
    input.addEventListener("input", () => {
      codeDraft = input.value.replace(/[^a-z0-9]/gi, "").toUpperCase();
      input.value = codeDraft; submit.disabled = !data.enabled || codeDraft.length !== 8;
    });
    label.append(input); body.append(label, submit);
    if (data.friend_code) body.append(element("p", "mt-2 mb-0", "Your Friend Code · " + data.friend_code));
    else body.append(button(data.busy ? "Loading friend code…" : "Load friend code", () => network({ type: "load_settings" }), !data.enabled));
    return;
  }
  if (!data.signed_in) {
    body.append(button(data.busy ? "Waiting for Discord sign-in…" : "Sign in with Discord", () => network({ type: "sign_in" }), !data.enabled));
    return;
  }
  (data.warnings || []).forEach(warning => body.append(element("p", "text-warning mb-2", warning)));
  const statuses = networkTab === "friends" ? ["accepted"] : ["pending", "blocked"];
  const players = data.players.filter(player => statuses.includes(player.status));
  if (!players.length) body.append(element("p", "text-body-secondary mb-0", data.busy ? "Loading Network…" : networkTab === "friends" ? "No friends yet." : "No pending requests or blocked players."));
  players.forEach(player => body.append(networkPlayer(player)));
  requestRanks();
}

function renderPanel() {
  body.replaceChildren();
  if (state.panel === "notifications") notifications();
  if (state.panel === "network") networkPanel();
  if (state.panel === "actions") state.actions.slice(1).forEach(action => {
    const item = button(action.label, () => animatePanelClose(() => send({ type: "run", action: action.action })), false, "btn action-menu-item");
    item.title = action.label;
    body.append(item);
  });
}
window.showHeader = (data) => {
  const previousPanel = state?.panel;
  state = data;
  field("panel-back").hidden = !data.controller;
  field("workspace-title").textContent = data.title;
  field("workspace-title").title = data.title;
  field("workspace-title").classList.toggle("me-auto", !data.page_warning);
  field("page-warning").textContent = data.page_warning || "";
  field("page-warning").title = data.page_warning || "";
  field("page-warning").hidden = !data.page_warning;
  field("actions").hidden = !data.actions.length;
  field("primary").textContent = data.actions[0]?.label || "";
  field("more").hidden = data.actions.length < 2;
  const status = data.connected ? data.status.replace(/^✔\s*/, "") : data.status;
  field("rl-status").textContent = status;
  field("rl-status").title = status;
  field("rl-status-check").hidden = !data.connected;
  field("rl-status-group").classList.toggle("is-connected", data.connected);
  field("unread").textContent = data.unread || "";
  field("unread").hidden = !data.unread;
  ["network", "notifications", "more"].forEach(id => field(id).setAttribute("aria-expanded", String(data.panel === (id === "more" ? "actions" : id))));
  if (previousPanel !== data.panel) field("panel").classList.remove("is-closing");
  field("panel").hidden = !data.panel;
  field("panel").dataset.kind = data.panel || "";
  field("panel").style.setProperty("--action-inset", (data.action_inset || 0) + "px");
  field("panel").style.setProperty("--action-width", (data.action_width || 180) + "px");
  field("panel-heading").classList.toggle("d-none", data.panel === "actions");
  syncActionAnchor();
  field("panel-title").textContent = { actions: "Actions", network: "Network", notifications: data.notifications_label }[data.panel] || "";
  const next = JSON.stringify([data.panel, data.notifications, data.network, data.panel === "actions" ? data.actions : null]);
  if (signature !== next) { signature = next; renderPanel(); }
};
field("panel-back").addEventListener("click", () => window.ipc.postMessage(JSON.stringify({ type: "panel_back" })));

window.ipc.postMessage(JSON.stringify({ type: "header_ready" }));
