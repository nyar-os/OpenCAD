# Architecture

OpenCAD has one implementation of the drawing model. Every other surface — the
command line, the TypeScript SDK, the studio window and the agent skill — adapts
that model instead of reimplementing it.

## Drawing model

A drawing is persisted as a `.ocad` file: UTF-8 JSON tagged `open-cad-drawing`,
format version `1`. The document holds:

- identity, revision, title and declared unit (`mm` or `inch`);
- title-block properties;
- layers, line types, dimension styles and text styles;
- entity groups, paper-space layouts and an explicit draw order;
- the flat entity list.

Every drawing starts with layer `0` (`#d0d7de`) and layer `dimensions`
(`#0b7285`), the `continuous` line type, one dimension style and one text style.
Layouts and groups start empty.

Entity kinds are `line`, `circle`, `arc`, `ellipse`, `point`, `polyline`,
`spline`, `text`, linear `dimension`, `radial_dimension`, `ordinate_dimension`,
`angular_dimension` and `hatch`. Rectangles and regular polygons are ordinary
closed polylines, so they carry no separate entity type and need no separate
export path. A spline stores its fit points and a closed flag; rendering samples
it with uniform Catmull-Rom spans. A hatch stores one closed boundary polygon and
a pattern name (`SOLID` is a solid fill); the pattern name is a description, not
a rendered pattern definition.

Entity ids are deterministic: `entity-1`, `entity-2`, and so on. A deleted id is
never reused. Layers, line types, dimension styles and text styles are unique by
name. Renaming a layer, or renaming a line type, rewrites every reference in the
same mutation.

The revision increments on every mutation. Saving does not change it, so a
reopened file keeps the revision it was written with. Undo and redo bump the
revision because they produce a new committed state.

## Module boundaries

| Layer | Location | Responsibility |
| :--- | :--- | :--- |
| Model | `projects/crates/cad-model` (crate `open-cad-drawing`) | Schema, store, persistence, undo snapshots, entity and layer operations, inspection, DXF import subset, SVG/PDF/PNG/DXF/STL/OBJ/STEP writers |
| Binding | `projects/crates/cad-napi` (crate `open-cad-napi`) | One Node-API function per operation; JSON string in, JSON string out; process-wide drawing handles; WebView host for the studio |
| Native access | `projects/packages/open-cad/src/native.ts`, `service.ts` | Resolve the addon, decode JSON, map failures to stable `OpenCADError` codes |
| Document | `projects/packages/open-cad/src/document.ts` | Own one native handle and the last drawing summary; forward edits; sidecar history |
| Session | `projects/packages/open-cad/src/open-cad.ts` | Create, open, import, inspect, dispose; owns the documents it opens |
| Command line | `projects/packages/open-cad/src/cli.ts` | Parse options, call one session or document method, print text and set the exit code |
| Studio | `projects/packages/open-cad-ui` | WebView shell; no drawing data of its own |
| Skill | `projects/packages/open-cad-skills` | Agent routing and verified command list |

The binding maps values only. It performs no geometry, parsing or persistence
checks; those live in the model. The TypeScript document and CLI hold no drawing
logic either: `cli.ts` parses flags and calls a method, and `document.ts` forwards
to `service.ts` and keeps the summary in sync.

## Model → native → SDK/CLI flow

1. A CLI command or SDK call builds a request object.
2. `service.ts` calls one native export with JSON arguments and decodes the JSON
   reply.
3. `cad-napi` forwards the request to `cad-model` and serialises the result.
4. The model mutates or reads the document in its process-wide store, persists
   `.ocad` files and projects exchange formats.
5. The document adopts the returned summary and revision; the CLI prints it.

Drawing handles are `u64` values from the model's store and stay valid for the
process lifetime until closed. An absent addon is a declarative state, not a
crash: calls report `capability/not-implemented` instead of inventing a drawing.
The addon is resolved from `OPEN_CAD_NATIVE` when set, otherwise from
`@wae/open-cad-<platform>-<arch>`.

## Determinism and error semantics

- Stable error codes are the contract. Callers compare codes and never parse a
  message. The list lives in `projects/packages/open-cad/src/errors.ts`
  (`session/*`, `drawing/*`, `object/*`, `format/*`, `capability/*`).
- Unsupported routes report `capability/not-implemented`. Argument problems
  report `session/argument-invalid`; missing ids report `object/not-found`;
  the wrong entity type reports `object/unsupported`; a locked or frozen layer
  refuses writes with `drawing/layer-locked` or `drawing/layer-frozen`.
- The CLI routes domain failures through the command framework so the stable
  code survives. Exit codes: `0` success, `1` a failing inspection or a failed
  studio launch, `2` a usage problem (missing file, flag or value).
- Export results report what the target actually kept in `losses`, so a
  successful exit does not imply a lossless file.
- Sampling is fixed: arcs and ellipses are sampled at five-degree steps in
  measurement and mesh export, splines are sampled into uniform Catmull-Rom
  spans, and DXF import flattens ARC and ELLIPSE to polylines at five degrees
  per segment with at least eight segments. The same input produces the same
  output.

## Undo history ownership

Undo is owned by the native handle: each handle keeps up to 32 snapshots, every
mutation adds one step, and create/open start empty. Save adds no step.

A separate CLI invocation opens a new handle and therefore cannot see the
previous process's snapshots. The CLI runs with `history: 'sidecar'`, which keeps
the replaced document in `<file>.history.json`:

- up to 32 snapshots per direction, written with a temporary file and rename;
- every mutation saves the document in place first and records the previous
  content;
- `undo` and `redo` swap the file with a snapshot and bump the revision relative
  to the file they replace;
- `create` and `import` save through the SDK, which adopts the new target and
  clears that target's stale sidecar;
- saving to a new target adopts it and drops the target's stale history.

`history: 'memory'` (the SDK default) leaves snapshots inside the handle only.
The sidecar is an implementation detail of cross-process undo, not a second
drawing model.

## Workspace layout

```text
Cargo.toml                      Rust workspace (cad-model, cad-napi)
rust-toolchain.toml             nightly toolchain
package.json                    workspace scripts
pnpm-workspace.yaml             projects/packages/*, projects/sites/*
projects/crates/cad-model       drawing model, store, inspection, exchange
projects/crates/cad-napi        Node-API binding and WebView host
projects/packages/open-cad      SDK and CLI
projects/packages/open-cad-*    prebuilt addon per platform and architecture
projects/packages/open-cad-ui   studio shell
projects/packages/open-cad-skills  agent skill documents
projects/sites/homepage         product site
documentation/                  maintainer documentation
```
