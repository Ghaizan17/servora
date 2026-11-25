/**
 * Theme-matched dropdown.
 *
 * The native `<select>` popup is drawn by the browser or the GTK theme, so its
 * colours and radii never match Servora's palette. This component replaces it
 * with a listbox built from the same tokens as the rest of the interface.
 *
 * Behaviour:
 * - click or Enter/Space/ArrowDown opens, Escape or Tab closes
 * - ArrowUp / ArrowDown / Home / End move the highlight, Enter selects
 * - clicking outside closes, and the trigger keeps a visible focus ring
 * - `role="listbox"` + `aria-activedescendant` keep screen readers working
 */
import { icons } from "../lib/icons.js";
import { escapeHtml } from "../lib/format.js";

/**
 * @param {object} config
 * @param {Array<{value: string, label: string}>} config.options
 * @param {string} [config.value]        initial value
 * @param {(value: string) => void} [config.onChange]
 * @param {string} [config.ariaLabel]
 * @param {"start"|"end"} [config.align] which edge the menu aligns to
 * @param {boolean} [config.block]       stretch to the container width
 */
export function select({
  options = [],
  value,
  onChange,
  ariaLabel = "",
  align = "start",
  block = false,
} = {}) {
  const root = document.createElement("div");
  root.className = `select${block ? " select-block" : ""}`;
  root.dataset.select = "";

  const trigger = document.createElement("button");
  trigger.type = "button";
  trigger.className = "select-trigger";
  trigger.setAttribute("aria-haspopup", "listbox");
  trigger.setAttribute("aria-expanded", "false");
  if (ariaLabel) trigger.setAttribute("aria-label", ariaLabel);

  const label = document.createElement("span");
  label.className = "select-label truncate";
  label.dataset.role = "label";

  const chevron = document.createElement("span");
  chevron.className = "select-chevron";
  chevron.innerHTML = icons.chevron(12);

  trigger.append(label, chevron);

  const menu = document.createElement("div");
  menu.className = "select-menu";
  menu.dataset.align = align;
  menu.setAttribute("role", "listbox");
  if (ariaLabel) menu.setAttribute("aria-label", ariaLabel);
  menu.hidden = true;

  const items = options.map((option, index) => {
    const item = document.createElement("button");
    item.type = "button";
    item.className = "select-option";
    item.id = `${ariaLabel ? ariaLabel.replace(/\W+/g, "-").toLowerCase() : "select"}-option-${index}`;
    item.dataset.value = option.value;
    item.setAttribute("role", "option");
    item.setAttribute("aria-selected", "false");
    item.innerHTML = `<span class="truncate">${escapeHtml(option.label)}</span><span class="select-check">${icons.check(12)}</span>`;
    menu.append(item);
    return item;
  });

  root.append(trigger, menu);

  let current = value ?? options[0]?.value ?? "";
  let activeIndex = Math.max(
    0,
    options.findIndex((option) => option.value === current),
  );
  let open = false;

  function applySelection(next, notify) {
    current = next;
    const selected = options.find((option) => option.value === next);
    label.textContent = selected?.label ?? "";
    trigger.title = selected?.label ?? "";
    for (const item of items) {
      const isSelected = item.dataset.value === next;
      item.setAttribute("aria-selected", String(isSelected));
    }
    activeIndex = Math.max(0, items.findIndex((item) => item.dataset.value === next));
    if (notify) onChange?.(next);
  }

  function highlight(index) {
    if (!items.length) return;
    activeIndex = (index + items.length) % items.length;
    items.forEach((item, itemIndex) => {
      const isActive = itemIndex === activeIndex;
      item.dataset.active = String(isActive);
      if (isActive) {
        trigger.setAttribute("aria-activedescendant", item.id);
        item.scrollIntoView({ block: "nearest" });
      }
    });
  }

  /** Usable vertical space, respecting a surrounding dialog or scroll pane. */
  function bounds() {
    let top = 0;
    let bottom = window.innerHeight;
    let dialog = null;
    for (let node = root.parentElement; node; node = node.parentElement) {
      const rect = node.getBoundingClientRect();
      const overflowY = getComputedStyle(node).overflowY;
      if (
        overflowY === "hidden" ||
        overflowY === "auto" ||
        overflowY === "scroll" ||
        overflowY === "clip"
      ) {
        top = Math.max(top, rect.top);
        bottom = Math.min(bottom, rect.bottom);
      }
      if (node.getAttribute("role") === "dialog") {
        dialog = rect;
        break;
      }
    }
    if (dialog) {
      top = Math.max(top, dialog.top);
      bottom = Math.min(bottom, dialog.bottom);
    }
    return { top, bottom };
  }

  function place() {
    // Flip above the trigger when the list would not fit below it, so a dropdown
    // near the bottom of a dialog stays inside the dialog instead of hanging
    // over its edge.
    const triggerBox = trigger.getBoundingClientRect();
    const menuHeight = menu.getBoundingClientRect().height;
    const { top, bottom } = bounds();
    const spaceBelow = bottom - triggerBox.bottom;
    const spaceAbove = triggerBox.top - top;
    const upwards = spaceBelow < menuHeight + 12 && spaceAbove > spaceBelow;
    root.dataset.placement = upwards ? "up" : "down";
  }

  function openMenu() {
    if (open || !items.length) return;
    open = true;
    menu.hidden = false;
    trigger.setAttribute("aria-expanded", "true");
    place();
    highlight(activeIndex);
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("resize", onViewportChange);
    window.addEventListener("scroll", onViewportChange, true);
  }

  function closeMenu({ focusTrigger = false } = {}) {
    if (!open) return;
    open = false;
    menu.hidden = true;
    trigger.setAttribute("aria-expanded", "false");
    trigger.removeAttribute("aria-activedescendant");
    items.forEach((item) => {
      item.dataset.active = "false";
    });
    document.removeEventListener("pointerdown", onPointerDown, true);
    document.removeEventListener("keydown", onKeyDown, true);
    window.removeEventListener("resize", onViewportChange);
    window.removeEventListener("scroll", onViewportChange, true);
    if (focusTrigger) trigger.focus();
  }

  function commit(index) {
    const item = items[index];
    if (!item) return;
    applySelection(item.dataset.value, true);
    closeMenu({ focusTrigger: true });
  }

  function onPointerDown(event) {
    if (!root.contains(event.target)) closeMenu();
  }

  function onViewportChange() {
    // The menu is positioned against the trigger, so a scroll only needs the
    // flip re-evaluated. Scrolling must never close it: the highlight itself
    // scrolls the list into view while opening.
    place();
  }

  function onKeyDown(event) {
    if (!open) return;
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        highlight(activeIndex + 1);
        break;
      case "ArrowUp":
        event.preventDefault();
        highlight(activeIndex - 1);
        break;
      case "Home":
        event.preventDefault();
        highlight(0);
        break;
      case "End":
        event.preventDefault();
        highlight(items.length - 1);
        break;
      case "Enter":
      case " ":
        event.preventDefault();
        commit(activeIndex);
        break;
      case "Escape":
        event.preventDefault();
        closeMenu({ focusTrigger: true });
        break;
      case "Tab":
        closeMenu();
        break;
      default:
        break;
    }
  }

  trigger.addEventListener("click", () => {
    if (open) closeMenu({ focusTrigger: true });
    else openMenu();
  });

  trigger.addEventListener("keydown", (event) => {
    if (open) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      openMenu();
    } else if (event.key === "Escape") {
      closeMenu();
    }
  });

  for (const item of items) {
    item.addEventListener("click", () => commit(items.indexOf(item)));
  }

  applySelection(current, false);

  return {
    element: root,
    getValue: () => current,
    setValue: (next) => applySelection(next, false),
    getOptions: () => options.map((option) => option.value),
    open: () => openMenu(),
    close: () => closeMenu(),
    unmount: () => {
      closeMenu();
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
    },
  };
}
