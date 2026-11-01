# Scripting Grid4D

A script is a short program that reads and changes the open document:
builds rows of objects, renames, recolours, arranges, runs plugin commands.
Scripts are written in [Rhai](https://rhai.rs), a small language that reads
much like Rust or JavaScript, and saved as `.rhai` files. This guide covers
running scripts, the language as far as scripts need it, and the whole
document API. The behaviour is specified in
[`reference/extend-scripting.md`](reference/extend-scripting.md); the
user-level introduction is the manual's
[Scripts chapter](manual.md#22-scripts).

Every block of Rhai in this guide is run by Grid4D's tests on its demo scene
(the Cube, the Sphere, the Group with a Cylinder and a Box, a Light and a
Camera; the Sphere and then the Cube selected), so the examples work as
printed.

## Running scripts

- **Script ▸ a script** runs one from your **script library**, the folder
  `scripts` in Grid4D's data folder (on Windows
  `%APPDATA%\grid4d\data\scripts`). Grid4D fills it with examples the first
  time it starts; any `.rhai` file you put there is listed.
- **Script ▸ Script Manager…** opens the editor: New, Open (from the
  library), Save (into the library, under the name in the Name field), Run
  or **Ctrl+Enter**; a console shows what scripts print and their errors.
- **Script ▸ Run Script File…** runs any `.rhai` file.

How a run works:

- A script runs **once**, from top to bottom, on a copy of the document.
  When it finishes, the copy becomes the document and everything it
  changed is **one undo step**, "Run Script" (recorded in the active take
  like any edit).
- If it stops with an **error**, nothing is changed — not even what it did
  before the error — and the message says where: `Line 3, column 9: …`.
- A script is stopped after **100 million operations** (an endless loop),
  changing nothing. It also cannot nest calls deeper than 64 or build
  strings, arrays and maps beyond about a million items.
- A script **cannot** read or write files, reach the network, run other
  programs or use `eval`; it sees the document and nothing else.
- `print(…)` writes a line to the console (or, with the Script Manager
  closed, to a message shown when the script ends).

## Rhai in a page

```rhai
let count = 3;                        // a whole number
let size = 50.0;                      // a number with a fraction
let name = "Box";                     // text
let place = [0, 100, 0];              // an array
let options = #{ Count: 10, Seed: 4 }; // a map (keys with spaces in quotes)
let nothing = ();                     // "no value"

for i in 0..count {                   // 0, 1, 2
    print(`box ${i} of ${count}`);    // backticks: text with ${…} inside
}

if size > 20.0 && name != "" {
    print("big");
} else {
    print("small");
}

let label = if count == 1 { "object" } else { "objects" };   // if gives a value

fn double(x) {                         // a function; it sees only its arguments
    x * 2
}
print(double(21));

place[1] += 50;                        // arrays index from 0
print(place.len());
```

Things worth knowing:

- **Whole numbers and fractions:** `7 / 2` is `3`, `7 / 2.0` is `3.5`.
  Mixing them in arithmetic gives a fraction.
- **Set properties on a variable.** `find("Cube").name = "Box"` is a syntax
  error; write `let cube = find("Cube"); cube.name = "Box";`. Changing an
  array's element's property works: `objects[0].position = [0, 0, 0];`.
- **Functions do not see the script's variables**; pass what they need.
- **`throw "message"`** stops the script with that message (and changes
  nothing).
- `()` means "no value": `find` gives it when there is no such object; test
  with `if cube == () { … }`.

## Values

Numbers are **as the Attribute Manager shows them**:

| Kind of value | In scripts |
|---------------|------------|
| lengths | centimetres (whatever length unit the Preferences show) |
| angles | degrees |
| fractions | percent, 0 – 100 |
| position, rotation (H, P, B), scale | arrays of three numbers, `[x, y, z]` |
| colours | arrays of three numbers from 0 to 1, `[r, g, b]` (linear) |

A number may be written as a whole number or with a fraction.

## The document

| Function | Gives / does |
|----------|--------------|
| `objects()` | an array of every object, parents before their children, in Object Manager order |
| `roots()` | the top-level objects |
| `find(name)` | the first object (in that order) with exactly this name, or `()` |
| `selection()` | the selected objects, in the order they were selected (the last one is the active object) |
| `select(object)`, `select([objects])` | the selection to have when the script ends (the last one active) |
| `frame()` | the current frame, a whole number |
| `create(kind)`, `create(kind, parent)` | a new object, last at the top level or last under `parent`; `kind` is a name from the Create menu (`"Cube"`, `"Null"`, `"Bend"`, `"Cloner"`, `"Circle"`, …, case does not matter) or a plugin's (`"Gear"`, or its id `"grid4d.gear"`); named as the menu names it (`"Cube.1"` when there is a Cube), with the menu's defaults |
| `delete(object)` | deletes it with everything under it |
| `run_command(name)`, `run_command(name, options)` | runs a plugin command (`"Scatter"`, or its id) on the script's selection; `options` is a map of option names to values (the rest at their defaults); gives an array of the new top-level objects; if the command cannot run, its message stops the script |
| `print(value)` | a line of output |

```rhai
// Every top-level object, with its kind.
for object in roots() {
    print(`${object.name}: ${object.kind}`);
}
let cube = find("Cube");
if cube != () {
    print(`the cube is at ${cube.position}`);
}
print(`frame ${frame()}, ${selection().len()} selected`);
```

## Objects

An object is a value of type `Object`. Using an object after `delete` is an
error.

| Property / method | |
|-------------------|-|
| `name` | its name; set it with `=` |
| `kind` | its kind's name (`"Cube"`, `"Gear"`, …); read only |
| `position`, `rotation`, `scale` | in its parent's space; rotation as H, P, B in degrees |
| `color` | its display colour |
| `visible`, `visible_in_render`, `enabled` | its switches, `true` or `false` |
| `parent` | the object it is under, or `()`; setting it moves the object there (last), keeping its place in the world; `()` moves it to the top |
| `children` | the objects directly under it |
| `material` | the name of its material, or `""`; set it to a material's name, or `""` for none |
| `layer` | the name of its layer, or `""`; set it to a layer's name, or `""` for none |
| `get(name)` | a parameter's value (below) |
| `set(name, value)` | changes a parameter |

Objects can be compared with `==` and `!=`, and printed (`Cube (Cube)`).

```rhai
let sphere = find("Sphere");
sphere.position = [0, 150, 0];
sphere.rotation = [45, 0, 0];
sphere.scale = [1, 1.5, 1];
sphere.color = [0.9, 0.3, 0.1];
sphere.visible_in_render = false;
let group = find("Group");
sphere.parent = group;          // stays where it was in the world
print(group.children.len());    // 3
```

### Parameters

`get` and `set` reach a parameter **by its name on the Object tab** (case
does not matter):

- for Grid4D's own objects, the numbers that can be keyed: `"Radius"`,
  `"Height"`, `"Size . X"`, `"Size . Y"`, `"Size . Z"`, `"Strength"`,
  `"Angle"`, … — what an object has depends on its kind and options (a
  Circle has `"Radius Y"` only with its Ellipse option on); a parameter an
  object lacks is an error;
- for a plugin object, every parameter: numbers, `true` / `false`, a
  choice's index or its English name, `[x, y, z]` for vectors and colours.

A value that has keys changes under them (as without Autokeying): the keys
stay.

```rhai
let sphere = find("Sphere");
print(sphere.get("Radius"));
sphere.set("Radius", sphere.get("Radius") * 2);

let gear = create("Gear");
gear.set("Teeth", 24);
gear.set("Tooth Width", 30);     // percent
gear.position = [400, 0, 0];
```

## Examples

The examples Grid4D puts in a new script library.

### Grid of Cubes

```rhai
// A 5 × 5 grid of cubes, coloured from corner to corner.
let group = create("Null");
group.name = "Grid";
for x in 0..5 {
    for z in 0..5 {
        let cube = create("Cube", group);
        cube.name = `Cube ${x}-${z}`;
        cube.position = [(x - 2) * 60, 0, (z - 2) * 60];
        cube.set("Size . X", 40);
        cube.set("Size . Y", 20 + 15 * (x + z));
        cube.set("Size . Z", 40);
        cube.color = [x / 4.0, 0.4, z / 4.0];
    }
}
select(group);
print(`made ${group.children.len()} cubes`);
```

### Number Selected Objects

```rhai
// Numbers the selected objects in the order they were selected:
// "Cube" becomes "Cube 1", "Sphere" becomes "Sphere 2", …
let number = 1;
for object in selection() {
    object.name = `${object.name} ${number}`;
    number += 1;
}
print(`objects numbered: ${number - 1}`);
```

### Random Colours

```rhai
// Gives each selected object (or every object, with none selected) a
// display colour of its own. Change the seed for other colours.
let seed = 2026;

// A small random number generator: the next number after `s`.
fn next(s) {
    (s * 1103515245 + 12345) % 2147483648
}

let objects = if selection().is_empty() { objects() } else { selection() };
for object in objects {
    let colour = [];
    for channel in 0..3 {
        seed = next(seed);
        colour.push(0.2 + 0.8 * seed / 2147483648.0);
    }
    object.color = colour;
}
print(`objects coloured: ${objects.len()}`);
```

### Scatter on the Last Selected

```rhai
// Spreads 30 copies of the selected objects over the surface of the one
// selected last, with the Scatter plugin command, then selects them.
let made = run_command("Scatter", #{
    Count: 30,
    Seed: 7,
    "Scale Variation": 30,
});
select(made);
print("scattered 30 copies");
```

### Spread Selected Objects

```rhai
// Puts the selected objects in a row along X, 150 cm apart, centred on
// the first one's place, in the order they were selected.
let objects = selection();
if objects.is_empty() {
    throw "select the objects to spread first";
}
let spacing = 150.0;
let start = objects[0].position;
let first = start[0] - spacing * (objects.len() - 1) / 2.0;
for i in 0..objects.len() {
    objects[i].position = [first + spacing * i, start[1], start[2]];
}
print(`objects spread: ${objects.len()}`);
```

### More ideas

A staircase:

```rhai
let stairs = create("Null");
stairs.name = "Stairs";
for step in 0..12 {
    let tread = create("Cube", stairs);
    tread.name = `Step ${step + 1}`;
    tread.set("Size . X", 120);
    tread.set("Size . Y", 20);
    tread.set("Size . Z", 30);
    tread.position = [0, 10 + step * 20, step * 30];
}
select(stairs);
```

Rename every sphere:

```rhai
let count = 0;
for object in objects() {
    if object.kind == "Sphere" {
        count += 1;
        object.name = `Ball ${count}`;
    }
}
print(`spheres renamed: ${count}`);
```

Hide in the views every top-level object that is not selected:

```rhai
let chosen = selection();
for object in roots() {
    object.visible = chosen.contains(object);
}
```

Put the selected objects in a new group, keeping their places:

```rhai
let group = create("Null");
group.name = "Selected";
group.position = [0, 50, 0];
for object in selection() {
    object.parent = group;
}
select(group);
```

A ring of gears, each turned a little more:

```rhai
let ring = create("Null");
ring.name = "Gears";
let count = 8;
for i in 0..count {
    let gear = create("Gear", ring);
    let angle = 360.0 / count * i;
    gear.set("Teeth", 10 + i * 2);
    gear.rotation = [angle, 90, 0];
    gear.position = [0, 0, 0];
}
print(`gears: ${ring.children.len()}`);
```

## Errors

An error stops the script and changes nothing. The message names the line
and column (counted from 1) and says what went wrong, for example:

```text
Line 3, column 9: there is no kind of object called "Teapot"
Line 2, column 1: Syntax error: Expression cannot be assigned to
Line 5, column 5: the object no longer exists
Line 1, column 1: Scatter: Select the objects to scatter, then the surface last
the script was stopped after 100000000 operations (an endless loop?); nothing was changed
```

Grid4D's own messages:

| Message | Why |
|---------|-----|
| there is no kind of object called "…" | `create` with a name that is not in the Create menu or a plugin |
| the object no longer exists | using an object after `delete` |
| … has no parameter "…" / … has no number parameter "…" | `get` or `set` with a name the object does not have |
| "…" takes a different kind of value | a plugin parameter set to the wrong kind of value |
| there is no material / layer called "…" | `material` or `layer` set to a name the document does not have |
| an object cannot go under itself | `parent` set to the object or one of its children |
| expected an array of three numbers, like [0, 100, 0] | `position`, `rotation`, `scale` or `color` set to something else |
| there is no plugin command called "…" / … has no option "…" | `run_command` with a wrong name or option |

## What scripts cannot do (yet)

Set keys or animate; create materials, layers or tags; read deformed shapes
or meshes' points; run every frame (expressions); ask for input. The
[implementation plan](IMPLEMENTATION_PLAN.md) lists what is planned.
