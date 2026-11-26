const field = (id) => document.getElementById(id);
const send = (action) => window.ipc.postMessage(JSON.stringify({ type: "swapper", action }));
let state = null;
let currentCategory = null;
let sourceId = "";
let targetId = "";
let choosingReplacement = false;
let page = 0;
let requestedPreviews = "";
let pickerSignature = "";
let columns = 6;
let mode = null;
const grid = field("picker");

function options(select, rows, value, placeholder) {
  const list = placeholder ? [new Option(placeholder, "")] : [];
  rows.forEach(([id, name]) => list.push(new Option(name, id)));
  select.replaceChildren(...list);
  if (list.some(option => option.value === String(value))) select.value = value;
}

function compatible(source, target) {
  if (!source || source.id === target.id) return false;
  if (state.decals && field("match-car").checked && source.car !== target.car) return false;
  if (field("owned").checked && !target.owned) return false;
  if (!state.goals) return true;
  if (target.kind === "2parts") return source.kind === "2parts";
  if (target.kind === "3parts") return source.kind === "3parts" && source.package.toLowerCase() === target.package.toLowerCase();
  return source.kind === "simple" || source.kind === "2parts";
}

function matches(item, query) {
  return !query || item.label.toLowerCase().includes(query) || item.package.toLowerCase().includes(query);
}

function itemCard(item, selected) {
  const tile = document.createElement("button");
  tile.type = "button";
  tile.className = "btn swapper-tile";
  tile.classList.toggle("active", selected);
  tile.setAttribute("aria-pressed", String(selected));
  tile.title = item.label + "\n" + item.package;
  const art = document.createElement("span");
  art.className = "swapper-tile-art";
  const preview = state.catalog?.previews?.[item.id];
  if (preview?.image) {
    const picture = document.createElement("img");
    picture.src = preview.image;
    picture.alt = "";
    art.append(picture);
  } else {
    art.textContent = preview?.status || "Loading preview…";
    if (preview?.error) art.title = preview.error;
  }
  const caption = document.createElement("span");
  caption.className = "swapper-tile-caption";
  const name = document.createElement("span");
  name.textContent = item.label;
  caption.append(name);
  const flags = mode === "active"
    ? ["Replaces " + item.target, item.paint].filter(Boolean)
    : [item.owned ? "Owned" : "", item.applied ? "Applied" : ""].filter(Boolean);
  if (flags.length) {
    const label = document.createElement("small");
    label.textContent = flags.join(" · ");
    caption.append(label);
  }
  if (mode === "active") {
    tile.classList.add("swapper-restore-tile");
    tile.disabled = state.game_running;
    tile.setAttribute("aria-label", "Restore " + item.target);
    const restore = document.createElement("small");
    restore.textContent = "Restore";
    caption.append(restore);
    tile.append(art, caption);
    tile.addEventListener("click", () => send({ type: "restore", key: item.id }));
    return tile;
  }
  tile.append(art, caption);
  tile.addEventListener("click", () => {
    if (choosingReplacement) targetId = item.id;
    else { sourceId = item.id; targetId = ""; }
    renderEditor();
  });
  return tile;
}

function renderEditor() {
  const items = state.catalog?.items || [];
  const cars = (state.catalog?.cars || []).filter(car => !field("owned").checked || car.owned || car.universal);
  const previousCar = field("car").value;
  const car = cars.find(row => row.id === previousCar) || cars.find(row => row.universal) || cars[0];
  options(field("car"), cars.map(row => [row.id, row.label]), car?.id);
  const query = field("source-search").value.trim().toLowerCase();
  const sources = items.filter(item => matches(item, query)
    && (!state.decals || item.car === car?.id));
  const source = (choosingReplacement ? items : sources).find(item => item.id === sourceId);
  if (!source) {
    sourceId = targetId = "";
    if (choosingReplacement) { choosingReplacement = false; page = 0; }
  }
  const targetQuery = field("target-search").value.trim().toLowerCase();
  const matchesForStep = choosingReplacement
    ? items.filter(item => matches(item, targetQuery) && compatible(source, item))
    : sources;
  if (choosingReplacement && !matchesForStep.some(item => item.id === targetId)) targetId = "";
  field("source-search").hidden = choosingReplacement;
  field("target-search").hidden = !choosingReplacement;
  field("car-fields").hidden = !state.decals || choosingReplacement;
  field("match-car-label").hidden = !state.decals;
  field("back").hidden = !choosingReplacement;
  field("choose-replacement").hidden = choosingReplacement;
  field("choose-replacement").disabled = !source;
  field("paint-fields").hidden = !choosingReplacement || !state.paint;
  field("speed-fields").hidden = !choosingReplacement || !state.speed;
  field("apply").hidden = !choosingReplacement;
  field("picker-banner").textContent = choosingReplacement
    ? (field("owned").checked ? "Select an item you own to replace with " : "Select the item to replace with ") + source.label
    : "Select the item you want to use";
  renderPicker(matchesForStep, choosingReplacement ? targetId : sourceId,
    choosingReplacement ? "No compatible replacements match these filters." : "No items match these filters.");
  field("apply").disabled = !sourceId || !targetId || state.game_running;
}

function renderPicker(items, selected, emptyText) {
  grid.hidden = items.length === 0;
  field("picker-empty").hidden = items.length > 0;
  field("picker-empty").textContent = emptyText;
  const measured = Math.max(1, Math.min(12, Math.floor((grid.clientWidth + 10) / 164) - 1));
  if (grid.clientWidth) columns = measured;
  grid.style.setProperty("--swapper-columns", columns);
  const pageSize = columns * 3;
  const pages = Math.max(1, Math.ceil(items.length / pageSize));
  page = Math.max(0, Math.min(page, pages - 1));
  field("picker-pages").hidden = items.length === 0;
  field("page-label").textContent = (page + 1) + " / " + pages;
  field("prev-page").disabled = page === 0;
  field("next-page").disabled = page + 1 >= pages;
  const visible = items.slice(page * pageSize, (page + 1) * pageSize);
  const ids = visible.map(item => item.id);
  const signature = JSON.stringify([mode, visible, selected, state.game_running, ids.map(id => state.catalog?.previews?.[id])]);
  if (signature !== pickerSignature) {
    pickerSignature = signature;
    const focusedIndex = Array.from(grid.children).indexOf(document.activeElement);
    grid.replaceChildren(...visible.map(item => itemCard(item, selected === item.id)));
    if (focusedIndex >= 0) grid.children[focusedIndex]?.focus({ preventScroll: true });
  }
  const category = mode === "active" ? null : currentCategory;
  const request = JSON.stringify([category, ids]);
  if ((requestedPreviews !== request || ids.some(id => !(state.preview_ids || []).includes(id))) && category !== undefined) {
    requestedPreviews = request;
    send({ type: "previews", category, ids });
  }
}

function renderActive() {
  const items = state.catalog?.items || [];
  field("picker-banner").textContent = "Active swaps";
  field("restore-all").disabled = !items.length || state.game_running;
  renderPicker(items, "", "No item swaps are active.");
}

function renderCurrent() {
  if (mode === "active") renderActive();
  else renderEditor();
}

new ResizeObserver(() => { if (state) renderCurrent(); }).observe(grid);

field("category").addEventListener("change", () => send({ type: "category", index: Number(field("category").value) }));
field("reload").addEventListener("click", () => send({ type: "reload" }));
field("owned").addEventListener("change", () => window.ipc.postMessage(JSON.stringify({type: "header", action: {type: "owned_only", enabled: field("owned").checked}})));
["owned", "car", "match-car", "source-search", "target-search"].forEach(id =>
  field(id).addEventListener("input", () => { page = 0; renderEditor(); }));
field("choose-replacement").addEventListener("click", () => { choosingReplacement = true; page = 0; renderEditor(); });
field("back").addEventListener("click", () => { choosingReplacement = false; page = 0; renderEditor(); });
field("prev-page").addEventListener("click", () => { page--; renderCurrent(); });
field("next-page").addEventListener("click", () => { page++; renderCurrent(); });
field("paint").addEventListener("change", () => { field("custom-fields").hidden = field("paint").value !== "1"; });
field("apply").addEventListener("click", () => {
  if (!choosingReplacement || field("apply").disabled) return;
  field("apply").disabled = true;
  const colour = field("custom").value.slice(1).match(/../g).map(part => parseInt(part, 16));
  send({ type: "apply", swap: {
    source: sourceId, target: targetId, owned_only: field("owned").checked,
    match_car: field("match-car").checked, paint: Number(field("paint").value),
    custom_colour: colour, speed: field("speed").value === "" ? 1 : Number(field("speed").value),
  } });
});
field("restore-all").addEventListener("click", () => send({ type: "restore", key: null }));

window.showSwapper = (snapshot) => {
  state = snapshot;
  const nextMode = state.mode === "active" ? "active" : "swapper";
  document.querySelector("main").classList.toggle("swapper-category-layout", nextMode !== "active");
  document.querySelector("main").classList.toggle("swapper-active-layout", nextMode === "active");
  if (mode !== nextMode) {
    mode = nextMode;
    page = 0;
    requestedPreviews = "";
    choosingReplacement = false;
    sourceId = targetId = "";
  }
  const status = field("status");
  status.textContent = state.status?.[1] || "";
  status.hidden = !status.textContent;
  status.classList.toggle("text-warning", !!state.status?.[0]);
  field("inventory").hidden = mode === "active";
  field("restore-all").hidden = mode !== "active";
  if (mode === "active") {
    for (const id of ["catalog-status", "reload", "category-title", "car-fields", "source-search", "target-search", "back", "paint-fields", "speed-fields", "choose-replacement", "apply"]) {
      field(id).hidden = true;
    }
    field("editor").hidden = false;
    renderActive();
    return;
  }
  field("category-title").hidden = true;
  const category = state.categories.find(row => row[2])?.[0];
  if (currentCategory !== category) {
    currentCategory = category;
    sourceId = targetId = "";
    choosingReplacement = false;
    page = 0;
    field("source-search").value = field("target-search").value = "";
    field("paint").value = "0";
  }
  options(field("category"), state.categories.map(([id, label]) => [id, label]), category);
  field("category-title").textContent = state.category_label;
  field("owned").checked = !!state.ownedOnly;
  const navigation = field("category-navigation");
  navigation.replaceChildren(...state.categories.map(([id, label, active]) => {
    const button = document.createElement("button");
    button.type = "button"; button.className = "btn btn-sm btn-" + (active ? "primary" : "outline-secondary");
    button.textContent = label;
    button.addEventListener("click", () => { field("category").value = id; field("category").dispatchEvent(new Event("change")); });
    return button;
  }));

  field("catalog-status").textContent = state.inventory.error || (state.inventory.loading ? "Reading save inventory…" : "");
  field("catalog-status").hidden = !field("catalog-status").textContent;
  field("editor").hidden = !state.catalog?.items;
  field("reload").hidden = state.loading || !(state.catalog_error || state.catalog?.failed);
  field("catalog-status").textContent = state.catalog_error || state.catalog?.status || (state.loading ? "Loading item catalogs…" : "");
  field("catalog-status").hidden = !field("catalog-status").textContent;
  options(field("paint"), (state.paint || []).map((label, index) => [index, label]), field("paint").value || "0");
  field("custom-fields").hidden = field("paint").value !== "1";
  renderEditor();
};

window.ipc.postMessage(JSON.stringify({ type: "swapper_ready" }));
