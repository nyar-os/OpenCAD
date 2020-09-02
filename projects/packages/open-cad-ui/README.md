# open-cad-ui

VMZ static studio shell for the OpenCAD desktop window. `vmz build . --profile
static` emits `dist/static`, which the repository staging step copies to
`projects/packages/open-cad/dist/gui` for the native WebView host.

```bash
pnpm --filter @wae/open-cad-ui run check   # vmz check
pnpm --filter @wae/open-cad-ui run build   # vmz build --profile static
pnpm --filter @wae/open-cad-ui run serve   # local preview
```

The shell is deliberately small: a header, a layer rail, a viewport frame, an
inspector and a status bar. It holds no drawing data of its own. Create, open,
save, undo/redo, layer management, DXF import, inspection and SVG/DXF/PDF export
issue real host requests; the canvas keeps saying that the interactive viewport
renderer is not implemented.

The page talks to the host over `chrome.webview.postMessage` JSON-RPC (`ready`,
`capabilities`, `drawing.create`, `drawing.open`, `drawing.save`, `drawing.undo`,
`drawing.redo`, `drawing.addLayer`, `drawing.updateLayer`, `drawing.importDxf`,
`drawing.inspect`, `drawing.export`, `drawing.close`, `drawing.addLine`,
`drawing.addCircle`, `drawing.addPolyline`, `drawing.addText`,
`drawing.addDimension`, `drawing.moveEntity`, `drawing.updateEntity`,
`drawing.deleteEntity`, `drawing.listEntities`, `close`). Without a WebView host
it shows the empty state and "Preview mode — native window unavailable"; it
never renders a mock drawing.
