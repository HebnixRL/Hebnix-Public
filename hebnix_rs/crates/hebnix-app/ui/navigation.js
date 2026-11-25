const navigation = document.getElementById("navigation");
const tabs = /*TABS*/[];
const send = action => window.ipc.postMessage(JSON.stringify(action));
let expanded = null;
let selectedTab = null;
let selectedChild = null;
let selectedLeaf = null;
const expandedChildren = new Map();

function buttonFor(label, action) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "btn btn-sm";
  button.textContent = label;
  button.addEventListener("click", action);
  return button;
}

function markCurrent(button, current) {
  button.classList.toggle("active", current);
  if (current) button.setAttribute("aria-current", "page");
  else button.removeAttribute("aria-current");
}

const groups = tabs.map(({ label, children, descendants }, tab) => {
  const group = document.createElement("div");
  const panel = document.createElement("div");
  panel.id = "subtabs-" + tab;
  panel.className = "subnav";
  panel.hidden = true;
  const button = buttonFor(label, () => {
    if (children.length && expanded === tab) {
      expanded = null;
      updateSelection();
    } else {
      expanded = children.length ? tab : null;
      send({ type: "select", tab, child: null });
      updateSelection();
    }
  });
  button.classList.add("main-tab");
  if (children.length) button.setAttribute("aria-controls", panel.id);
  group.append(button);
  const leaves = [];
  const childButtons = children.map((name, child) => {
    const options = descendants?.[child] || [];
    const list = document.createElement("div");
    list.id = "leaf-tabs-" + tab + "-" + child;
    list.className = "subnav";
    list.hidden = true;
    leaves.push(list);
    const childButton = buttonFor(name, () => {
      if (options.length && selectedTab === tab && selectedChild === child &&
          expandedChildren.get(tab) === child) {
        expandedChildren.delete(tab);
        updateSelection();
      } else {
        expanded = tab;
        expandedChildren.set(tab, child);
        send({ type: "select", tab, child });
        updateSelection();
      }
    });
    if (options.length) {
      childButton.classList.add("main-tab");
      childButton.setAttribute("aria-controls", list.id);
    }
    for (const [leaf, text] of options) {
      const leafButton = buttonFor(text, () => {
        selectedLeaf = leaf;
        send({ type: "nested", tab, child, leaf });
        updateSelection();
      });
      leafButton.dataset.leaf = leaf;
      list.append(leafButton);
    }
    panel.append(childButton, list);
    return childButton;
  });
  if (children.length) group.append(panel);
  navigation.append(group);
  return { button, panel, children: childButtons, leaves };
});

function updateSelection() {
  groups.forEach(({ button, panel, children, leaves }, tab) => {
    markCurrent(button, tab === selectedTab && !children.length);
    button.classList.toggle("active", tab === selectedTab);
    panel.hidden = tab !== expanded;
    if (children.length) button.setAttribute("aria-expanded", String(tab === expanded));
    children.forEach((button, child) => {
      const selected = tab === selectedTab && child === selectedChild;
      markCurrent(button, selected);
      const list = leaves[child];
      list.hidden = !list.children.length || expandedChildren.get(tab) !== child;
      if (list.children.length) button.setAttribute("aria-expanded", String(!list.hidden));
      list.querySelectorAll("button").forEach(leaf => {
        markCurrent(leaf, selected && Number(leaf.dataset.leaf) === selectedLeaf);
      });
    });
  });
}

window.selectHebnixTab = (tab, child, leaf = null) => {
  if (selectedTab !== tab || selectedChild !== child) {
    expanded = tab;
    if (child !== null) expandedChildren.set(tab, child);
  }
  selectedTab = tab;
  selectedChild = child;
  selectedLeaf = leaf;
  updateSelection();
};
send({ type: "ready" });
