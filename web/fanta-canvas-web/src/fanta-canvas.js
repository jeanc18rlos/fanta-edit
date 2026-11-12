// fanta-canvas-web: the host-neutral canvas component (spec 0008 T2).
//
// The browser is the renderer: the component injects the project's own
// `page.svg` and the stylesheets that SVG names in its `<?xml-stylesheet?>`
// processing instructions, and draws selection above them. Hosts supply only
// file access; with a T1 wasm engine the canvas also applies `batch_design`
// ops and swaps the re-emitted SVG in place, no reload (R3, R4, R5).
//
//   import { createFantaCanvas } from "./fanta-canvas.js";
//   const canvas = await createFantaCanvas({
//     container,                       // a block element the canvas fills
//     files: {
//       read,                          // async (path) -> Uint8Array | string
//       write,                         // async (path, Uint8Array); engine mode
//       list,                          // async () -> [path]; engine mode
//       watch,                         // optional (onChange) -> dispose
//     },
//     engine,                          // optional T1 FantaEngine instance
//     onSelect,                        // optional ([nodeId]) callback
//   });
//   await canvas.showPage(slug);       // slugs from canvas.pages
//   await canvas.applyOps(ops, label); // engine mode: edit, save, swap
//
// Dependency-free ES module; no DOM assumptions beyond the container.

const NODE_ID = /^(n_[0-9A-HJKMNP-TV-Z]{26})/;
const MIN_ZOOM = 0.05;
const MAX_ZOOM = 8;

/// The page list from an emitted `pages.js` (`const FANTA_PAGES = [...];`).
export function parsePagesManifest(text) {
  const start = text.indexOf("[");
  const end = text.lastIndexOf("]");
  if (start === -1 || end <= start) {
    throw new Error("pages.js holds no FANTA_PAGES array");
  }
  return JSON.parse(text.slice(start, end + 1));
}

/// The stylesheet hrefs a standalone SVG names, in order. Browsers ignore
/// `<?xml-stylesheet?>` on inlined markup, so the component loads these
/// itself.
export function parseStylesheetHrefs(svgText) {
  const hrefs = [];
  const instruction = /<\?xml-stylesheet\b[^?]*\?>/g;
  for (const match of svgText.slice(0, 4096).matchAll(instruction)) {
    const href = /href="([^"]+)"/.exec(match[0]);
    if (href) {
      hrefs.push(href[1]);
    }
  }
  return hrefs;
}

/// The node id an element id belongs to: emitted ids are the node id in its
/// display form, sometimes suffixed (`n_<ulid>-bg`, `n_<ulid>-v1-clip`).
export function nodeIdOf(elementId) {
  const match = NODE_ID.exec(elementId || "");
  return match ? match[1] : null;
}

/// `relative` resolved against the directory of `from`, both project paths.
export function resolvePath(from, relative) {
  const parts = from.split("/").slice(0, -1);
  for (const part of relative.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") {
      parts.pop();
    } else {
      parts.push(part);
    }
  }
  return parts.join("/");
}

function asText(data) {
  return typeof data === "string" ? data : new TextDecoder().decode(data);
}

export async function createFantaCanvas(options) {
  const { container, files, engine = null, onSelect = null } = options;
  if (!container || !files || typeof files.read !== "function") {
    throw new Error("createFantaCanvas needs a container and files.read");
  }

  container.innerHTML = "";
  const stage = document.createElement("div");
  stage.style.cssText =
    "position:relative;overflow:hidden;width:100%;height:100%;" +
    "touch-action:none;background:transparent;";
  const styles = document.createElement("div");
  const sheet = document.createElement("div");
  sheet.style.cssText = "position:absolute;transform-origin:0 0;";
  const overlay = document.createElement("div");
  overlay.style.cssText =
    "position:absolute;inset:0;pointer-events:none;overflow:hidden;";
  stage.append(styles, sheet, overlay);
  container.appendChild(stage);

  const pages = options.pages
    ? options.pages.slice()
    : parsePagesManifest(asText(await files.read("pages.js")));

  const state = {
    page: null,
    scale: 1,
    offsetX: 0,
    offsetY: 0,
    selection: [],
    disposed: false,
  };

  function apply() {
    sheet.style.transform =
      `translate(${state.offsetX}px, ${state.offsetY}px) scale(${state.scale})`;
    drawSelection();
  }

  function fit() {
    const page = state.page;
    if (!page) return;
    const pad = 32;
    const width = Math.max(1, page.width || 1);
    const height = Math.max(1, page.height || 1);
    state.scale = Math.min(
      (stage.clientWidth - pad) / width,
      (stage.clientHeight - pad) / height,
      1,
    );
    if (!isFinite(state.scale) || state.scale <= 0) state.scale = 1;
    state.offsetX = (stage.clientWidth - width * state.scale) / 2;
    state.offsetY = (stage.clientHeight - height * state.scale) / 2;
    apply();
  }

  async function loadStylesheets(svgPath, svgText) {
    const loaded = [];
    for (const href of parseStylesheetHrefs(svgText)) {
      const path = resolvePath(svgPath, href);
      try {
        const css = asText(await files.read(path));
        const style = document.createElement("style");
        style.dataset.fantaCss = path;
        style.textContent = css;
        loaded.push(style);
      } catch (error) {
        console.warn(`fanta-canvas: stylesheet ${path} failed to load`, error);
      }
    }
    styles.replaceChildren(...loaded);
  }

  // Keep `<image href="assets/…">` working for hosts without a base URL for
  // the project (panes, virtual file systems) by inlining the bytes.
  async function resolveAssetHrefs(svg) {
    const images = svg.querySelectorAll("image");
    for (const image of images) {
      const href =
        image.getAttribute("href") || image.getAttribute("xlink:href");
      if (!href || !href.startsWith("assets/")) continue;
      try {
        const bytes = await files.read(href);
        if (typeof bytes === "string") continue;
        const blob = new Blob([bytes]);
        image.setAttribute("href", URL.createObjectURL(blob));
      } catch {
        // The href keeps working on hosts that serve the project directory.
      }
    }
  }

  async function mountSvg(svgPath, svgText, keepView) {
    await loadStylesheets(svgPath, svgText);
    sheet.innerHTML = svgText;
    const svg = sheet.querySelector("svg");
    if (svg && options.resolveAssets !== false) {
      await resolveAssetHrefs(svg);
    }
    if (!keepView) fit();
    state.selection = state.selection.filter((id) =>
      sheet.querySelector(`[id^="${id}"]`),
    );
    apply();
  }

  async function showPage(slug) {
    const page = pages.find((entry) => entry.slug === slug) || null;
    if (!page) throw new Error(`no page is named ${slug}`);
    state.page = page;
    const path = `pages/${page.slug}/page.svg`;
    await mountSvg(path, asText(await files.read(path)), false);
  }

  async function refresh() {
    if (!state.page) return;
    const path = `pages/${state.page.slug}/page.svg`;
    await mountSvg(path, asText(await files.read(path)), true);
  }

  // ---- selection ------------------------------------------------------------

  function drawSelection() {
    overlay.replaceChildren();
    const stageBox = stage.getBoundingClientRect();
    for (const id of state.selection) {
      const target =
        sheet.querySelector(`#${CSS.escape(id)}`) ||
        sheet.querySelector(`[id^="${id}"]`);
      if (!target) continue;
      const box = target.getBoundingClientRect();
      const outline = document.createElement("div");
      outline.dataset.fantaSelection = id;
      outline.style.cssText =
        "position:absolute;border:1.5px solid #4f7cff;border-radius:1px;" +
        "box-shadow:0 0 0 1px rgba(255,255,255,0.7);" +
        `left:${box.left - stageBox.left}px;top:${box.top - stageBox.top}px;` +
        `width:${box.width}px;height:${box.height}px;`;
      overlay.appendChild(outline);
    }
  }

  function select(ids) {
    state.selection = ids;
    drawSelection();
    if (onSelect) onSelect(ids.slice());
  }

  function hitNodeId(eventTarget) {
    let element = eventTarget instanceof Element ? eventTarget : null;
    while (element && element !== sheet) {
      const id = nodeIdOf(element.id);
      if (id) return id;
      element = element.parentElement;
    }
    return null;
  }

  // ---- pan, zoom, click -----------------------------------------------------

  let gesture = null;
  stage.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    gesture = {
      startX: event.clientX,
      startY: event.clientY,
      originX: state.offsetX,
      originY: state.offsetY,
      moved: false,
      // Resolved now: setPointerCapture retargets the matching pointerup
      // to the stage, so the up event no longer says what was under it.
      hit: hitNodeId(event.target),
    };
    stage.setPointerCapture(event.pointerId);
  });
  stage.addEventListener("pointermove", (event) => {
    if (!gesture) return;
    const dx = event.clientX - gesture.startX;
    const dy = event.clientY - gesture.startY;
    if (Math.abs(dx) + Math.abs(dy) > 3) gesture.moved = true;
    if (gesture.moved) {
      state.offsetX = gesture.originX + dx;
      state.offsetY = gesture.originY + dy;
      apply();
    }
  });
  stage.addEventListener("pointerup", (event) => {
    const finished = gesture;
    gesture = null;
    if (!finished || finished.moved) return;
    const id = finished.hit;
    if (!id) {
      select([]);
    } else if (event.shiftKey) {
      select(
        state.selection.includes(id)
          ? state.selection.filter((existing) => existing !== id)
          : [...state.selection, id],
      );
    } else {
      select([id]);
    }
  });
  stage.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      const factor = Math.exp(-event.deltaY * 0.0015);
      const next = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, state.scale * factor));
      const rect = stage.getBoundingClientRect();
      const px = event.clientX - rect.left;
      const py = event.clientY - rect.top;
      state.offsetX = px - ((px - state.offsetX) / state.scale) * next;
      state.offsetY = py - ((py - state.offsetY) / state.scale) * next;
      state.scale = next;
      apply();
    },
    { passive: false },
  );

  // ---- editing through the T1 engine -----------------------------------------

  let engineLoaded = false;
  async function loadEngine() {
    if (engineLoaded) return;
    if (!engine) throw new Error("this canvas was created without an engine");
    if (typeof files.list !== "function" || typeof files.write !== "function") {
      throw new Error("engine mode needs files.list and files.write");
    }
    for (const path of await files.list()) {
      const data = await files.read(path);
      const bytes =
        typeof data === "string" ? new TextEncoder().encode(data) : data;
      engine.add_file(path, bytes);
    }
    JSON.parse(engine.load());
    engineLoaded = true;
  }

  /// Apply a `batch_design` ops array as one undo step, write every re-emitted
  /// file back through the host, and swap the current page's SVG in place.
  async function applyOps(ops, label = "Canvas edit") {
    await loadEngine();
    const result = JSON.parse(
      engine.apply(JSON.stringify(ops), label, true),
    );
    const emitted = JSON.parse(engine.emit());
    let currentSvg = null;
    const currentPath = state.page ? `pages/${state.page.slug}/page.svg` : null;
    for (const path of emitted.files) {
      const bytes = engine.take_file(path);
      if (!bytes) continue;
      await files.write(path, bytes);
      if (path === currentPath) currentSvg = bytes;
    }
    if (currentSvg) {
      await mountSvg(currentPath, asText(currentSvg), true);
    } else {
      await refresh();
    }
    return result;
  }

  const stopWatching =
    typeof files.watch === "function"
      ? files.watch(() => {
          if (!state.disposed) refresh();
        })
      : null;
  const onResize = () => fit();
  window.addEventListener("resize", onResize);

  if (pages.length > 0) {
    await showPage(pages[0].slug);
  }

  return {
    pages,
    showPage,
    refresh,
    applyOps,
    select,
    selection: () => state.selection.slice(),
    zoomToFit: fit,
    currentPage: () => (state.page ? state.page.slug : null),
    dispose() {
      state.disposed = true;
      window.removeEventListener("resize", onResize);
      if (stopWatching) stopWatching();
      container.innerHTML = "";
    },
  };
}
