const dialog = document.getElementById("dialog");
const title = document.getElementById("title");
const body = document.getElementById("body");
const actions = document.getElementById("actions");
let activeId = null;
let submitting = false;
let current;

function submit(key) {
  if (!current || submitting) return;
  const message = JSON.stringify({ type: "dialog", id: current.id, key });
  submitting = true;
  if (!current.buttons.find(button => button.key === key)?.close) {
    window.ipc.postMessage(message);
    submitting = false;
    return;
  }
  dialog.classList.add("is-closing");
  window.setTimeout(() => {
    if (dialog.open) dialog.close();
    dialog.classList.remove("is-closing");
    activeId = null;
    window.ipc.postMessage(message);
  }, window.matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 150);
}

window.showDialog = data => {
  const changed = activeId !== data.id;
  activeId = data.id;
  current = data;
  submitting = false;
  title.textContent = data.title;
  body.replaceChildren();
  for (const line of data.lines) {
    const row = document.createElement("p");
    row.className = "mb-2";
    row.textContent = line;
    body.append(row);
  }
  if (data.error) {
    const error = document.createElement("p");
    error.className = "text-danger mb-2";
    error.textContent = data.error;
    body.append(error);
  }
  if (data.progress) {
    const progress = document.createElement("progress");
    progress.className = "w-100";
    progress.setAttribute("aria-label", data.progress.label);
    if (data.progress.total) {
      progress.max = data.progress.total;
      progress.value = data.progress.value;
    }
    body.append(progress);
  }
  if (data.toggle) {
    const label = document.createElement("label");
    label.className = "d-flex align-items-center gap-2";
    const input = document.createElement("input");
    input.className = "form-check-input mt-0";
    input.type = "checkbox";
    input.checked = data.toggle.checked;
    input.addEventListener("change", () => {
      window.ipc.postMessage(JSON.stringify({ type: "dialog", id: data.id, key: input.checked ? "on" : "off" }));
    });
    label.append(input, data.toggle.label);
    body.append(label);
  }
  actions.replaceChildren();
  for (const action of data.buttons) {
    const node = document.createElement("button");
    node.type = "button";
    node.className = "btn btn-" + (action.key === "primary" ? "primary" : "outline-secondary");
    node.textContent = action.label;
    node.addEventListener("click", () => submit(action.key));
    actions.append(node);
  }
  if (changed && dialog.open) dialog.close();
  if (!dialog.open) dialog.showModal();
};

window.clearDialog = () => {
  if (dialog.open) dialog.close();
  dialog.classList.remove("is-closing");
  activeId = null;
  current = null;
  submitting = false;
};

dialog.addEventListener("cancel", event => {
  event.preventDefault();
  if (current?.buttons.some(button => button.key === "dismiss")) submit("dismiss");
});

window.ipc.postMessage(JSON.stringify({ type: "dialog_ready" }));
