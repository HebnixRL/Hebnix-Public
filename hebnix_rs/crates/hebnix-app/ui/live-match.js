const field = (id) => document.getElementById(id);
let teamsSignature = "";
let eventsSignature = "";

function node(tag, className, text = "") {
  const element = document.createElement(tag);
  element.className = className;
  element.textContent = text;
  return element;
}

function teamCard(team, columns) {
  const card = node("section", "tracker-detail live-team");
  const heading = node("header", "d-flex align-items-center justify-content-between gap-3 p-3");
  heading.style.borderInlineStartColor = team.colour;
  heading.append(node("h2", "h5 mb-0 text-break", team.name), node("strong", "tracker-number", team.score));
  const scroll = node("div", "table-responsive");
  const table = node("table", "table table-sm mb-0 live-table");
  const head = node("thead", "");
  const row = node("tr", "");
  columns.forEach((label) => {
    const th = node("th", "", label);
    th.scope = "col";
    row.append(th);
  });
  head.append(row);
  const body = node("tbody", "");
  team.players.forEach(([name, ...stats]) => {
    const line = node("tr", "");
    const player = node("th", "text-break", name);
    player.scope = "row";
    line.append(player, ...stats.map(value => node("td", "", value)));
    body.append(line);
  });
  table.append(head, body);
  scroll.append(table);
  card.append(heading, scroll);
  return card;
}

const runDashboardAction = action => window.ipc.postMessage(JSON.stringify({type: "header", action: {type: "run", action}}));
let dashboardRunning = false;
field("dashboard-start").addEventListener("click", () => runDashboardAction(dashboardRunning ? "close_rocket_league" : "start_rocket_league"));
window.showLiveMatch = (state) => {
  document.querySelector(".live-summary").hidden = !state.dashboard;
  document.querySelector(".live-columns").hidden = !!state.dashboard;
  field("dashboard-actions").hidden = !state.dashboard;
  dashboardRunning = state.running;
  const launch = field("dashboard-start");
  launch.replaceChildren(document.createTextNode(state.running ? "Stop Rocket League" : "Start Rocket League"));
  const symbol = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  symbol.setAttribute("viewBox", "0 0 16 16");
  symbol.setAttribute("aria-hidden", "true");
  symbol.classList.add("dashboard-action-icon");
  const shape = document.createElementNS(symbol.namespaceURI, "path");
  shape.setAttribute("d", state.running ? "M3 3h10v10H3Z" : "M4 2l10 6-10 6Z");
  symbol.append(shape);
  launch.append(symbol);
  launch.className = "btn " + (state.running ? "btn-outline-danger" : "btn-primary");
  field("running").textContent = state.running_label;
  field("running").classList.toggle("text-success", state.running);
  field("connection").textContent = state.connection_label;
  field("connection").classList.toggle("text-success", state.connected);
  const cosmetics = field("live-cosmetics");
  cosmetics.textContent = state.live_cosmetics.label;
  cosmetics.className = "text-" + state.live_cosmetics.colour;
  cosmetics.title = state.live_cosmetics.detail;
  const installing = !!state.live_cosmetics.installing;
  cosmetics.setAttribute("aria-busy", String(installing));
  const progress = field("live-cosmetics-progress");
  progress.hidden = !state.dashboard || !installing;
  progress.title = state.live_cosmetics.detail;
  let error = field("live-cosmetics-error");
  if (!error) {
    error = node("small", "text-danger text-break");
    error.id = "live-cosmetics-error";
    cosmetics.parentElement.append(error);
  }
  error.hidden = state.live_cosmetics.label !== "Error";
  error.textContent = error.hidden ? "" : state.live_cosmetics.detail.replace(/^Error:\s*/i, "");
  const game = state.game;
  field("match-heading").hidden = !game;
  field("match-clock").hidden = !game;
  field("clock").textContent = game?.clock || "";
  field("arena").textContent = game?.arena || "";
  const details = [game?.ended, game?.replay, game?.stale].filter(Boolean).join(" · ");
  field("match-status").hidden = !details;
  field("details").textContent = details;
  field("waiting").hidden = !!game;
  field("waiting").textContent = state.waiting;
  const spectators = game?.spectators || [];
  field("spectators").hidden = !spectators.length;
  field("spectators").textContent = spectators.length ? state.spectator_label + " · " + spectators.join(", ") : "";
  const teams = game?.teams || [];
  const nextTeams = JSON.stringify([teams, state.columns]);
  if (nextTeams !== teamsSignature) {
    teamsSignature = nextTeams;
    const scrollPositions = Array.from(field("teams").children, card => card.lastElementChild.scrollLeft);
    field("teams").replaceChildren(...teams.map(team => teamCard(team, state.columns)));
    Array.from(field("teams").children).forEach((card, index) => { card.lastElementChild.scrollLeft = scrollPositions[index] || 0; });
  }
  field("events-title").textContent = state.events_title;
  field("empty-events").hidden = !!state.events.length;
  field("empty-events").textContent = state.empty_events;
  const nextEvents = JSON.stringify(state.events);
  if (nextEvents !== eventsSignature) {
    eventsSignature = nextEvents;
    field("events").replaceChildren(...state.events.map((event) => {
      const item = node("li", "live-event p-3");
      item.append(node("strong", "small live-event-clock", event.clock), node("div", "text-break", event.message));
      return item;
    }));
  }
};

window.ipc.postMessage(JSON.stringify({ type: "live_match_ready" }));
