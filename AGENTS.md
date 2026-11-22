# AGENTS.md

## Repository purpose

OpenCAD is a 2D technical drawing tool: one `.ocad` document model behind an
`open-cad` command line and a TypeScript SDK, with a declared ASCII DXF import
subset and SVG/PDF/DXF/STL/OBJ/STEP export. This repository is the whole product:
drawing model, native binding, SDK/CLI, studio shell, agent skill and site.

## Documentation map

| Path | Audience | Owns |
| :--- | :--- | :--- |
| `README.md` | readers | Product landing, getting started, highlights |
| `documentation/architecture.md` | maintainers | Drawing model, modules, call path, determinism, history, layout |
| `documentation/capabilities.md` | everyone | The support matrix, DXF subset, loss codes and boundaries |
| `documentation/cli.md` | users | Every command and flag |
| `documentation/development.md` | maintainers | Build, test, native staging, CI and release |
| `projects/packages/open-cad/README.md` | npm readers | Exports, usage, constraints |
| `AGENTS.md` | agents | This file |

Change the owning file, not a copy. Never put build commands, workspace trees or
CI notes in a reader-facing README.

## Architecture invariants

1. `projects/crates/cad-model` is the only implementation of the drawing model.
   Operations, validation, persistence, inspection, import and export live there.
2. `projects/crates/cad-napi` maps values only: JSON in, JSON out, one function
   per operation. It must not contain geometry, parsing or policy.
3. `projects/packages/open-cad/src/document.ts` and `cli.ts` must not duplicate
   domain logic. The CLI parses flags and calls one method; the document forwards
   to `service.ts` and tracks the summary.
4. `capabilities()` is the only support claim. Never state support in prose,
   comments or a skill without a matching capability entry.
5. Unsupported routes return `capability/not-implemented`. Never add a silent
   fallback or a second implementation.
6. Entity ids are deterministic (`entity-N`) and never reused. Revision bumps on
   every mutation, not on save.
7. Undo belongs to the native handle (32 snapshots). The CLI sidecar is a
   transport for cross-process undo, not a second model.
8. Reader READMEs stay implementation-agnostic. No Rust, N-API, monorepo tree,
   status matrix or build commands.

## Verified commands

```bash
pnpm install          # install workspace dependencies
pnpm typecheck        # type-check every package
pnpm test             # package tests (needs a built or staged addon)
pnpm build:napi       # build the native addon for the host
pnpm stage:native     # place the addon in its platform package
pnpm build:rust       # cargo build --release
pnpm test:rust        # cargo test --release
pnpm check:rust       # fmt check, clippy -D warnings, model/napi tests
```

The TypeScript smoke test skips addon cases when the native module is missing, so
a green `pnpm test` without `pnpm build:napi` proves less than it appears to.

## Change checklist

- [ ] Model behaviour changed in `cad-model` first; no duplicate logic above it.
- [ ] Binding changes are value mapping only.
- [ ] SDK and CLI both reached through the same document/service path.
- [ ] `capabilities()` updated for any new or removed support.
- [ ] Import/export loss codes added with severity and a precise detail string.
- [ ] Every new CLI flag checked against `cli.ts`; no README-only flags.
- [ ] Rust tests for model changes; smoke test for SDK-visible changes.
- [ ] Documentation updated in its owning file (table above).
- [ ] `git diff --check` passes.
- [ ] Stage explicit paths only. Never `git add -A`, never commit unrelated WIP.

## Commit style

One logical change per commit, imperative summary, emoji prefix:

| Prefix | Use |
| :--- | :--- |
| `✨` | New behaviour or capability |
| `🐛` | Bug fix |
| `📝` | Documentation only |

Example: `📝 Move maintainer documentation into documentation/ and add AGENTS.md`
