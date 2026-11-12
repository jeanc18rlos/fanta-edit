// Run with `node --test web/fanta-canvas-web/test/`. The DOM-dependent half
// of the component is exercised by the demo against a served project (A3
// lands with T3's browser host).
import test from "node:test";
import assert from "node:assert/strict";
import {
  nodeIdOf,
  parsePagesManifest,
  parseStylesheetHrefs,
  resolvePath,
} from "../src/fanta-canvas.js";

test("parsePagesManifest reads the emitted pages.js shape", () => {
  const pages = parsePagesManifest(
    'const FANTA_PAGES = [\n  {\n    "slug": "home",\n    "name": "Home",\n' +
      '    "width": 1280.0,\n    "height": 800.0\n  }\n];\n',
  );
  assert.deepEqual(pages, [
    { slug: "home", name: "Home", width: 1280, height: 800 },
  ]);
  assert.throws(() => parsePagesManifest("const FANTA_PAGES = 7;"));
});

test("parseStylesheetHrefs reads the SVG's processing instructions", () => {
  const svg =
    '<?xml-stylesheet type="text/css" href="../../tokens.css"?>\n' +
    '<?xml-stylesheet type="text/css" href="page.css"?>\n' +
    '<svg xmlns="http://www.w3.org/2000/svg" data-fanta="page"></svg>';
  assert.deepEqual(parseStylesheetHrefs(svg), ["../../tokens.css", "page.css"]);
  assert.deepEqual(parseStylesheetHrefs("<svg></svg>"), []);
});

test("resolvePath resolves hrefs against the SVG's directory", () => {
  assert.equal(
    resolvePath("pages/home/page.svg", "../../tokens.css"),
    "tokens.css",
  );
  assert.equal(resolvePath("pages/home/page.svg", "page.css"), "pages/home/page.css");
  assert.equal(
    resolvePath("components/card/master.svg", "./master.css"),
    "components/card/master.css",
  );
});

test("nodeIdOf maps element ids to their node id", () => {
  const node = "n_01M75FTSJ121EWN0A9WJXTZXSV";
  assert.equal(nodeIdOf(node), node);
  assert.equal(nodeIdOf(`${node}-bg`), node);
  assert.equal(nodeIdOf(`${node}-v1-clip`), node);
  assert.equal(nodeIdOf("page-root"), null);
  assert.equal(nodeIdOf(""), null);
  // ULIDs never hold I, L, O or U.
  assert.equal(nodeIdOf("n_01M75FTSJ121EWN0A9WJXTZXSI"), null);
});
