const field = id => document.getElementById(id);
const send = action => window.ipc.postMessage(JSON.stringify({ type: "arena", action }));
const sideNames = ["donor", "host"];
let state;
let request = "";
const selected = { donor: "", host: "" };
const signatures = { donor: "", host: "" };
let previews = {};
const noBackground = "__HBNX_NO_BACKGROUND__";
const renderedPreviews = new WeakMap();
let visibilityFrame = 0;

function placeholderIcon(empty) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS(svg.namespaceURI, "path");
  path.setAttribute("d", empty
    ? "M20.5 12a8.5 8.5 0 1 1-17 0 8.5 8.5 0 0 1 17 0ZM6 6l12 12"
    : "M3 5h18v14H3zM3 16l5-5 4 4 3-3 6 6M16 9h.01");
  svg.append(path);
  return svg;
}

function tile(item, side) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "btn swapper-tile";
  button.classList.toggle("active", selected[side] === item.id);
  button.setAttribute("aria-pressed", String(selected[side] === item.id));
  button.title = item.name;
  button.dataset.arena = item.id;
  const artwork = document.createElement("span");
  artwork.className = "swapper-tile-art";
  const caption = document.createElement("span");
  caption.className = "swapper-tile-caption";
  const label = document.createElement("span");
  label.textContent = item.name;
  caption.append(label);
  button.append(artwork, caption);
  button.addEventListener("click", () => {
    selected[side] = item.id;
    field("bulk-confirm").hidden = true;
    refreshPickers();
  });
  return button;
}

function updateArtwork(card) {
  const id = card.dataset.arena;
  const preview = previews[id];
  if (renderedPreviews.has(card) && renderedPreviews.get(card) === preview) return;
  renderedPreviews.set(card, preview);
  const artwork = card.firstElementChild;
  artwork.replaceChildren();
  if (id === noBackground || preview === null) {
    artwork.append(placeholderIcon(id === noBackground));
  } else if (preview) {
    const image = document.createElement("img");
    image.src = preview;
    image.alt = "";
    image.decoding = "async";
    artwork.append(image);
  } else {
    artwork.textContent = "Loading preview…";
  }
}

function fitColumns() {
  for (const side of sideNames) {
    const picker=field(side+"s");
    const available=picker.clientWidth-8;
    const columns=Math.max(2,Math.min(5,Math.floor((available+8)/128)));
    picker.style.gridTemplateColumns=`repeat(${columns},minmax(0,1fr))`;
  }
}

function refreshPickers() {
  for (const side of sideNames) {
    const rows = state[side + "s"];
    if (!rows.some(row => row.id === selected[side])) selected[side] = "";
    const query = field(side + "-search").value.trim().toLowerCase();
    const filtered = rows.filter(row => row.name.toLowerCase().includes(query));
    const signature = JSON.stringify(filtered);
    const grid = field(side + "s");
    if (signatures[side] !== signature) {
      signatures[side] = signature;
      const scrollTop = grid.scrollTop;
      const focused = grid.contains(document.activeElement) ? document.activeElement.dataset.arena : undefined;
      grid.replaceChildren(...filtered.map(card => tile(card, side)));
      fitColumns(grid);
      grid.scrollTop = scrollTop;
      [...grid.children].find(card => card.dataset.arena === focused)?.focus({ preventScroll: true });
    }
    for (const card of grid.children) {
      const active = selected[side] === card.dataset.arena;
      card.classList.toggle("active", active);
      card.setAttribute("aria-pressed", String(active));
      updateArtwork(card);
    }
  }
  scheduleVisiblePreviews();
  const isRemoval = selected.donor === noBackground;
  field("apply").textContent = isRemoval ? "Remove background" : "Apply";
  field("apply").disabled = state.busy || state.running || !selected.donor || !selected.host || selected.donor.replace(/^(arena|full-map):/, "") === selected.host;
  field("bulk").disabled = state.busy || state.running || !selected.donor || isRemoval || !state.bulk_targets;
  field("bulk-apply").disabled = field("bulk").disabled;
}
function requestVisiblePreviews() {
  if (!state) return;
  const ids = new Set();
  for (const side of sideNames) {
    const grid = field(side + "s");
    const frame = grid.getBoundingClientRect();
    for (const card of grid.children) {
      const id = card.dataset.arena;
      if (id === noBackground || Object.hasOwn(previews, id)) continue;
      const bounds = card.getBoundingClientRect();
      if (bounds.bottom > frame.top && bounds.top < frame.bottom) ids.add(id);
    }
  }
  const next = JSON.stringify([...ids]);
  if (next !== request) {
    request = next;
    send({ type: "preview", ids: [...ids] });
  }
}
function scheduleVisiblePreviews() {
  if (visibilityFrame) return;
  visibilityFrame = requestAnimationFrame(() => {
    visibilityFrame = 0;
    requestVisiblePreviews();
  });
}
for (const side of sideNames) {
  const grid = field(side + "s");
  field(side + "-search").addEventListener("input", () => { grid.scrollTop = 0; refreshPickers(); });
  grid.addEventListener("scroll", scheduleVisiblePreviews, { passive: true });
}
window.addEventListener("resize", scheduleVisiblePreviews);
field("apply").addEventListener("click", () => send({ type: "apply", host: selected.host, donor: selected.donor }));
field("refresh").addEventListener("click", () => {
  previews = {};
  send({ type: "refresh" });
});
field("restore-all").addEventListener("click", () => send({ type: "restore_all" }));
field("folder").addEventListener("click", () => send({ type: "folder" }));
field("bulk").addEventListener("click", () => { field("bulk-confirm").hidden = false; });
function closeBulkConfirmation(complete = () => {}) {
  const popup = field("bulk-confirm");
  if (popup.hidden || popup.classList.contains("is-closing")) return;
  popup.classList.add("is-closing");
  const done = () => {
    popup.hidden = true;
    popup.classList.remove("is-closing");
    complete();
  };
  const animation = popup.getAnimations().find(animation => animation.animationName === "popup-close");
  if (animation) animation.finished.then(done, done);
  else done();
}
field("bulk-cancel").addEventListener("click", () => closeBulkConfirmation());
field("bulk-apply").addEventListener("click", () => {
  const donor = selected.donor;
  closeBulkConfirmation(() => send({ type: "apply_all", donor }));
});

window.showArenaEditor = next => {
  if (state?.full !== next.full) {
    selected.donor = selected.host = "";
    field("donors").scrollTop = field("hosts").scrollTop = 0;
    for (const side of sideNames) field(side + "-search").value = "";
    field("bulk-confirm").hidden = true;
    request = "";
    previews = {};
  }
  state = next;
  Object.assign(previews, state.previews);
  field("map-instructions").hidden = false;
  field("map-instructions").textContent = state.full ? "Choose the map you want to play, then choose the arena it will replace entirely." : "Choose the arena you want to play, then choose the fog, sky, and background to borrow.";
  field("donor-label").textContent = state.full ? "Map you want" : "Background you want";
  field("donor-search").placeholder = state.full ? "Search maps" : "Search backgrounds";
  field("folder").hidden = field("bulk").hidden = state.full;
  field("bulk-warning").textContent = state.bulk_warning;
  field("status").textContent = state.running ? "Close Rocket League before applying or restoring changes."
    : !state.changed || /^Restored .+original background\./.test(state.status || "") ? "" : state.status;
  field("active").hidden = !state.active.length;
  field("restore-all").disabled = state.running || state.busy;
  field("active-list").replaceChildren(...state.active.map(([id, name, donor]) => {
    const row = document.createElement("div");
    row.className = "d-flex justify-content-between align-items-center gap-2 py-2 border-top";
    const label = document.createElement("span");
    label.textContent = donor + " → " + name;
    const restore = document.createElement("button");
    restore.type = "button";
    restore.className = "btn btn-sm btn-outline-secondary";
    restore.textContent = "Restore";
    restore.disabled = state.busy || state.running;
    restore.addEventListener("click", () => send({ type: "restore", host: id }));
    row.append(label, restore);
    return row;
  }));
  refreshPickers();
};

new ResizeObserver(() => { if (state) fitColumns(field("hosts")); }).observe(field("hosts"));
window.ipc.postMessage(JSON.stringify({ type: "arena_ready" }));
