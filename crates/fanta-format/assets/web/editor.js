// Edit mode for the emitted viewer (spec 0008 T3). This module activates only
// under a host that can write — `script/serve-project` answers
// /__fanta/capabilities — and otherwise exits, leaving viewer.js as the
// read-only canvas. When active it replaces the viewer's stage and page
// buttons with the fanta-canvas-web component (canvas.js, emitted beside this
// file), loads the wasm engine the host serves from outside the project tree,
// edits through `batch_design` ops, saves every re-emitted file back with
// HTTP PUT, and polls the host's change stamp so edits arriving from outside
// — an agent over fanta-mcp, a git checkout — appear live.
(async function () {
  "use strict";
  let capabilities;
  try {
    const response = await fetch("__fanta/capabilities");
    if (!response.ok) return;
    capabilities = await response.json();
  } catch {
    return;
  }
  if (!capabilities || capabilities.write !== true) return;
  // Whether the running Fanta app has this project open: then edits route
  // through its live MCP server (`POST __fanta/ops`) instead of local
  // writes, and this canvas refreshes when the app's autosave lands.
  let live = capabilities.live === true;

  const { createFantaCanvas } = await import("./canvas.js");

  let engine = null;
  if (capabilities.engine === true) {
    try {
      const wasm = await import("./__fanta/engine/fanta_wasm_engine.js");
      await wasm.default();
      engine = new wasm.FantaEngine();
    } catch (error) {
      console.warn("fanta editor: the wasm engine failed to load", error);
    }
  }

  const files = {
    async read(path) {
      const response = await fetch(path);
      if (!response.ok) throw new Error(`${path}: ${response.status}`);
      return new Uint8Array(await response.arrayBuffer());
    },
    async write(path, bytes) {
      const response = await fetch(path, { method: "PUT", body: bytes });
      if (!response.ok) throw new Error(`saving ${path}: ${response.status}`);
    },
    async list() {
      const response = await fetch("__fanta/files");
      if (!response.ok) throw new Error(`listing files: ${response.status}`);
      return (await response.json()).files;
    },
  };

  // A fresh stage node: the viewer's pan handlers capture the pointer on the
  // old one, which would steal every pointerup from the canvas component.
  // cloneNode keeps the attributes (and so the stylesheet) but no listeners.
  const viewerStage = document.getElementById("stage");
  const stage = viewerStage.cloneNode(false);
  viewerStage.replaceWith(stage);
  const nav = document.getElementById("pages");
  const container = document.createElement("div");
  container.style.cssText = "position:absolute;inset:0;";
  stage.replaceChildren(container);

  const badge = document.createElement("div");
  badge.style.cssText =
    "position:absolute;right:14px;bottom:14px;z-index:2;display:flex;gap:8px;" +
    "align-items:center;background:rgba(28,28,28,0.82);color:#fff;" +
    "border-radius:8px;padding:5px 10px;font-size:12px;";
  const mode = document.createElement("span");
  function baseMode() {
    if (live) return "Editing (live app)";
    return engine ? "Editing" : "Viewing (no engine)";
  }
  mode.textContent = baseMode();
  const fitButton = document.createElement("button");
  fitButton.textContent = "Fit";
  fitButton.style.cssText =
    "border:0;background:none;color:inherit;font:inherit;cursor:pointer;";
  badge.append(mode, fitButton);
  stage.appendChild(badge);

  function note(text) {
    mode.textContent = text;
  }

  const canvas = await createFantaCanvas({
    container,
    files,
    engine,
    editable: engine !== null || live,
    async applyRemote(ops, label) {
      if (!live) return null;
      const response = await fetch("__fanta/ops", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ ops, label }),
      });
      if (!response.ok) return null;
      return await response.json();
    },
    onSelect(ids) {
      if (!engine && !live) return;
      note(ids.length ? `${ids.length} selected` : baseMode());
    },
    onError(error) {
      note(`Save failed: ${error.message || error}`);
    },
  });
  fitButton.addEventListener("click", canvas.zoomToFit);

  function showPage(slug) {
    canvas.showPage(slug).catch((error) => note(`${error.message || error}`));
    for (const button of nav.querySelectorAll("button")) {
      if (button.dataset.slug === slug) {
        button.setAttribute("aria-current", "page");
      } else {
        button.removeAttribute("aria-current");
      }
    }
    if (location.hash.slice(1) !== slug) {
      history.replaceState(null, "", "#" + slug);
    }
  }

  // The viewer's buttons fetch into the stage this module replaced; rebuild
  // them on the component.
  nav.replaceChildren();
  for (const page of canvas.pages) {
    const button = document.createElement("button");
    button.textContent = page.name;
    button.dataset.slug = page.slug;
    button.addEventListener("click", function () {
      showPage(page.slug);
    });
    nav.appendChild(button);
  }
  const requested = location.hash.slice(1);
  const initial =
    canvas.pages.find((page) => page.slug === requested) || canvas.pages[0];
  if (initial) showPage(initial.slug);

  // Outside edits: refresh when the host's change stamp moves.
  let stamp = capabilities.stamp;
  setInterval(async function () {
    try {
      const response = await fetch("__fanta/capabilities");
      if (!response.ok) return;
      const current = await response.json();
      if (live !== (current.live === true)) {
        live = current.live === true;
        note(baseMode());
      }
      if (stamp !== undefined && current.stamp !== stamp) {
        await canvas.refresh();
      }
      stamp = current.stamp;
    } catch {
      // The host went away; keep the canvas as it stands.
    }
  }, 2000);
})();
