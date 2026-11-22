# @wae/open-cad-ui

The studio shell for the OpenCAD desktop window. It is an internal, unpublished
package.

It renders the layer rail, viewport frame, inspector and status bar, and holds no
drawing data of its own. Every edit goes through the same document model the
command line and SDK use. The studio is a shell, not an interactive CAD canvas:
SVG output is the preview, and the viewport renderer is not implemented.

For the product overview, start at the [root README](../../../README.md). The
exact support boundary is in
[Capabilities](../../../documentation/capabilities.md).
