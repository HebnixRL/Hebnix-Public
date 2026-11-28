let tabTransition;
window.animateHebnixTab = () => {
  tabTransition?.cancel();
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  tabTransition = document.body.animate(
    [{ opacity: .6 }, { opacity: 1 }],
    { duration: 150, easing: "ease-out" },
  );
};

window.applyHebnixTheme = theme => {
  const root = document.documentElement;
  root.dataset.bsTheme = theme?.mode || "dark";
  root.toggleAttribute("data-hebnix-theme", Boolean(theme));
  if (!theme) return;
  for (const key of ["background", "surface", "input", "text", "muted", "border", "accent"]) {
    root.style.setProperty("--theme-" + key, theme[key]);
  }
  root.style.setProperty("--theme-radius", theme.radius + "px");
  root.style.setProperty("--theme-padding", theme.padding === null ? ".85rem" : theme.padding + "px");
  const [surface, selected, text] = theme.navigation || [theme.background, theme.accent, theme.text];
  root.style.setProperty("--theme-nav-surface", surface);
  root.style.setProperty("--theme-nav-selected", selected);
  root.style.setProperty("--theme-nav-text", text);
};

window.navigateController = key => {
  if (key === "back") {
    const modal = document.querySelector("dialog[open]");
    if (modal) {
      modal.dispatchEvent(new Event("cancel", { cancelable: true }));
    } else {
      window.ipc.postMessage(JSON.stringify({ type: "panel_back" }));
    }
    return;
  }
  const root = document.querySelector("dialog[open]") || document;
  const actions = Array.from(root.querySelectorAll("button, a[href], select, input:not([type=hidden]), textarea, summary, [tabindex='0']"))
    .filter(node => !node.disabled && node.getClientRects().length && !node.closest("[hidden], [inert]"));
  if (!actions.length) return;
  const current = document.activeElement;
  const index = actions.indexOf(current);
  if (key === "accept") {
    if (index === -1) actions[0].focus();
    else if (current instanceof HTMLSelectElement && typeof current.showPicker === "function") {
      try { current.showPicker(); } catch { current.focus(); }
    } else current.click();
    return;
  }
  if (key === "previous" || key === "next") {
    const control = actions.find(node => node.tagName === "BUTTON" &&
      (node.textContent.trim().toLowerCase() === key || node.getAttribute("aria-label")?.toLowerCase().startsWith(key)));
    control?.click();
    return;
  }
  const step = key === "up" || key === "left" ? -1 : 1;
  if (index >= 0 && (key === "left" || key === "right")) {
    if (current instanceof HTMLSelectElement) {
      const start = current.selectedIndex;
      for (let next = start + step; next >= 0 && next < current.options.length; next += step) {
        if (current.options[next].disabled) continue;
        current.selectedIndex = next;
        current.dispatchEvent(new Event("change", { bubbles: true }));
        return;
      }
    }
    if (current instanceof HTMLInputElement && current.type === "range") {
      current.stepUp(step);
      for (const event of ["input", "change"]) current.dispatchEvent(new Event(event, { bubbles: true }));
      return;
    }
  }
  actions[index < 0 ? 0 : (index + step + actions.length) % actions.length].focus();
};
// Native colour inputs retain the platform picker. Parent movement is forwarded by Wry.

