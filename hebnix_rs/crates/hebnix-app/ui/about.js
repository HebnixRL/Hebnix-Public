const version = document.getElementById("version");
const maintenance = document.getElementById("maintenance");
const send = (action) => window.ipc.postMessage(JSON.stringify(action));

document.querySelectorAll("[data-link]").forEach((link) => {
  link.addEventListener("click", (event) => {
    event.preventDefault();
    send({ type: "about_link", link: link.dataset.link });
  });
});

maintenance.addEventListener("click", () => send({ type: "maintenance" }));

window.showAbout = (about) => {
  version.textContent = about.version
    .replace(/^(Version\s+\S+)\s+/, "$1\n\n")
    .replace(/\s+(Press\s)/, "\n\n$1");
  maintenance.hidden = true;
  const card = document.querySelector(".about-card");
  card.classList.remove("enter");
  requestAnimationFrame(() => card.classList.add("enter"));
};

send({ type: "about_ready" });
