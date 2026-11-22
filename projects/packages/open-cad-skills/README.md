# @wae/open-cad-skills

Agent skill documents for OpenCAD. The package ships
`skills/open-cad/SKILL.md` and no runtime code.

Use it when an agent should build, edit, audit or export a drawing through the
`open-cad` command line. The skill fixes the order of operations and names the
checks to run before export, so the agent follows a verified workflow instead of
guessing one.

## Install

```bash
pnpm add @wae/open-cad-skills
```

## Use

Point the agent at
`node_modules/@wae/open-cad-skills/skills/open-cad/SKILL.md`, then ask:

> Use OpenCAD to create `plan.ocad` in millimetres with a 6,000 × 4,000 room on a
> `Walls` layer, add an overall dimension on the `dimensions` layer, inspect it,
> and export `plan.svg`.

The skill must never claim a method, command or flag that the installed SDK and
CLI do not expose. Support is defined by
[Capabilities](../../../documentation/capabilities.md); the product overview is
the [root README](../../../README.md).
