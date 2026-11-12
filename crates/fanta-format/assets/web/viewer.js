// Dependency-free viewer for the plain web projection: page navigation and
// pan/zoom over the per-page SVGs. Each page SVG is fetched and inlined into
// the document so tokens.css and design.css style it; that is also why the
// folder must be served rather than opened over file://. Generated files:
// pages.js (the manifest), tokens.css, design.css, pages/<slug>.svg.
(function () {
  "use strict";
  var stage = document.getElementById("stage");
  var canvas = document.getElementById("canvas");
  var nav = document.getElementById("pages");
  var zoomLevel = document.getElementById("zoom-level");
  var pages = window.FANTA_PAGES || (typeof FANTA_PAGES !== "undefined" ? FANTA_PAGES : []);

  if (location.protocol === "file:") {
    document.getElementById("notice").hidden = false;
  }

  var current = null;
  var scale = 1;
  var offsetX = 0;
  var offsetY = 0;

  function apply() {
    canvas.style.transform =
      "translate(" + offsetX + "px, " + offsetY + "px) scale(" + scale + ")";
    zoomLevel.textContent = Math.round(scale * 100) + "%";
  }

  function fit() {
    if (!current) return;
    var pad = 48;
    var sw = stage.clientWidth - pad;
    var sh = stage.clientHeight - pad;
    scale = Math.min(sw / current.width, sh / current.height, 1);
    if (!isFinite(scale) || scale <= 0) scale = 1;
    offsetX = (stage.clientWidth - current.width * scale) / 2;
    offsetY = (stage.clientHeight - current.height * scale) / 2;
    apply();
  }

  function show(page) {
    current = page;
    fetch("pages/" + page.slug + "/page.svg")
      .then(function (response) {
        if (!response.ok) throw new Error(response.status);
        return response.text();
      })
      .then(function (markup) {
        if (current !== page) return;
        canvas.innerHTML = markup;
        fit();
      })
      .catch(function () {
        document.getElementById("notice").hidden = false;
      });
    var buttons = nav.querySelectorAll("button");
    for (var i = 0; i < buttons.length; i++) {
      if (buttons[i].dataset.slug === page.slug) {
        buttons[i].setAttribute("aria-current", "page");
      } else {
        buttons[i].removeAttribute("aria-current");
      }
    }
    if (location.hash.slice(1) !== page.slug) {
      history.replaceState(null, "", "#" + page.slug);
    }
    fit();
  }

  pages.forEach(function (page) {
    var button = document.createElement("button");
    button.textContent = page.name;
    button.dataset.slug = page.slug;
    button.addEventListener("click", function () {
      show(page);
    });
    nav.appendChild(button);
  });

  var panning = null;
  stage.addEventListener("pointerdown", function (event) {
    if (event.button !== 0) return;
    panning = { x: event.clientX - offsetX, y: event.clientY - offsetY };
    stage.classList.add("panning");
    stage.setPointerCapture(event.pointerId);
  });
  stage.addEventListener("pointermove", function (event) {
    if (!panning) return;
    offsetX = event.clientX - panning.x;
    offsetY = event.clientY - panning.y;
    apply();
  });
  stage.addEventListener("pointerup", function () {
    panning = null;
    stage.classList.remove("panning");
  });
  stage.addEventListener(
    "wheel",
    function (event) {
      event.preventDefault();
      var factor = Math.exp(-event.deltaY * 0.0015);
      var next = Math.min(8, Math.max(0.05, scale * factor));
      var rect = stage.getBoundingClientRect();
      var px = event.clientX - rect.left;
      var py = event.clientY - rect.top;
      offsetX = px - ((px - offsetX) / scale) * next;
      offsetY = py - ((py - offsetY) / scale) * next;
      scale = next;
      apply();
    },
    { passive: false }
  );
  function zoomBy(factor) {
    var next = Math.min(8, Math.max(0.05, scale * factor));
    var cx = stage.clientWidth / 2;
    var cy = stage.clientHeight / 2;
    offsetX = cx - ((cx - offsetX) / scale) * next;
    offsetY = cy - ((cy - offsetY) / scale) * next;
    scale = next;
    apply();
  }
  document.getElementById("zoom-in").addEventListener("click", function () {
    zoomBy(1.25);
  });
  document.getElementById("zoom-out").addEventListener("click", function () {
    zoomBy(0.8);
  });
  document.getElementById("zoom-fit").addEventListener("click", fit);
  window.addEventListener("resize", fit);

  var initial = pages[0];
  var requested = location.hash.slice(1);
  for (var i = 0; i < pages.length; i++) {
    if (pages[i].slug === requested) initial = pages[i];
  }
  if (initial) show(initial);
})();
