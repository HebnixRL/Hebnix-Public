const field = (id) => document.getElementById(id);
const send = (action) => window.ipc.postMessage(JSON.stringify({ type: "tracker", action }));
const chart = field("chart");
let history = [];
let modeSignature = "";

function drawChart() {
  if (!history.length || field("plot").hidden) return;
  const width = chart.clientWidth;
  const height = chart.clientHeight;
  if (width < 100 || height < 100) return;
  const scale = window.devicePixelRatio || 1;
  chart.width = Math.round(width * scale);
  chart.height = Math.round(height * scale);
  const ctx = chart.getContext("2d");
  if (!ctx) return;
  ctx.scale(scale, scale);
  const theme = getComputedStyle(chart);
  const colour = (name, fallback) => theme.getPropertyValue(name).trim() || fallback;
  const accent = colour("--theme-accent", "#72c8ff");
  const muted = colour("--theme-muted", "#a9b6d0");
  const border = colour("--theme-border", "#364158");
  const left = 58, right = width - 24, top = 16, bottom = height - 28;
  const first = history[0].day, span = history.at(-1).day - first;
  const min = Math.min(...history.map(point => point.mmr));
  const max = Math.max(...history.map(point => point.mmr));
  const padding = Math.max(3, Math.ceil((max - min) * .12));
  const low = min - padding, high = max + padding;
  const points = history.map(point => [
    left + (right - left) * (span ? (point.day - first) / span : .5),
    top + (bottom - top) * ((high - point.mmr) / (high - low)),
  ]);
  ctx.font = "12px system-ui";
  ctx.textAlign = "right";
  ctx.textBaseline = "middle";
  for (let i = 0; i <= 3; i++) {
    const y = top + (bottom - top) * i / 3;
    ctx.strokeStyle = border;
    ctx.beginPath(); ctx.moveTo(left, y); ctx.lineTo(right, y); ctx.stroke();
    ctx.fillStyle = muted;
    ctx.fillText(Math.round(high - (high - low) * i / 3).toLocaleString(), left - 8, y);
  }
  if (points.length > 1) {
    ctx.beginPath();
    points.forEach(([x, y], i) => i ? ctx.lineTo(x, y) : ctx.moveTo(x, y));
    ctx.lineTo(points.at(-1)[0], bottom);
    ctx.lineTo(points[0][0], bottom);
    ctx.closePath();
    const gradient = ctx.createLinearGradient(0, top, 0, bottom);
    gradient.addColorStop(0, accent);
    gradient.addColorStop(1, "transparent");
    ctx.save(); ctx.globalAlpha = .22; ctx.fillStyle = gradient; ctx.fill(); ctx.restore();
    ctx.beginPath();
    points.forEach(([x, y], i) => i ? ctx.lineTo(x, y) : ctx.moveTo(x, y));
    ctx.strokeStyle = accent; ctx.lineWidth = 2; ctx.stroke();
  }
  ctx.textAlign = "center";
  let labelX = -Infinity;
  points.forEach(([x, y], i) => {
    if (history[i].label && x - labelX >= 42 && (i === points.length - 1 || points.at(-1)[0] - x >= 42)) {
      ctx.fillStyle = muted;
      ctx.fillText(history[i].label, x, bottom + 15);
      labelX = x;
    }
  });

  const dotLayer = field("points");
  const focusedIndex = Array.from(dotLayer.children).indexOf(document.activeElement);
  dotLayer.replaceChildren(...points.map(([x, y], index) => {
    const point = history[index];
    const label = [point.label, point.mmr.toLocaleString() + " MMR"].filter(Boolean).join(" · ");
    const dot = document.createElement("button");
    dot.type = "button";
    dot.className = "tracker-point";
    dot.setAttribute("aria-label", label);
    dot.style.left = x + "px";
    dot.style.top = y + "px";
    const tip = document.createElement("span");
    tip.className = "tracker-tooltip";
    tip.setAttribute("aria-hidden", "true");
    tip.textContent = label;
    tip.style.left = (Math.min(width - 86, Math.max(86, x)) - x + 10) + "px";
    dot.append(tip);
    return dot;
  }));
  if (focusedIndex >= 0) dotLayer.children[focusedIndex]?.focus({ preventScroll: true });
}
new ResizeObserver(drawChart).observe(chart);
new MutationObserver(drawChart).observe(document.documentElement, {
  attributes: true, attributeFilter: ["style", "data-hebnix-theme", "data-bs-theme"],
});
window.showTracker = (data) => {
  field("subtitle").textContent = data.subtitle;
  const status = [data.waiting, data.error, data.loading, data.empty].filter(Boolean).join(" · ");
  field("status").textContent = status;
  field("status").hidden = !status;
  const player = data.player;
  field("player").hidden = !player;
  if (player) {
    field("name").textContent = player.name;
    field("avatar").src = player.avatar || "/tracker-logo.png";
  }
  const selection = data.selection;
  field("summary").hidden = !player && !selection;
  for (const id of ["playlist", "current", "rank-card", "change-card", "detail"]) {
    field(id).hidden = !selection;
  }
  const signature = JSON.stringify([data.modes, selection?.id]);
  if (signature !== modeSignature) {
    modeSignature = signature;
    const modes = field("modes");
    const scrollTop = modes.scrollTop;
    const focusedIndex = Array.from(modes.children).indexOf(document.activeElement);
    modes.replaceChildren(...data.modes.map((mode) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "btn tracker-tile";
      button.classList.toggle("active", selection?.id === mode.id);
      button.setAttribute("aria-pressed", String(selection?.id === mode.id));
      if (mode.badge) {
        const badge = document.createElement("img");
        badge.src = mode.badge; badge.alt = ""; badge.className = "tracker-badge";
        button.append(badge);
      } else {
        const icon = document.createElement("span");
        icon.className = "tracker-badge tracker-casual-icon";
        icon.setAttribute("aria-hidden", "true");
        icon.textContent = "🎮";
        button.append(icon);
      }
      const name = document.createElement("span");
      const rating = document.createElement("strong");
      name.className = "tracker-mode-name";
      name.textContent = mode.name;
      button.title = mode.name + " · " + mode.rating;
      rating.textContent = mode.rating;
      button.append(name, rating);
      button.addEventListener("click", () => send({ type: "select", id: mode.id }));
      return button;
    }));
    modes.scrollTop = scrollTop;
    if (focusedIndex < 0 && !scrollTop) modes.querySelector(".active")?.scrollIntoView({ block: "nearest" });
    if (focusedIndex >= 0) modes.children[focusedIndex]?.focus({ preventScroll: true });
  }
  if (!selection) {
    history = [];
    field("points").replaceChildren();
    return;
  }
  const rank = [selection.rank, selection.division].filter(Boolean).join(" · ");
  const change = selection.change || "—";
  field("playlist-name").textContent = selection.name;
  field("rating").textContent = field("detail-rating").textContent = selection.rating;
  field("rank").textContent = selection.rank || "—";
  field("rank-division").textContent = selection.division || "";
  field("rank-division").hidden = !selection.division;
  field("rank-card").title = rank;
  field("change").textContent = field("detail-change").textContent = change;
  field("change-label").textContent = field("detail-change-label").textContent = selection.change_label;
  field("mode-title").textContent = selection.name;
  field("mode-rank").textContent = rank;
  field("mode-rank").hidden = !rank;
  for (const id of ["badge", "playlist-badge", "rank-badge"]) {
    const badge = field(id);
    badge.hidden = !selection.badge;
    if (selection.badge) badge.src = selection.badge;
  }
  field("history-title").textContent = data.history_title;
  history = selection.history;
  field("plot").hidden = !history.length;
  field("history-empty").hidden = !!history.length;
  field("history-empty").textContent = data.empty_history;
  if (!history.length) field("points").replaceChildren();
  drawChart();
};

window.ipc.postMessage(JSON.stringify({ type: "tracker_ready" }));
