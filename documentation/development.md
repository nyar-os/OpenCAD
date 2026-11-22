# Development

## Environment

- Node 22 is what the TypeScript CI job uses; publishing uses Node 24.
- pnpm `10.28.0`, pinned by the root `packageManager` field.
- Rust nightly, pinned by `rust-toolchain.toml`. The workspace declares
  `rust-version = "1.90"` and edition 2024.
- `cargo` needs `rustfmt` and `clippy`; native builds need a C toolchain.

```bash
pnpm install
```

## Build

| Command | What it does |
| :--- | :--- |
| `pnpm build:rust` | `cargo build --release` |
| `pnpm build:napi` | `wae native build` for the host target |
| `pnpm stage:native` | `wae native stage` — copies the built addon into its platform package |
| `pnpm build:ts` | Compiles `@wae/open-cad` to `dist/` |
| `pnpm build:ui` | Builds the studio shell |
| `pnpm build` | `wae build` — the full pipeline |
| `pnpm gui` | Builds everything, then runs `open-cad gui` from `dist/` |

The studio shell (`projects/packages/open-cad-ui`) builds with `vmz`:
`vmz check .`, `vmz build . --profile static` and `vmz serve . --profile static`.
The static output is copied to `projects/packages/open-cad/dist/gui` for the
native WebView host. It renders UI only; the canvas reports that the interactive
viewport renderer is not implemented.

## Native addon staging

The SDK loads the addon from one of two places, in order:

1. `OPEN_CAD_NATIVE` — an absolute path to a `.node` file. Use this for a local
   debug build or a CI artifact.
2. `@wae/open-cad-<platform>-<arch>` — the platform package declared in
   `optionalDependencies` and resolved through `createRequire`.

A missing addon is not a crash: the SDK reports `capability/not-implemented`.
For local work, `pnpm build:napi && pnpm stage:native` puts the addon where the
platform package expects it. CI builds with
`wae native build --target <target>` and copies
`open-cad.<target>.node` into `projects/packages/open-cad-<target>/`.

## Test

| Command | What it does |
| :--- | :--- |
| `pnpm test` | Every package's `test` script; `@wae/open-cad` compiles `src` and `tests` then runs `node --test dist-test/tests/smoke.test.js` |
| `pnpm test:rust` | `cargo test --release` |
| `pnpm typecheck` | Type-checks each package without emitting |
| `pnpm check:rust` | `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test` for `open-cad-drawing` and `open-cad-napi` |
| `pnpm fmt` | `cargo fmt --all` |

The TypeScript smoke test covers the addon end to end. It skips its addon cases
when the native module cannot be loaded, so a green run without a built addon is
weaker than it looks. Run `pnpm build:napi` first for real coverage. Rust tests
cover the drawing model, history, import and exchange directly.

## CI

| Workflow | Trigger | Steps |
| :--- | :--- | :--- |
| `.github/workflows/check-rust.yml` | push/PR to `master`, `dev` | Rust nightly on Ubuntu, macOS and Windows; `cargo build --workspace --release`; `cargo test --workspace --release` |
| `.github/workflows/check-typescript.yml` | push/PR to `master`, `dev` | Node 22, `pnpm install --no-frozen-lockfile`, `pnpm typecheck`, `pnpm test` |
| `.github/workflows/publish-npm.yml` | tag `v*` or manual tag | Builds five targets, stages the artifacts, builds the SDK and studio, publishes with npm Trusted Publisher |

## Release and publish

Real versions publish only from `.github/workflows/publish-npm.yml` through npm
Trusted Publisher (OIDC). Keep that filename: it is the trusted workflow entry
for the `NPM_PUBLISH` environment on `nyar-os/OpenCAD`.

1. Tag `v<version>` and push the tag (or run the workflow with an existing tag).
2. The native job builds `win32-x64`, `darwin-x64`, `darwin-arm64`, `linux-x64`
   and `linux-arm64`.
3. The publish job copies each artifact into its platform package, runs
   `pnpm run build:ts && pnpm run build:ui`, and runs
   `nifty publish --access public --version=<version>`.

Local `nifty publish --placeholder` exists only for `0.0.0` stubs; do not use it
for a real release.
