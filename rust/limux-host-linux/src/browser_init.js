// limux browser automation init script
// Runs at DocumentStart on every top-frame navigation, before any page
// script. Exposes `window.__limux` with ref tagging, console/error ring
// buffers, history observers, and ready/editable probes.

(() => {
  if (window.__limux) return;

  // webkit6 JSC returns -2147483648 for Date.now() and performance.timeOrigin
  // on most pages (wall-clock clamping to i32::MIN). performance.now() is the
  // only reliable time source — it's monotonic, relative to page load, and
  // comes back as a real number. We pair it with a sequence counter so the
  // caller can filter by "since last call" deterministically.
  let seq = 0;
  const nowMs = () => Math.floor(performance.now ? performance.now() : 0);
  const nextSeq = () => ++seq;


  const LOG_CAP = 5000;
  const ERR_CAP = 5000;
  const INTERACTIVE_TAGS = new Set([
    "A", "BUTTON", "INPUT", "SELECT", "TEXTAREA", "SUMMARY", "DETAILS",
    "LABEL",
  ]);
  const INTERACTIVE_ROLES = new Set([
    "button", "link", "checkbox", "radio", "menuitem", "menuitemcheckbox",
    "menuitemradio", "tab", "option", "combobox", "textbox", "searchbox",
    "slider", "spinbutton", "switch", "treeitem", "listitem",
  ]);

  const state = {
    nextRefId: 1,
    refMeta: Object.create(null),
    logs: [],
    logsDroppedCount: 0,
    errors: [],
    errorsDroppedCount: 0,
    navCount: 0,
    lastMutationAt: 0,
    mutationQuietMs: 500,
  };

  function isInteractive(el) {
    if (!el || el.nodeType !== 1) return false;
    if (INTERACTIVE_TAGS.has(el.tagName)) {
      if (el.tagName === "A") return !!el.getAttribute("href");
      if (el.tagName === "LABEL") return !!el.getAttribute("for");
      return true;
    }
    const role = el.getAttribute && el.getAttribute("role");
    if (role && INTERACTIVE_ROLES.has(role.toLowerCase())) return true;
    if (el.isContentEditable) return true;
    const tabindex = el.getAttribute && el.getAttribute("tabindex");
    if (tabindex != null && Number.parseInt(tabindex, 10) >= 0) return true;
    return false;
  }

  function accessibleName(el) {
    const aria = el.getAttribute && el.getAttribute("aria-label");
    if (aria) return aria.trim();
    const labelledBy = el.getAttribute && el.getAttribute("aria-labelledby");
    if (labelledBy) {
      const parts = labelledBy.split(/\s+/)
        .map((id) => document.getElementById(id))
        .filter(Boolean)
        .map((n) => (n.textContent || "").trim())
        .filter(Boolean);
      if (parts.length) return parts.join(" ");
    }
    if (el.tagName === "INPUT" || el.tagName === "SELECT" || el.tagName === "TEXTAREA") {
      if (el.id) {
        const label = document.querySelector(`label[for="${CSS.escape(el.id)}"]`);
        if (label) return (label.textContent || "").trim();
      }
      if (el.placeholder) return el.placeholder;
      if (el.name) return el.name;
    }
    if (el.tagName === "IMG") {
      const alt = el.getAttribute("alt");
      if (alt) return alt;
    }
    const title = el.getAttribute && el.getAttribute("title");
    if (title) return title;
    const text = (el.innerText || el.textContent || "").replace(/\s+/g, " ").trim();
    if (text.length > 120) return text.slice(0, 120);
    return text;
  }

  function elementRole(el) {
    const explicit = el.getAttribute && el.getAttribute("role");
    if (explicit) return explicit.toLowerCase();
    switch (el.tagName) {
      case "A": return "link";
      case "BUTTON": return "button";
      case "SELECT": return "combobox";
      case "TEXTAREA": return "textbox";
      case "INPUT": {
        const type = (el.type || "text").toLowerCase();
        if (type === "checkbox") return "checkbox";
        if (type === "radio") return "radio";
        if (type === "submit" || type === "button" || type === "reset") return "button";
        if (type === "range") return "slider";
        if (type === "search") return "searchbox";
        return "textbox";
      }
      case "SUMMARY": return "button";
      case "DETAILS": return "group";
      case "LABEL": return "LabelText";
      default: return el.tagName.toLowerCase();
    }
  }

  function assignRef(el) {
    if (!isInteractive(el)) return null;
    let id = el.getAttribute("data-limux-ref");
    if (id) {
      // refresh metadata in case attributes changed
      state.refMeta[id] = {
        tag: el.tagName.toLowerCase(),
        role: elementRole(el),
        name: accessibleName(el),
      };
      return id;
    }
    id = "e" + state.nextRefId++;
    try { el.setAttribute("data-limux-ref", id); } catch (_) { return null; }
    state.refMeta[id] = {
      tag: el.tagName.toLowerCase(),
      role: elementRole(el),
      name: accessibleName(el),
    };
    return id;
  }

  function tagSubtree(root) {
    if (!root) return;
    if (root.nodeType === 1) assignRef(root);
    if (root.querySelectorAll) {
      const all = root.querySelectorAll(
        "a[href], button, input, select, textarea, summary, details, label[for], " +
        "[role], [contenteditable], [tabindex]"
      );
      for (let i = 0; i < all.length; i++) assignRef(all[i]);
    }
  }

  function releaseRef(el) {
    if (!el || el.nodeType !== 1) return;
    const id = el.getAttribute && el.getAttribute("data-limux-ref");
    if (!id) return;
    delete state.refMeta[id];
  }

  function releaseSubtree(root) {
    if (!root) return;
    if (root.nodeType === 1) releaseRef(root);
    if (root.querySelectorAll) {
      const all = root.querySelectorAll("[data-limux-ref]");
      for (let i = 0; i < all.length; i++) releaseRef(all[i]);
    }
  }

  function pushRing(buffer, entry, cap, dropCounterKey) {
    if (buffer.length >= cap) {
      buffer.shift();
      state[dropCounterKey]++;
    }
    buffer.push(entry);
  }

  function recordLog(level, args) {
    try {
      const parts = [];
      for (let i = 0; i < args.length; i++) {
        const a = args[i];
        if (a == null) { parts.push(String(a)); continue; }
        if (typeof a === "string") { parts.push(a); continue; }
        if (a instanceof Error) { parts.push(a.stack || a.message); continue; }
        try { parts.push(JSON.stringify(a)); } catch (_) { parts.push(String(a)); }
      }
      pushRing(state.logs, {
        seq: nextSeq(),
        ts_ms: nowMs(),
        level,
        text: parts.join(" "),
      }, LOG_CAP, "logsDroppedCount");
    } catch (_) { /* never throw from console hook */ }
  }

  function installConsoleHook() {
    const levels = ["log", "warn", "error", "info", "debug"];
    for (const level of levels) {
      const original = console[level] ? console[level].bind(console) : null;
      console[level] = function () {
        recordLog(level, arguments);
        if (original) original.apply(null, arguments);
      };
    }
  }

  function installErrorHook() {
    window.addEventListener("error", (ev) => {
      pushRing(state.errors, {
        seq: nextSeq(),
        ts_ms: nowMs(),
        source: "error",
        message: ev.message,
        filename: ev.filename,
        lineno: ev.lineno,
        colno: ev.colno,
        stack: ev.error && ev.error.stack ? ev.error.stack : null,
      }, ERR_CAP, "errorsDroppedCount");
    }, true);
    window.addEventListener("unhandledrejection", (ev) => {
      let reason = ev.reason;
      if (reason instanceof Error) reason = reason.stack || reason.message;
      else { try { reason = JSON.stringify(reason); } catch (_) { reason = String(reason); } }
      pushRing(state.errors, {
        seq: nextSeq(),
        ts_ms: nowMs(),
        source: "unhandledrejection",
        message: String(reason),
      }, ERR_CAP, "errorsDroppedCount");
    }, true);
  }

  function installHistoryHook() {
    const fire = (kind, url) => {
      state.navCount++;
      try {
        window.dispatchEvent(new CustomEvent("limux:navigation", {
          detail: { kind, url, navCount: state.navCount },
        }));
      } catch (_) { /* ignore */ }
    };
    const origPush = history.pushState;
    history.pushState = function (data, title, url) {
      const ret = origPush.apply(this, arguments);
      fire("pushState", url || location.href);
      return ret;
    };
    const origReplace = history.replaceState;
    history.replaceState = function (data, title, url) {
      const ret = origReplace.apply(this, arguments);
      fire("replaceState", url || location.href);
      return ret;
    };
    window.addEventListener("popstate", () => fire("popstate", location.href), true);
  }

  function installMutationObserver() {
    const obs = new MutationObserver((records) => {
      state.lastMutationAt = nowMs();
      for (const rec of records) {
        if (rec.type === "childList") {
          for (const node of rec.addedNodes) tagSubtree(node);
          for (const node of rec.removedNodes) releaseSubtree(node);
        } else if (rec.type === "attributes" && rec.target && rec.target.nodeType === 1) {
          assignRef(rec.target);
        }
      }
    });
    const start = () => {
      if (!document.documentElement) return;
      obs.observe(document.documentElement, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: [
          "role", "aria-label", "aria-labelledby", "href", "for", "tabindex",
          "contenteditable", "disabled", "checked", "value", "placeholder",
          "name", "type",
        ],
      });
      tagSubtree(document.body);
    };
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", start, { once: true });
    } else {
      start();
    }
  }

  // API surface. Frozen — reads cannot accidentally mutate internal state.
  const api = Object.freeze({
    version: 1,
    refMeta: state.refMeta,
    get logs() { return state.logs.slice(); },
    get errors() { return state.errors.slice(); },
    get logsDroppedCount() { return state.logsDroppedCount; },
    get errorsDroppedCount() { return state.errorsDroppedCount; },
    get navCount() { return state.navCount; },
    isReady() {
      if (document.readyState !== "complete") return false;
      if (state.lastMutationAt === 0) return true;
      return (nowMs() - state.lastMutationAt) >= state.mutationQuietMs;
    },
    isEditable() {
      const a = document.activeElement;
      if (!a) return false;
      if (a.isContentEditable) return true;
      const tag = a.tagName;
      if (tag === "INPUT") {
        const type = (a.type || "text").toLowerCase();
        return !["button", "submit", "reset", "checkbox", "radio", "range"].includes(type);
      }
      return tag === "TEXTAREA";
    },
    clearLogs() { state.logs.length = 0; state.logsDroppedCount = 0; },
    clearErrors() { state.errors.length = 0; state.errorsDroppedCount = 0; },
    lookupRef(id) {
      const el = document.querySelector(`[data-limux-ref="${CSS.escape(id)}"]`);
      return el || null;
    },
    refInfo(id) {
      const meta = state.refMeta[id];
      if (!meta) return null;
      const el = this.lookupRef(id);
      return { id, tag: meta.tag, role: meta.role, name: meta.name, attached: !!el };
    },
    tagSubtree: tagSubtree,
    assignRef: assignRef,
    elementRole: elementRole,
    accessibleName: accessibleName,
    isInteractive: isInteractive,
  });
  Object.defineProperty(window, "__limux", { value: api, configurable: false, writable: false });

  installConsoleHook();
  installErrorHook();
  installHistoryHook();
  installMutationObserver();
})();
