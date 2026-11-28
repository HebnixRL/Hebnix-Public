const field = (id) => document.getElementById(id);
const send = (action) => window.ipc.postMessage(JSON.stringify({ type: "browse_maps", action }));
let state = null;
let mapSignature = "";
let deleteId = null;
let filterTimer;
const grid = field("maps");
let columns = 1;
let sentColumns = 0;

function fitGrid() {
  if (!grid.clientWidth) return;
  columns = Math.max(1, Math.min(12, Math.floor((grid.clientWidth + 16) / 286)));
  if (columns === sentColumns) return;
  sentColumns = columns;
  grid.style.setProperty("--map-columns", columns);
  send({ type: "columns", count: columns });
  if (state) renderMaps();
}

new ResizeObserver(() => { if (state) fitGrid(); }).observe(grid);

function node(tag, classes, text = "") {
  const element = document.createElement(tag);
  element.className = classes;
  element.textContent = text;
  return element;
}

function button(label, classes, action, disabled = false) {
  const control = node("button", classes, label);
  control.type = "button";
  control.title = label;
  control.disabled = disabled;
  control.addEventListener("click", action);
  return control;
}

function filter() {
  clearTimeout(filterTimer);
  send({ type: "filter", search: field("search").value, downloaded: field("downloaded").checked });
}
field("search").addEventListener("input", () => {
  field("clear-search").hidden = !field("search").value;
  clearTimeout(filterTimer);
  filterTimer = setTimeout(filter, 450);
});
field("downloaded").addEventListener("change", filter);
field("clear-search").addEventListener("click", () => {
  field("search").value = "";
  field("clear-search").hidden = true;
  filter();
  field("search").focus();
});
field("steam-search").addEventListener("click", () => {
  clearTimeout(filterTimer);
  send({ type: "search", search: field("search").value });
});
field("search").addEventListener("keydown", (event) => {
  if (event.key === "Enter") field("steam-search").click();
});
field("prev").addEventListener("click", () => send({ type: "page", index: state.page - 1 }));
field("next").addEventListener("click", () => send({ type: "page", index: state.page + 1 }));
field("enable-menu").addEventListener("click", () => send({ type: "enable_menu" }));

function mapCard(map) {
  const card = node("article", "tracker-detail maps-card");
  const image = node("div", "maps-banner");
  if (map.banner) {
    const picture = node("img", "");
    picture.src = map.banner;
    picture.alt = "";
    image.append(picture);
  } else image.textContent = map.banner_status;
  const contents = node("div", "maps-card-body p-3");
  const heading = node("h2", "h6 mb-1 text-break maps-card-title", map.name);
  heading.title = map.description ? map.name + "\n" + map.description : map.name;
  const badge = node("span", "badge text-truncate maps-status " + (map.busy ? "text-bg-warning" : map.loaded ? "text-bg-success" : "text-bg-secondary"), map.status);
  badge.title = map.status;
  image.append(badge);
  if (map.can_forget) {
    const forget = node("button", "maps-forget", "✕");
    forget.type = "button";
    forget.title = "Remove from cached search";
    forget.setAttribute("aria-label", "Remove " + map.name + " from cached search");
    forget.addEventListener("click", () => send({ type: "forget", id: map.id }));
    card.append(forget);
  }
  contents.append(heading, node("p", "small text-body-secondary mb-2 text-truncate", map.author));
  const actions = node("div", "maps-card-actions mt-auto");
  if (deleteId === map.id) {
    const prompt = node("p", "small mb-1 w-100", map.delete_prompt);
    contents.append(prompt);
    actions.append(button("Remove", "btn btn-sm btn-danger", () => {
      deleteId = null;
      send({ type: "delete", id: map.id });
      renderMaps();
    }), button("Cancel", "btn btn-sm btn-outline-secondary", () => { deleteId = null; renderMaps(); }));
  } else {
    const primary = button(map.busy ? map.primary : map.loaded ? "Unload" : map.cached ? "Load" : "Download",
      map.loaded ? "btn btn-sm btn-outline-warning" : "btn btn-sm btn-primary",
      () => send({ type: map.loaded ? "unload" : "load", id: map.id }), map.busy);
    primary.title = map.primary;
    actions.append(primary);
    if (map.can_delete) actions.append(button("Remove", "btn btn-sm btn-outline-danger",
      () => { deleteId = map.id; renderMaps(); }));
  }
  contents.append(actions);
  window.patchProgress(contents, !!map.busy, map.progress, map.primary || "Updating map");
  card.append(image, contents);
  return card;
}

function renderMaps() {
  if (!state.maps.some(map => map.id === deleteId && map.can_delete)) deleteId = null;
  grid.replaceChildren(...state.maps.slice(0, columns * 2).map(mapCard));
}

window.showBrowseMaps = (data) => {
  state = data;
  fitGrid();
  const search = field("search");
  if (document.activeElement !== search) search.value = data.search;
  field("clear-search").hidden = !search.value;
  field("banner").hidden = !data.banner;
  field("banner").textContent = data.banner;
  search.placeholder = data.search_label;
  search.setAttribute("aria-label", data.search_label);
  field("downloaded").checked = data.downloaded;
  field("downloaded-label").textContent = data.downloaded_label;
  const locked = data.menu_disabled || data.menu_busy;
  field("menu-warning").hidden = !locked;
  document.querySelector("main").inert = locked;
  field("menu-lock-title").textContent = data.menu_lock.title;
  field("menu-lock-body").textContent = data.menu_lock.body;
  field("enable-menu").textContent = data.menu_busy ? data.menu_lock.busy : data.menu_lock.enable;
  field("enable-menu").disabled = data.menu_busy;
  window.patchProgress(field("menu-card"), !!data.menu_busy, null, "Updating Workshop in-game menu");
  field("count").textContent = data.steam_total
    ? data.steam_total.toLocaleString() + " maps on the Steam Workshop" + (data.loading ? " (loading...)" : "")
    : data.count.toLocaleString() + (data.count === 1 ? " map" : " maps") + (data.loading ? " (loading...)" : "");
  field("page").textContent = (data.page + 1) + " / " + data.pages;
  field("prev").disabled = data.page === 0;
  field("next").disabled = data.page + 1 >= data.pages;
  field("map-pages").hidden = !data.count;
  field("empty").hidden = !!data.maps.length;
  field("empty").textContent = data.empty;
  const signature = JSON.stringify(data.maps);
  if (signature !== mapSignature) {
    mapSignature = signature;
    renderMaps();
  }
};

window.ipc.postMessage(JSON.stringify({ type: "browse_maps_ready" }));
