const field = (id) => document.getElementById(id);
const input = field("command");
const send = (action) => window.ipc.postMessage(JSON.stringify({ type: "console", action }));
let history = [];
let historyIndex = 0;
let suggestions = [];
let suggestionIndex = -1;
let output = null;
let initialized = false;
let lastDraft = null;

function changeDraft(draft) {
  lastDraft = draft;
  send({ type: "draft", value: draft });
}

function useSuggestion(index) {
  input.value = suggestions[index];
  suggestions = [];
  suggestionIndex = -1;
  showSuggestions();
  changeDraft(input.value);
  input.focus();
}

function showSuggestions() {
  const list = field("suggestions");
  list.hidden = suggestions.length === 0;
  list.replaceChildren(...suggestions.map((text, index) => {
    const item = document.createElement("li");
    item.className = "list-group-item p-0";
    const button = document.createElement("button");
    button.type = "button";
    button.className = "btn text-start w-100 font-monospace";
    if (index === suggestionIndex) button.classList.add("active");
    button.textContent = text;
    button.addEventListener("click", () => useSuggestion(index));
    item.append(button);
    return item;
  }));
}

input.addEventListener("input", () => {
  suggestionIndex = -1;
  changeDraft(input.value);
});
input.addEventListener("keydown", (event) => {
  const up = event.key === "ArrowUp";
  if (event.key === "Tab" && !event.shiftKey && suggestions.length) {
    event.preventDefault();
    useSuggestion(Math.max(0, suggestionIndex));
  } else if (up || event.key === "ArrowDown") {
    event.preventDefault();
    if (suggestions.length) {
      const next = suggestionIndex < 0 ? (up ? suggestions.length - 1 : 0) : suggestionIndex + (up ? -1 : 1);
      suggestionIndex = (next + suggestions.length) % suggestions.length;
      showSuggestions();
    } else {
      historyIndex = Math.min(history.length, Math.max(0, historyIndex + (up ? -1 : 1)));
      input.value = history[historyIndex] || "";
      lastDraft = input.value;
      send({ type: "history_draft", value: input.value });
    }
  }
});
field("command-form").addEventListener("submit", (event) => {
  event.preventDefault();
  if (suggestionIndex >= 0) {
    useSuggestion(suggestionIndex);
    return;
  }
  const value = input.value.trim();
  if (value) {
    input.value = "";
    lastDraft = "";
    input.focus();
    send({ type: "submit", value });
  }
});

window.showConsole = (state) => {
  input.placeholder = state.placeholder;
  if (!initialized) {
    input.value = state.draft;
    initialized = true;
  }
  if (history.length !== state.history.length) {
    history = state.history;
    historyIndex = history.length;
  }
  if (lastDraft === null || lastDraft === state.draft) {
    suggestions = state.suggestions;
    if (suggestionIndex >= suggestions.length) suggestionIndex = -1;
    showSuggestions();
  }
  const text = state.lines.join("\n");
  if (text !== output) {
    const log = field("lines");
    const atBottom = output === null || log.scrollHeight - log.scrollTop - log.clientHeight < 24;
    log.textContent = text;
    if (atBottom) log.scrollTop = log.scrollHeight;
    output = text;
  }
};

window.ipc.postMessage(JSON.stringify({ type: "console_ready" }));
