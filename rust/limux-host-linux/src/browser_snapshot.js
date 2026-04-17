// limux browser snapshot walker. Invoked on-demand via evaluate_javascript.
// Returns a JSON string per the design doc:
// {
//   url, title, hash,
//   snapshot_text: "page ...\n- banner\n  - link \"Home\" [ref=e1]\n...",
//   refs: { eN: {selector, role, name, tag} },
//   shadow_closed: bool,
//   truncated: null | { logs: N, errors: N, recreated: bool }
// }
//
// Options are passed in by the server:
//   opts.full_tree    — emit non-interactive text nodes too
//   opts.raw_html     — bypass AX walker, return document.documentElement.outerHTML
//   opts.selector     — scope walker to the first matching element
//   opts.max_depth    — clamp walker depth
//   opts.since_hash   — if matches current hash, emit diff only
//
// All parameters come in via placeholder substitution as a JSON literal
// named `__LIMUX_SNAPSHOT_OPTS__`. Rust builds the call like:
//   (opts) => { ... }(__LIMUX_SNAPSHOT_OPTS__)

((opts) => {
  if (!window.__limux) {
    return JSON.stringify({ error: { code: "INIT_NOT_READY", message: "init script not yet installed" } });
  }
  const api = window.__limux;
  const { full_tree = false, raw_html = false, selector = null, max_depth = null, since_hash = null } = (opts || {});

  if (raw_html) {
    const html = document.documentElement ? document.documentElement.outerHTML : "";
    return JSON.stringify({
      url: location.href,
      title: document.title,
      raw_html: html,
      refs: api.refMeta,
      truncated: truncationReport(),
    });
  }

  const root = selector ? document.querySelector(selector) : document.body;
  if (!root) {
    return JSON.stringify({
      url: location.href,
      title: document.title,
      snapshot_text: "",
      refs: {},
      shadow_closed: false,
      truncated: truncationReport(),
      error: { code: "SELECTOR_NOT_FOUND", message: "selector root not found" },
    });
  }

  // Refresh refs on subtree first to catch elements added since last mutation tick.
  api.tagSubtree(root);

  const refs = Object.create(null);
  let shadowClosed = false;
  const lines = [];
  const depthCap = (max_depth == null) ? Infinity : Number(max_depth);

  const pageLine = `page ${location.href}  title ${jsonQuote(document.title || "")}`;
  lines.push(pageLine);

  walk(root, 0);
  const text = lines.join("\n");
  const hash = djb2(text);

  if (since_hash && since_hash === hash) {
    return JSON.stringify({
      url: location.href,
      title: document.title,
      hash,
      unchanged: true,
      truncated: truncationReport(),
    });
  }

  return JSON.stringify({
    url: location.href,
    title: document.title,
    hash,
    snapshot_text: text,
    refs,
    shadow_closed: shadowClosed,
    truncated: truncationReport(),
  });

  function walk(el, depth) {
    if (!el || el.nodeType !== 1) return;
    if (depth > depthCap) return;

    const tag = el.tagName;
    if (tag === "SCRIPT" || tag === "STYLE" || tag === "NOSCRIPT" || tag === "TEMPLATE") return;

    // Detect closed shadow DOM host so the agent knows to switch tools.
    if (el.shadowRoot === null && typeof el.attachShadow === "function") {
      // shadowRoot null doesn't prove closed mode; use getRootNode to detect open-mode hosts.
    }
    // Rough heuristic: if the element has slotted children but shadowRoot is
    // inaccessible, mark as closed. WebKit exposes open shadow via shadowRoot.
    if (el.shadowRoot && el.shadowRoot.mode === "closed") shadowClosed = true;

    const hidden = isHiddenForSnapshot(el);
    if (hidden && !el.matches("[tabindex], input, button, a[href], [role]")) return;

    const interactive = api.isInteractive(el);
    let refId = null;
    if (interactive) {
      refId = el.getAttribute("data-limux-ref") || api.assignRef(el);
      if (refId) {
        refs[refId] = {
          selector: `[data-limux-ref="${refId}"]`,
          role: api.elementRole(el),
          name: api.accessibleName(el),
          tag: tag.toLowerCase(),
        };
      }
    }

    const role = api.elementRole(el);
    const name = interactive ? api.accessibleName(el) : pickTextualName(el);

    if (interactive || full_tree || isLandmark(role) || name) {
      lines.push(formatNode(depth, role, name, refId, el, hidden));
    }

    // Iframes + open shadow DOM children traversal (cross-frame snapshot for open mode only).
    if (el.shadowRoot && el.shadowRoot.mode === "open") {
      for (const child of el.shadowRoot.children) walk(child, depth + 1);
    }

    for (const child of el.children) walk(child, depth + 1);
  }

  function formatNode(depth, role, name, refId, el, hidden) {
    const indent = "  ".repeat(depth + 1);
    const attrs = [];
    if (refId) attrs.push(`ref=${refId}`);
    if (hidden) attrs.push("hidden=true");
    if (el.hasAttribute("required")) attrs.push("required");
    if (el.hasAttribute("disabled")) attrs.push("disabled");
    if (el.hasAttribute("readonly")) attrs.push("readonly");
    if (el.type === "checkbox" || el.type === "radio") attrs.push(`checked=${!!el.checked}`);
    const ariaExpanded = el.getAttribute("aria-expanded");
    if (ariaExpanded != null) attrs.push(`expanded=${ariaExpanded}`);
    const ariaSelected = el.getAttribute("aria-selected");
    if (ariaSelected != null) attrs.push(`selected=${ariaSelected}`);
    if (el.tagName === "H1") attrs.push("level=1");
    else if (el.tagName === "H2") attrs.push("level=2");
    else if (el.tagName === "H3") attrs.push("level=3");
    else if (el.tagName === "H4") attrs.push("level=4");
    else if (el.tagName === "H5") attrs.push("level=5");
    else if (el.tagName === "H6") attrs.push("level=6");
    if (el.tagName === "INPUT") {
      const type = (el.type || "text").toLowerCase();
      if (type !== "text") attrs.push(`type=${type}`);
      if (el.placeholder) attrs.push(`placeholder=${jsonQuote(el.placeholder)}`);
      if (el.value && type !== "password") attrs.push(`value=${jsonQuote(trunc(el.value, 80))}`);
    }

    let line = `${indent}- ${role}`;
    if (name) line += ` ${jsonQuote(trunc(name, 120))}`;
    if (attrs.length) line += ` [${attrs.join(", ")}]`;
    return line;
  }

  function isHiddenForSnapshot(el) {
    if (el.hasAttribute("hidden")) return true;
    if (el.getAttribute && el.getAttribute("aria-hidden") === "true") return true;
    const style = el.ownerDocument.defaultView && el.ownerDocument.defaultView.getComputedStyle
      ? el.ownerDocument.defaultView.getComputedStyle(el)
      : null;
    if (style && (style.display === "none" || style.visibility === "hidden")) return true;
    return false;
  }

  function isLandmark(role) {
    return [
      "main", "banner", "navigation", "contentinfo", "complementary", "search",
      "form", "region", "dialog", "alert", "alertdialog", "status",
      "heading", "list", "listitem", "table", "row", "cell",
    ].includes(role);
  }

  function pickTextualName(el) {
    // For landmarks/text containers, collect own text (not descendants).
    if (!el.childNodes) return "";
    let out = "";
    for (const child of el.childNodes) {
      if (child.nodeType === 3) out += child.nodeValue;
    }
    out = out.replace(/\s+/g, " ").trim();
    if (out.length > 120) out = out.slice(0, 120);
    return out;
  }

  function jsonQuote(s) {
    return JSON.stringify(String(s == null ? "" : s));
  }
  function trunc(s, n) {
    if (s == null) return "";
    const str = String(s);
    return str.length > n ? str.slice(0, n) : str;
  }
  function djb2(str) {
    let h = 5381;
    for (let i = 0; i < str.length; i++) {
      h = ((h << 5) + h + str.charCodeAt(i)) | 0;
    }
    return "djb2:" + (h >>> 0).toString(16);
  }
  function truncationReport() {
    const logs = api.logsDroppedCount;
    const errors = api.errorsDroppedCount;
    if (logs === 0 && errors === 0) return null;
    return { logs, errors };
  }
})(__LIMUX_SNAPSHOT_OPTS__);
