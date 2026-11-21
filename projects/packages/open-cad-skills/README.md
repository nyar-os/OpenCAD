# @wae/open-cad-skills

Agent skills for OpenCAD. The package ships skill documents only; it is not a
runtime library and contains no DWG, DXF, STEP, IGES or mesh code.

## Status

Executable document routes. `skills/open-cad/SKILL.md` lists the commands that
work today: create/open `.ocad` drawings, add, move, update and delete entities,
transform, copy, array, explode and measure, undo/redo, add and edit layers,
inspect the document, import the declared ASCII DXF subset, and export SVG, DXF,
PDF, STL or STEP. The skill states the limits of the build: no viewport
renderer, no DWG/STEP importer, no full DXF reader, no STL reader and no
constraint solver.

## Layout

```text
packages/open-cad-skills/
├── package.json
├── README.md
└── skills/
    └── open-cad/
        └── SKILL.md
```

The skill must never be shipped claiming methods, commands or flags that the
installed SDK/CLI does not expose.
