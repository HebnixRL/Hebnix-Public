// Shared patch activity: use real fractions when provided, never invent one.
window.patchProgress = (host, busy, progress = null, label = "Applying patch", key = "operation") => {
  let bar = [...host.children].find(node => node.dataset.patchProgress === key);
  if (!busy) { bar?.remove(); return; }
  if (!bar) {
    bar = document.createElement("div");
    bar.className = "progress patch-progress";
    bar.dataset.patchProgress = key;
    bar.setAttribute("role", "progressbar");
    bar.append(document.createElement("div"));
    bar.firstElementChild.className = "progress-bar";
    host.append(bar);
  }
  bar.setAttribute("aria-label", label || "Applying patch");
  bar.title = label || "Applying patch";
  let fraction = typeof progress === "number" ? progress : null;
  if (progress && typeof progress === "object" && Number.isFinite(progress.done) && progress.total > 0)
    fraction = progress.done / progress.total;
  const known = Number.isFinite(fraction);
  bar.classList.toggle("is-indeterminate", !known);
  if (known) {
    const percent = Math.round(Math.max(0, Math.min(1, fraction)) * 100);
    bar.setAttribute("aria-valuemin", "0"); bar.setAttribute("aria-valuemax", "100");
    bar.setAttribute("aria-valuenow", String(percent));
    bar.firstElementChild.style.width = percent + "%";
    bar.firstElementChild.style.animationDelay = "";
  } else {
    bar.removeAttribute("aria-valuemin"); bar.removeAttribute("aria-valuemax"); bar.removeAttribute("aria-valuenow");
    bar.firstElementChild.style.width = "36%";
    // Manager cards are redrawn as status arrives; keep animation phase stable.
    bar.firstElementChild.style.animationDelay = -(performance.now() % 1500) + "ms";
  }
};
