const field = (id) => document.getElementById(id);
const send = (action) => window.ipc.postMessage(JSON.stringify({ type: "replay_upload", action }));
const settings = ["save_locally", "upload_enabled", "provider", "naming_template", "visibility", "group_id", "debug_logging"];
const protectedFields = ["provider", "api_key", "clear_key", "visibility", "group_id"];
let historySignature = "";

settings.forEach((key) => field(key).addEventListener("change", (event) => {
  const input = event.target;
  send({ type: key, value: input.type === "checkbox" ? input.checked : input.value });
}));
field("enabled").addEventListener("change", (event) => {
  send({ type: "enable", value: event.target.checked });
  event.target.disabled = true;
});
field("api_key").addEventListener("change", (event) => {
  if (event.target.value) {
    send({ type: "api_key", value: event.target.value });
    event.target.value = "";
  }
});
field("clear_key").addEventListener("click", () => {
  field("api_key").value = "";
  send({ type: "api_key", value: "" });
});
field("check").addEventListener("click", () => send({ type: "check" }));

function textElement(tag, classes, value) {
  const element = document.createElement(tag);
  element.className = classes;
  element.textContent = value;
  return element;
}

function renderHistory(entries) {
  const signature = JSON.stringify(entries);
  if (signature === historySignature) return;
  historySignature = signature;
  field("empty").hidden = entries.length > 0;
  field("history").replaceChildren(...entries.map((entry) => {
    const item = textElement("li", "list-group-item px-0 bg-transparent", "");
    const heading = textElement("div", "d-flex flex-wrap align-items-center gap-2", "");
    heading.append(textElement("span", "text-break font-monospace small", entry.guid),
      textElement("span", "badge text-bg-secondary", entry.outcome));
    if (entry.has_link) {
      const link = textElement("button", "btn btn-link p-0", "Open on ballchasing.com");
      link.type = "button";
      link.addEventListener("click", () => send({ type: "open", guid: entry.guid }));
      heading.append(link);
    }
    item.append(heading);
    if (entry.title) item.append(textElement("div", "text-body-secondary", entry.title));
    if (entry.warning) item.append(textElement("div", "text-warning small", entry.warning));
    return item;
  }));
}

window.showReplayUpload = (state) => {
  settings.forEach((key) => {
    const input = field(key);
    if (input.type === "checkbox") input.checked = state[key];
    else if (document.activeElement !== input) input.value = state[key];
  });
  field("enabled").checked = state.enabled;
  field("enabled").disabled = !state.can_toggle;
  field("save_locally").disabled = !state.options_available;
  field("upload_enabled").disabled = !state.options_available;
  protectedFields.forEach((key) => { field(key).disabled = !state.upload_enabled; });
  field("clear_key").disabled ||= !state.has_key;
  field("api_key").placeholder = state.has_key ? "API key saved" : "";
  field("missing-key").hidden = !state.upload_enabled || state.has_key;
  field("check").disabled = !state.can_check;
  field("patch-status").hidden = !state.patch_status;
  field("patch-status").textContent = state.patch_status;
  field("status").textContent = state.pending ? state.status + " · Checking or delivering replay" : state.status;
  renderHistory(state.history);
};

window.ipc.postMessage(JSON.stringify({ type: "replay_upload_ready" }));
