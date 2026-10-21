# Grid4D User Manual

How to use Grid4D, feature by feature. This manual describes what the current build
does; features that are planned but not there yet are marked *planned*. The
screenshots come from the application itself.

> Grid4D follows the workflow of Cinema 4D's Standard layout, so the names of tools
> and shortcuts will feel familiar to Cinema 4D users. It is an independent project,
> not affiliated with Maxon.

**Contents**

1. [Getting started](#1-getting-started)
2. [The workspace](#2-the-workspace)
3. [Navigating the viewport](#3-navigating-the-viewport)
4. [Creating objects](#4-creating-objects)
5. [Selecting and transforming objects](#5-selecting-and-transforming-objects)
6. [The Object Manager](#6-the-object-manager)
7. [The Attribute Manager](#7-the-attribute-manager)
8. [Undo and redo](#8-undo-and-redo)
9. [Files](#9-files)
10. [Timeline and playback](#10-timeline-and-playback)
11. [Polygon modelling](#11-polygon-modelling)
12. [Generators: Subdivision Surface](#12-generators-subdivision-surface)
13. [Snapping](#13-snapping)
14. [Tags](#14-tags)
15. [Materials](#15-materials)
16. [Keyboard shortcuts](#16-keyboard-shortcuts)
17. [Limitations and what is planned](#17-limitations-and-what-is-planned)

---

## 1. Getting started

**Requirements:** a graphics card with Vulkan, Direct3D 12 or Metal support (any GPU
from the last decade). Grid4D runs on Windows, Linux and macOS.

**Running from source** (needs the Rust toolchain, 1.95 or newer):

```bash
cargo run --release
```

**Opening a document directly:** pass its path, e.g. `grid4d scene.g4d`. This is
also what "Open with" and file associations use.

Grid4D starts maximized with a small demo scene (a cube, a sphere, a group with a
cylinder and a box, a light and a camera), so there is something to try right away.
Window size, panel sizes and the list of recent files are remembered between
sessions.

## 2. The workspace

![The Grid4D workspace](images/manual/workspace.png)

| # | Area | What it is for |
|---|------|----------------|
| 1 | **Menu bar** | Every command: File, Edit, Create, Modes, Select, Tools, Mesh, Animate, Window, Help. |
| 2 | **Command palette** | Undo/redo, the selection tools, Move/Scale/Rotate, axis locks, World/Object coordinates, render buttons (*planned*) and buttons that create objects. |
| 3 | **Mode palette** | Make Editable, then the four modes: Model, Points, Edges, Polygons; the snapping toggle ([§13](#13-snapping)). |
| 4 | **Viewport** | The 3D view of the scene, or up to four views ([§3.2](#32-several-views-top-front-and-side)). Each view's header has the View, Cameras, Display, Options, Filter and Panel menus and three navigation icons. |
| 5 | **Object Manager** | The scene's objects as a hierarchy. |
| 6 | **Attribute Manager** | The parameters of the selected object, or the options of the current tool. |
| 7 | **Timeline** | The current frame, playback and the frame range. |
| 8 | **Status bar** | A hint for whatever is under the cursor, the object count and the graphics adapter. |

The panels resize with the window. Drag the border between the viewport and the
managers to make them wider.

![The command palette](images/manual/toolbar.png)

**Hover any button or menu entry**: its tooltip and the status bar explain it.

## 3. Navigating the viewport

| Action | Mouse | Alternative |
|--------|-------|-------------|
| Orbit around the scene | **Alt + left drag** | Drag the circular-arrow icon at the top right of the viewport header |
| Pan | **Alt + middle drag** or **middle drag** | Drag the cross icon in the header |
| Zoom | **Alt + right drag** or **mouse wheel** | Drag the magnifier icon in the header |
| Frame the whole scene | **H** | View ▸ Frame Scene |
| Frame the selected objects | **O** | View ▸ Frame Selected Objects |
| Back to the default view | | View ▸ Reset View |

- **Filter** (viewport header) turns the grid, the axis triad (bottom left), the
  HUD and kinds of objects on or off ([§3.4](#34-the-filter-menu)). The HUD shows
  the grid spacing, which adapts to the zoom (1, 10, 100 cm …).
- Units are centimetres; Y points up.

### 3.1 Looking through a camera

The viewport has its own camera (the *default camera*). It can also look through a
camera object of the scene, or through any object. With several views
([§3.2](#32-several-views-top-front-and-side)), this is offered in View 1, the
perspective view at the top left.

![The Cameras menu: Default Camera, Use Camera with the scene's cameras, and Selected Object as Camera](images/manual/cameras_menu.png)

- **Cameras ▸ Use Camera ▸** *a camera*: the view looks through that camera — from
  its position, along its viewing direction, with its bank and field of view. The
  view's title shows the camera's name.
- **Double-click a camera's icon** in the Object Manager to look through it; the
  icon of the camera in use is highlighted. Double-click it again to go back.
- **Cameras ▸ Selected Object as Camera** looks along the active object's −Z axis
  (for example, to see what a light points at). The view keeps its own field of
  view.
- **Cameras ▸ Default Camera** goes back to the view's own camera, where you left
  it.

![Looking through the scene's Camera; its icon in the Object Manager is highlighted](images/manual/camera_view.png)

While the view looks through a camera, **navigating moves the camera**: orbit, pan,
zoom and H / O framing change its position and direction, like moving it with the
gizmo. Each gesture is one undo step. On an animated camera the change is
temporary until you record it, or becomes a key with Autokeying
([§10.2](#102-recording-keys)). A Protection tag on the camera's position or
rotation keeps it — and the view — where it is. An animated camera, or one with a
Target tag, is followed while the animation plays.

Every camera that the view is not looking through is drawn with a **wire pyramid**
showing what it sees (its field of view, at the view's proportions), with a small
triangle marking its top. Selected cameras are drawn in orange.

A **new camera** (Create ▸ Object ▸ Camera) is placed where the view is, looking
the same way, with the same bank and field of view.

### 3.2 Several views: top, front and side

The viewport can show **four views** at once — a perspective view and exact top,
side and front views — to model from several sides.

![Four views: Perspective, Top, Right and Front, with the Cylinder selected in all of them](images/manual/four_views.png)

| View | Shows | Where |
|------|-------|-------|
| View 1 | Perspective | top left |
| View 2 | Top | top right |
| View 3 | Right | bottom left |
| View 4 | Front | bottom right |

- **F5** (or **Panel ▸ All Views**) shows the four views; **F1**–**F4** (Panel ▸
  View 1 … View 4) show one of them alone, filling the viewport.
- **Middle-click a view** (without dragging) to show it alone, and middle-click
  again to put it back among the others.
- **Panel** also offers *Single View*, *2 Views Stacked* (views 1 and 2, one above
  the other) and *2 Views Side-by-Side*. The viewport starts as a single
  perspective view.

![The Panel menu](images/manual/panel_menu.png)

**The active view** has a thin blue frame. It is the one you last clicked or
scrolled in: just click in another view to work there — the same click already
selects, starts a drag or uses the current tool. Selection, the gizmo, the
modelling tools and snapping work in every view; H, O and View ▸ Reset View act on
the active view.

**Each view's Cameras menu** chooses what it shows: *Perspective*, or one of the
orthographic projections *Top*, *Bottom*, *Front*, *Back*, *Left* and *Right*.

![A view's Cameras menu with the projections; Top is in use](images/manual/projections_menu.png)

The orthographic views look straight along a world axis, without perspective, so
sizes and distances can be compared directly (a cube is a square):

- **Front** looks along −Z, with X to the right and Y up; **Back** looks the other
  way. **Right** looks along −X (Z to the left), **Left** the other way. **Top**
  looks down, with X to the right and Z towards the bottom of the view; **Bottom**
  looks up.
- They **cannot orbit**: Alt + left drag pans in them. Panning (middle drag) and
  zooming (wheel, Alt + right drag) work as usual; zooming changes how much of the
  scene is in view.
- The **grid lies in the view's plane**: the XY plane in Front and Back, ZY in
  Left and Right, the ground (XZ) in Top and Bottom. Its axis lines keep their
  colours (X red, Y green, Z blue). Grid snapping uses that plane too.

Every view keeps **its own camera, display mode and Filter** (objects, grid, axis
triad, HUD), and shows its projection — or the name of the camera it looks through — at the top left.
When a view is too narrow for its menus and navigation icons, the icons are left
out; the mouse navigation works as always.

### 3.3 Display modes

Each view's **Display** menu chooses how it draws objects: smooth shading to
judge the form, lines to see how a mesh is built, boxes to keep a heavy scene
quick. Every view has its own mode, so the perspective view can stay shaded
while the top view shows lines.

![The Display menu](images/manual/display_menu.png)

The upper part chooses the **shading**:

| Mode | Shortcut | What you see |
|------|----------|--------------|
| Gouraud Shading | N~A | Smooth surfaces lit by the scene's lights — or by the default light when there are none (the default mode; [§4.1](#41-lights)). |
| Gouraud Shading (Lines) | N~B | The same, with lines over the surfaces. |
| Quick Shading | N~C | Lit by the default light only, whatever lights the scene has. |
| Quick Shading (Lines) | N~D | The same, with lines. |
| Constant Shading | N~E | Flat, in each object's colour, without light and shade. |
| Constant Shading (Lines) | | The same, with lines. |
| Hidden Line | N~F | Only lines, and only those not hidden behind a surface. |
| Lines | N~G | Only lines, all of them — you see through the objects. |

The lower part chooses **what is drawn** of each object:

- **Wireframe** (N~H): the object itself; its lines are the edges of its
  polygons (an n-gon shows its outline, not the triangles it is drawn with).
- **Box** (N~K): each object as a box around it — its bounding box, turned with
  the object. Shaded in a shaded mode, its twelve edges in Hidden Line or Lines.
  Handy when a scene is heavy. Clicking still selects the object by its real
  shape.

![Gouraud Shading (Lines), Constant Shading, Hidden Line, and Lines with Box](images/manual/display_modes.png)

Over shaded surfaces the lines are a dark shade of the object's colour; in
Hidden Line and Lines they take the object's colour. **Selected objects' lines
are orange.**

The N~ shortcuts change the **active view** (press N, then the letter; the
status bar lists the choices while it waits). The menu changes the view it
belongs to.

![Four views, each with its own mode: Gouraud Shading, Lines (Top), Gouraud Shading (Lines) (Right) and Hidden Line (Front)](images/manual/display_per_view.png)

### 3.4 The Filter menu

Each view's **Filter** menu chooses what that view shows — for example, no
cameras and lights while you model, or no grid and HUD for a clean picture.
Everything is shown in a new view.

![The Filter menu](images/manual/filter_menu.png)

- **Object types**: *Polygon* (polygon objects), *Generator* (the parametric
  objects: Cube, Sphere, …), *Subdivision Surface*, *Camera* and *Light*. A hidden
  type is neither drawn nor clickable in that view: clicks, rectangle selections
  and snapping go through it. Only the objects of that type are hidden, not their
  children. Hidden lights still light the view.
- **Elements**: *World Grid* (the grid), *World Axis* (the axis triad in the lower
  left corner) and *HUD* (the grid spacing in the lower right corner).
- Click an entry to turn it off or on. **Ctrl+click** shows only that entry and
  hides all the others. **All** shows everything again, **None** hides everything.
- An object you select in the Object Manager while its type is hidden still shows
  its gizmo, so you can move, scale or rotate it; in a component mode its points,
  edges and polygons still show.

![Generator hidden in the Top view only: its primitives are gone there, the other views still show them](images/manual/filter_views.png)

## 4. Creating objects

Click a button on the right side of the command palette, or use **Create ▸ Object**:

| Object | Parameters (Attribute Manager ▸ Object) |
|--------|-------------------------------------------|
| Cube | Size X/Y/Z, Segments X/Y/Z |
| Sphere | Radius, Segments |
| Cylinder | Radius, Height, Rotation and Height segments |
| Plane | Width, Height, Width and Height segments |
| Cone | Top and Bottom radius, Height, segments |
| Torus | Ring radius, Pipe radius, Ring and Pipe segments |
| Disc | Inner and Outer radius, Disc and Rotation segments |
| Capsule | Radius, Height, Height, Cap and Rotation segments |
| Null | An empty object used to group others |
| Camera | Field of view; **created at the current view**, so it frames what you see |
| Light | Colour, Intensity, Type, cone and falloff ([§4.1](#41-lights)) |

New objects are named like Cinema 4D's (`Cube`, `Cube.1`, `Cube.2` …) and become
the selected object. These are **parametric** objects: change their parameters at
any time. To edit their points and polygons, make them editable
([§11.1](#111-make-editable)).

**Generators** (Create ▸ Generator, or the green button at the right end of the
command palette) build their shape from their children. The Subdivision Surface
smooths a polygon mesh ([§12](#12-generators-subdivision-surface)).

### 4.1 Lights

Light objects light the viewport in **Gouraud Shading** — where they are, which
way they point, their colour and strength. A scene without (visible) lights is lit
by the **default light**, which comes from over your shoulder and moves with the
view. **Quick Shading** always uses the default light ([§3.3](#33-display-modes)).

![A spot light shining down on spheres with chrome, glossy red and matte blue materials and a glowing cube; an Infinite light's arrow at the right](images/manual/lights_viewport.png)

The light's settings are on the Attribute Manager's **Object** tab:

![A Spot light's settings](images/manual/light_attributes.png)

| Setting | What it does |
|---------|--------------|
| Color, Intensity | The light's colour and strength (100 % is normal; up to 10 000 %). |
| Type | **Omni**: shines in every direction, like a bulb. **Spot**: shines in a cone. **Infinite**: shines in one direction everywhere, like the sun — only its rotation matters, not its position. |
| Use Inner, Inner Angle, Outer Angle (Spot) | The **Outer Angle** is the cone's opening (default 30°). Inside the **Inner Angle** the light is at full strength; it fades out towards the outer angle. With **Use Inner** off, the whole cone is at full strength and its edge is sharp. |
| Falloff, Radius/Decay (Omni, Spot) | **None**: as strong at any distance (the default). **Linear**: fades out to nothing at the Radius/Decay distance. **Inverse Square**: exactly the set intensity at Radius/Decay, stronger nearer and weaker farther, like real light. |

- Spot and Infinite lights shine along their **−Z axis**, like cameras look. Turn
  them with the Rotate tool or the Coordinates, or give them a **Target tag** to
  point them at an object ([§14.3](#143-target)). *Cameras ▸ Selected Object as
  Camera* shows what a light points at.
- In the viewport a **Spot** shows its cone and an **Infinite** light an arrow
  along its direction (orange when selected).
- A light hidden in the editor (the upper dot in the Object Manager) does not
  light the viewport.
- Sides that no light reaches keep a little soft light, so shapes stay readable.

**Materials** show in the lit viewport too ([§15](#15-materials)): roughness sets
how sharp the highlights are, metals reflect in their own colour, and luminance
makes a surface glow whatever the light.

## 5. Selecting and transforming objects

**Selecting** (in Model mode):

- Click an object in the viewport or in the Object Manager.
- **Shift + click** adds to the selection, **Ctrl + click** toggles.
- Click empty space to deselect; **Ctrl+A** selects everything, **Ctrl+Shift+A**
  nothing.
- With the **Rectangle Selection** tool (**0**), drag a frame: objects whose origin
  is inside are selected.

The last object you selected is the **active** one: the gizmo and the Attribute
Manager work on it. Selected objects get orange corner brackets.

**Transforming:** pick **Move** (**E**), **Scale** (**T**) or **Rotate** (**R**),
then drag a handle of the gizmo.

![Move, Scale and Rotate gizmos](images/manual/gizmos.png)

- **Move**: drag an arrow to move along that axis. **Scale**: drag a handle with a
  square end. **Rotate**: drag a ring.
- The **X / Y / Z** buttons in the command palette lock axes. A disabled axis
  ignores the drag.
- The **World/Object** button next to them switches the gizmo between the world's
  axes and the object's own axes. Scale always uses the object's axes.
- **Alt + drag** always navigates, even over the gizmo.
- For exact values, type them in the Attribute Manager ([§7](#7-the-attribute-manager)).
- To place objects exactly on other objects or on the grid, turn on snapping
  (**Shift+S**, [§13](#13-snapping)).

Every drag is one undo step.

## 6. The Object Manager

![The Object Manager](images/manual/object-manager.png)

- **Hierarchy**: click the fold arrow to open or close a group.
- **Rename**: double-click the name, type, then press Enter (Esc cancels). Or
  right-click ▸ Rename.
- **Visibility dots** (two small dots on the right): the upper one hides the object
  in the viewport, the lower one hides it in renders. A hidden parent hides its
  children.
- **Enable check mark**: turns an object off entirely (it shows a red cross).
- **Re-parent**: drag a row onto another row. The object keeps its position in the
  world. Right-click ▸ *Move to Top Level* takes it out of its group.
- **Search**: type in the search field to show only matching objects.
- **Tags** appear as icons after the name; click one to edit it ([§14](#14-tags)).
- Right-click a row for Rename, Frame Object, Move to Top Level, Add Tag and Delete.

## 7. The Attribute Manager

![The Attribute Manager](images/manual/attribute-manager.png)

With an object selected it shows three tabs:

- **Basic**: name, editor and render visibility, enabled.
- **Coord.**: position (P), scale (S) and rotation (R, as H/P/B angles in degrees),
  plus *Reset Transform*. The values are those at the current frame; the small
  circle before each field is its key button ([§10.2](#102-recording-keys)).
- **Object**: the object's own parameters (sizes, segments, field of view …).

Drag a value left or right to change it, or click it and type a number. A change
from one drag or one typing session is one undo step.

**With nothing selected** it shows the project settings: the name, the frame rate and
the frame range.

**Tool mode:** when you pick a tool that has options (the selection tools,
Extrude, Inset, Bevel, Bridge, Loop/Path Cut, Line Cut), the Attribute Manager switches to that
tool's options. Its **Mode** menu switches between *Objects* and *Tool* by hand.

### 7.1 The Coordinate Manager

The **Coordinates** tab beside **Attributes** (**Shift+F7**; **Shift+F5** goes back
to the Attribute Manager; both are also in the Window menu) is for typing exact
positions, sizes and rotations.

![The Coordinate Manager: the Box inside the Group, in world space, with its size](images/manual/coordinates.png)

- Three columns: **Position**, the middle column, and **Rotation** (H, P, B in
  degrees).
- The list under the fields on the left chooses how values are read:
  - **Object**: in the object's parent, the same values as the Coord. tab.
  - **World**: in the world. Typing a world position puts the object there,
    wherever its parent is.
- The list in the middle chooses the middle column:
  - **Scale**: the object's scale (in World mode, including its parents').
  - **Size**: the object's extent along its own axes, in cm. Typing a size scales
    the object to it.
  - **Size+**: the same, with its children included.
- Drag a value or type one and press Enter: the object changes at once, and each
  change is one undo step. **Apply** writes all the fields as shown.
- **In Points, Edges or Polygons mode** the manager works on the selection:
  Position is the centre of the selected elements, Size the extent of their
  bounding box. Changing them moves or scales the selection around its centre.
  Rotation always shows 0: a value turns the selection by that much around its
  centre, then the field returns to 0.
- Values are those at the current frame. Changing an animated value follows the
  same rules as the gizmo: a temporary value, or a key with Autokeying
  ([§10.2](#102-recording-keys)).
- Channels locked by a Protection tag are greyed out. With several objects selected,
  the manager shows and changes the active one.

## 8. Undo and redo

- **Ctrl+Z** undoes, **Ctrl+Y** or **Ctrl+Shift+Z** redoes. The arrows at the left
  of the command palette do the same, and their tooltips name the step ("Undo Move").
- Everything that changes the document can be undone: creating, deleting and
  re-parenting objects, every drag and typed value, renames, selections of points,
  edges and polygons, and every modelling operation.
- Up to 100 steps are kept.

## 9. Files

| Command | Shortcut | Notes |
|---------|----------|-------|
| File ▸ New | Ctrl+N | An empty document. |
| File ▸ Open… | Ctrl+O | Opens `.g4d` documents (and older `.oc4d` ones), and models (`.gltf`, `.glb`, `.obj`) as new documents. |
| File ▸ Open Recent | | The last ten files. |
| File ▸ Merge… | Ctrl+Shift+O | Adds the objects of a `.g4d`, `.gltf`, `.glb` or `.obj` file to the current document. |
| File ▸ Export… | | Writes the scene as a model for other applications: glTF (`.glb`, or `.gltf` + `.bin`) or OBJ (`.obj` + `.mtl`). |
| File ▸ Save | Ctrl+S | Asks for a name the first time. |
| File ▸ Save As… | Ctrl+Shift+S | |
| File ▸ Quit | | |

- **Drag a `.g4d` file onto the window** to open it; **drag a model** (`.gltf`,
  `.glb` or `.obj`) onto it to merge it into the current document.
- The window title shows the document name and an asterisk (`*`) while there are
  unsaved changes. Undoing back to the saved state clears it again.
- Before anything would discard unsaved changes (New, Open, Quit, closing the
  window), Grid4D asks: **Save**, **Don't Save** or **Cancel**.
- Saving writes a temporary file first and then replaces the old one, so a failed
  save never leaves a broken document behind.
- `.g4d` files keep everything in the document: objects, hierarchy, parameters, the
  meshes of polygon objects, and their selected and hidden points, edges and
  polygons. Editor state (tool, mode, camera) is not saved.

![The File menu](images/manual/menu_file.png)

### 9.1 glTF models

glTF 2.0 is the common format for exchanging 3D models (`.gltf` with its `.bin`
file, or a single `.glb`). Most 3D applications can export it.

- **Open** a glTF file to start a new document from it. The document is named after
  the file; **Save** then asks where to put the `.g4d`.
- **Merge** one (File ▸ Merge… or drag and drop) to add it to the scene you are
  working on. The merged objects are added at the top of the hierarchy and selected,
  and one **Undo** removes all of them.

![A glTF scene opened in Grid4D: its hierarchy, meshes, camera and light](images/manual/gltf_open.png)

What comes in:

| glTF | Grid4D |
|------|--------|
| Node hierarchy, names, position / rotation / scale | The same hierarchy of objects. Metres become centimetres (× 100). |
| Meshes | Polygon objects, ready for modelling. Points that the file repeats at seams are merged, so the mesh is connected. Texture coordinates are kept. |
| Materials | Materials in the Material Manager (name, colour, roughness, metallic, emission), linked to the meshes by Material tags; the base colour also becomes the display colour. |
| Cameras | Camera objects (field of view). |
| Lights (`KHR_lights_punctual`) | Light objects (colour, intensity): point → Omni, spot → Spot (with its cone), directional → Infinite. |
| Empty nodes | Nulls. |

Not yet: textures, animation, skinning, morph targets and compressed
meshes (Draco). Shading comes from Grid4D's own smoothing, not from the file's
normals. Points the file has at *almost* the same place (not exactly) stay separate;
run **Optimize** (U~O) on such an object to merge them.

### 9.2 Exporting glTF

**File ▸ Export…** writes the whole scene as a glTF model that other applications
(Blender, game engines, web viewers, …) can read. Pick the file type in the dialog:

- **glTF Binary (`.glb`)**: one file with everything in it (the default).
- **glTF (`.gltf` + `.bin`)**: a text file plus a `.bin` file of the same name next
  to it. Keep the two together.

What goes out:

- Every object in its hierarchy, with names and positions (centimetres become
  metres).
- Polygon objects as meshes, split into triangles, with the smoothing you see in
  the viewport and their UVs. **Primitives** are written in their current shape.
  A **Subdivision Surface** is written as its smooth result at the viewport level,
  without its cage. A switched-off one is written as an empty object with its
  children as they are.
- Materials (name, colour, roughness, metallic, emission; one a mesh inherits from a
  parent included); objects without one get a simple material in their display
  colour. Cameras and lights.

Exporting does not change the document: there is nothing to undo, and the title's
`*` stays as it was.

![Top: the document; bottom: the exported .glb opened again. The Subdivision Surface arrives as one polygon object with the same shape](images/manual/gltf_export.png)

Opening an exported file brings back the hierarchy, names, positions, cameras and
lights. Every mesh comes back as a triangulated polygon object, since glTF stores
only triangles.

### 9.3 OBJ files

OBJ (Wavefront) is the oldest common mesh format; nearly every 3D application
reads and writes it. It stores **meshes only**, with no hierarchy, cameras or
lights. Unlike glTF, it keeps polygons with any number of corners. Open, merge
and drop `.obj` files like glTF files, and pick the `.obj` type in
**File ▸ Export…**.

- **Importing:** every object or group in the file becomes a polygon object, placed
  at the origin. Quads and n-gons stay as they are. Texture coordinates are kept,
  and the materials of the `.mtl` file named in the OBJ become materials (colour,
  emission) linked by Material tags, their colours also the display colours.
  OBJ has no units: 1 unit is read as 1 cm.
- **Exporting:** every mesh (primitives and Subdivision Surfaces included, as for
  glTF) becomes one OBJ object, with its points where they are in the scene (OBJ
  has no positions or hierarchy of its own). Polygons keep all their corners.
  Normals follow the viewport's smoothing. Materials (under their names, with colour
  and emission), or display colours, go to a `.mtl` file of the same name next to
  the `.obj`; keep the two together. Cameras, lights and Nulls are left out.

![Top: the document; bottom: its OBJ export opened again. One polygon object per mesh, in the same places, quads intact](images/manual/obj_export.png)

## 10. Timeline and playback

![The timeline](images/manual/timeline.png)

| Action | Shortcut |
|--------|----------|
| Play forwards / backwards | F8 / F6 |
| Stop | F7 |
| Previous / next frame | F / G |
| Previous / next key of the selected objects | Ctrl+F / Ctrl+G |
| Go to start / end | the outer transport buttons |
| Record Active Objects | F9 (the red button) |
| Autokeying on / off | Ctrl+F9 (the button with an A) |

- Click or drag on the ruler to scrub, or type a frame number.
- Under the ruler, the **key strip** shows the keys of the selected objects
  ([§10.3](#103-keys-on-the-timeline)).
- The two fields on the right set the frame range; the drop-down sets the frame
  rate. The loop button (also Animate ▸ Loop Playback) makes playback start over at
  the end.

### 10.1 Animation

Objects in a document can be **animated**: their position, rotation and scale can
change over time. While you play or step through the frames, the viewport shows
every animated object where it is at the current frame, and its children move with
it.

![An animated document at frames 0, 8, 15 and 30: a ball bouncing (and squashing at the bottom) and a turning bar carrying a sphere](images/manual/animation_frames.png)

- Each animated value has **keys**: a value at a frame. Between keys the value
  changes smoothly (*Spline*, the default), in a straight line (*Linear*) or
  holds and then jumps (*Step*). Smooth curves never shoot past a highest or
  lowest key. Before the first key and after the last one, the value stays put.
- The Coordinates fields and the gizmo show and change the values **at the
  current frame**.
- Animation is saved with the document.

### 10.2 Recording keys

1. Select the objects to animate and go to a frame.
2. Place them: move, scale or rotate them with the gizmo or the Coordinates fields.
3. Press **F9** (*Record Active Objects*: the red button under the timeline, or
   Animate ▸ Record Active Objects). Every selected object gets keys for its
   position, scale and rotation at this frame.
4. Go to another frame, change them again, press F9 again, and so on.

Animate ▸ Record chooses which of **Position**, **Scale** and **Rotation** F9
records (all three by default).

**Key buttons.** In the Attribute Manager's Coord. tab, the circle before each
field tells you about that value:

| Circle | Meaning |
|--------|---------|
| grey ring | not animated |
| red ring | animated, no key at this frame |
| red dot | a key at this frame |
| yellow ring / dot | changed here but not recorded (no key / a key at this frame) |

- **Click** a circle to set a key for just that value at the current frame. Click a
  red dot to remove that key.
- **Shift+Ctrl+click** removes all the value's keys; it keeps the value it shows.

**Changes that are not recorded.** If you change an animated value without
recording, the change is temporary: the circle turns yellow, and the object stays
where you put it only at this frame. Press F9 (or click the circle) to keep it;
going to another frame drops it.

![The cube moved at frame 31 but not recorded: P . X shows a yellow ring](images/manual/animation_unkeyed.png)

![After F9 at frame 31: keys at frames 0 and 31 on the timeline, red dots in the Coordinates](images/manual/animation_record.png)

**Autokeying** (**Ctrl+F9**, the button with an A, or Animate ▸ Autokeying) records
for you: while it is on, the viewport has a red frame, and every change to an
**animated** value becomes a key at the current frame. Values that are not animated
yet change without a key, so record them once with F9 first. Switch Autokeying off
when you are done, so later edits do not overwrite your animation.

![Autokeying on: the viewport is framed in red](images/manual/animation_autokey.png)

Every recording, key click and Autokeying drag is one undo step.

### 10.3 Keys on the timeline

The strip under the frame ruler shows a mark at every frame where a selected object
has keys.

- **Click** a mark to select it (orange); **Shift+click** adds or removes marks.
  **Drag across empty space** to select all the marks in that range. A click on
  empty space clears the selection.
- **Drag a mark** to move the selected keys by whole frames. Keys already at the
  frames they land on are replaced.
- **Delete** (or Backspace) right after clicking in the strip deletes the selected
  keys; so do the strip's right-click menu and Animate ▸ Delete Keys. Once you click
  elsewhere, Delete works on objects again.
- **Ctrl+click** in the strip records keys at that frame without moving the time
  (with the values shown at the current frame).
- **Ctrl+F / Ctrl+G** (Animate ▸ Go to Previous / Next Key) jump to the previous or
  next frame with a key.

### 10.4 The F-Curve editor

The **F-Curve** tab beside **Timeline** (or **Tab** with the pointer over the
timeline) shows every animated value of the selected objects as a curve: time from
left to right, the value from bottom to top. Rotations are shown in degrees. Drag
the top edge of the panel to make it taller.

![The F-Curve editor: the cube's tracks on the left, their curves on the right; the selected key (orange) shows its tangent handle, and the header its frame and value](images/manual/fcurve.png)

- **The track list** on the left names each animated value with its colour. Click
  one to show only its curve, Shift+click to add or remove more, and click the
  object's name to show them all again.
- **Select keys**: click a key; **Ctrl+click** adds or removes keys; drag a box
  around keys; click a curve between keys to select all of its keys. A click on
  empty space clears the selection.
- **Move keys**: drag a selected key. All selected keys move, in whole frames and
  freely in value. Hold **Shift** to keep the move horizontal or vertical.
- **Edit a key exactly**: with one key selected, the header shows its frame and
  value; drag or type them.
- **Shape the curve**: a selected key shows its **tangent handles**. Drag one to
  change the slope; the other side follows so the curve stays smooth. Hold
  **Shift** to move one side alone (a sharp corner).
- **Add a key**: **Ctrl+click** on a curve adds a key there, without changing the
  curve's shape.
- **Right-click** for **Interpolation** (Spline, Linear, Step), **Tangents** (Auto
  Tangents, Zero Angle for flat handles, Zero Length for a straight segment),
  **Delete Keys**, **Frame All** and **Frame Selected**.
- **Delete** after a click in the graph deletes the selected keys.
- **Navigate**: the wheel zooms around the pointer (Ctrl+wheel zooms time only,
  Shift+wheel values only); drag with the middle button (or Alt + left) to pan.
  **H** frames all curves, **S** the selected keys, while the pointer is over the
  graph. The view frames the curves by itself when you select other objects.
- Click or drag on the frame numbers at the top to set the current frame.

![Dragging the handle down: the curve now rises steadily into the key](images/manual/fcurve_tangent.png)

Every drag, typed value and menu command is one undo step.

## 11. Polygon modelling

Modelling works on **polygon objects**: meshes of points, edges and polygons that
you shape directly.

### 11.1 Make Editable

Select a primitive and press **C** (or click the top button of the mode palette, or
Mesh ▸ Conversion ▸ Make Editable). It becomes a polygon object that looks exactly
the same: same name, position and place in the hierarchy. Its icon changes to a
triangle. One undo step turns it back into the parametric object. On a Subdivision
Surface, Make Editable keeps the smooth result
([§12.4](#124-switching-it-off-and-baking-it)).

### 11.2 Component modes

The mode palette switches between **Model** (whole objects) and the three component
modes: **Points**, **Edges** and **Polygons** (Modes menu too). In a component
mode the **active polygon object** shows its components: blue points or edges, and
selected ones in orange. A click selects what is under the cursor. Components
hidden by surfaces in front are not picked.

| Click | Effect |
|-------|--------|
| Click | Select that point / edge / polygon |
| Shift + click | Add it |
| Ctrl + click | Remove it |
| Click on empty space | Deselect all |

Each mode keeps its own selection. **Convert a selection** to another mode: Shift +
click that mode's button (tolerant, everything touched), or Ctrl + click it (strict:
only what is fully selected).

### 11.3 Selection tools and commands

![Points selected; an edge loop; an edge ring; a grown polygon selection](images/manual/components.png)

| Tool | Shortcut | How |
|------|----------|-----|
| Live Selection | 9 | Paint over components. **Middle drag** changes the brush size. Esc cancels the stroke. |
| Rectangle Selection | 0 | Drag a frame. Esc cancels. |
| Loop Selection | U~L | The loop under the cursor is highlighted; click to select it. |
| Ring Selection | U~B | The same for rings (the parallel edges across a strip). |

Options (Attribute Manager, Tool mode):

- **Only Select Visible Elements**: ignore what lies behind the surface.
- **Tolerant Selection**: anything the brush or frame touches counts.
- **Stop at Selections / Non-Quads / Poles**: where loops and rings end.

**Double-click** with the Move tool: on an edge it selects its loop, on a polygon
all connected polygons.

![The Select menu](images/manual/menu-select.png)

The **Select** menu also has:

- **Invert** (U~I), **Select Connected** (U~W), **Grow** (U~Y) and **Shrink** (U~K)
  Selection, and *Grow Using Edges Only*.
- **Hide Selected / Hide Unselected / Unhide All**. Hidden polygons disappear from
  the viewport and cannot be picked or changed until you unhide them; use this to
  work on one part of a mesh.

![Before and after Hide Selected](images/manual/hide.png)

### 11.4 Transforming components

In a component mode the Move, Scale and Rotate gizmo appears at the centre of the
selection. Drag it to move, scale or rotate the selected points, edges or polygons
together. Each drag is one undo step, and the selection stays as it was.

**Tweak Mode** (option of the Move tool): with nothing selected, drag any point,
edge or polygon to move just that one, without selecting it.

### 11.5 Extrude and Inset

![Extrude options, and a box after extrude, inset and extrude again](images/manual/extrude.png)

- **Extrude** (**D**) pushes the selected polygons out of the surface and builds
  walls around them. In Edges mode it grows new polygons from selected **border**
  edges.
- **Inset** (**I**) shrinks the selected polygons inside their outline and fills the
  gap with a ring of polygons (Polygons mode).

**Using them:**

1. Select what to work on. With nothing selected, the tool works on everything
   visible.
2. Press D or I (or pick it from the Mesh menu).
3. **Drag left or right anywhere in the viewport.** Right is outwards (Extrude) or
   inwards (Inset). The model follows live; release to finish, Esc to cancel.

Or set the options and click **Apply** in the Attribute Manager. Every drag or Apply
is a new operation, so dragging again extrudes again. The moved polygons stay
selected.

| Option | Meaning |
|--------|---------|
| Offset | Distance (negative goes the other way). A drag writes it here. |
| Subdivision | Extra rows of polygons along the walls (Extrude) or rings (Inset). |
| Create N-gons | One polygon per wall or ring side instead of rows. |
| Create Caps | (Extrude) keeps a copy at the start, so the result encloses a volume. |
| Preserve Groups | Neighbouring polygons move as one piece. Off: each polygon on its own. |
| Maximum Angle | With Preserve Groups: neighbours bent more than this are split. |
| Rotation (Edges) | (Extrude) tilts the new edge polygons around their edge. |

Typical box modelling: select the top polygon, **I** (inset), **D** (extrude),
repeat.

### 11.6 Loop/Path Cut and Line Cut

![Loop/Path Cut options and a loop cut around a cube](images/manual/loop-cut.png)

**Loop/Path Cut** (**K~L**) adds an edge loop all the way round:

1. Pick a component mode and press K, then L.
2. Hover an edge: an orange preview shows the loop that would be cut, at the
   cursor's position along the edge.
3. Click to cut.

| Option | Meaning |
|--------|---------|
| Offset | Where the cut crosses the edge, in percent. The click sets it. |
| Number of Cuts | Several loops, evenly spaced. |
| Select Cuts | The new edges become the edge selection. |

The loop runs through quads and stops at borders and at polygons that are not quads.

**Line Cut** (**K~K**), the knife: drag a line across the object. When you release,
every polygon it crosses is cut. Esc cancels.

![Line Cut options](images/manual/line-cut-options.png)

| Option | Meaning |
|--------|---------|
| Visible Only | Cut only what you can see, not the back of the object. |
| Infinite Cut | The cut continues beyond the line's ends. |
| Connect Cut Edges | Split the polygons. Off: only add points on the cut edges. |
| Select Cuts | The new edges become the edge selection. |

### 11.7 Connect, Melt, Dissolve and Delete

These commands work on the selection of the current mode (Mesh menu, or the
shortcuts). Each is one undo step.

![The Mesh menu](images/manual/menu-mesh.png)

| Command | Shortcut | Points | Edges | Polygons |
|---------|----------|--------|-------|----------|
| **Connect Points/Edges** | M~M | Joins selected points that share a polygon with new edges | Puts a point in the middle of each selected edge and joins them (on an edge ring: a loop through the middle) | — |
| **Melt** | U~Z, Alt+Backspace | Removes the points, merging their polygons | Merges the two polygons of each edge (points stay) | Merges each connected group into one polygon |
| **Dissolve** | M~N, Ctrl+Backspace | As Melt | As Melt, and removes the points left over in the middle of a merged side | As Melt |
| **Delete** | Delete, Backspace | Removes the points and the polygons using them | Removes the polygons using the edges | Removes the polygons |

![Deleting a corner point removes it and its three polygons](images/manual/delete-point.png)

- A merged region must have a single outline: a ring of polygons around a hole
  cannot become one polygon, so Melt leaves it alone.
- Points that no polygon uses any more are removed automatically.
- In Model mode, Delete removes the selected objects instead.

### 11.8 Bevel, Bridge, Weld and Optimize

![Results: a cube with every edge bevelled and rounded, a hole bridged through a cube, a cube with one edge welded into a wedge](images/manual/bevel-bridge-weld.png)

**Bevel** (**M~S**) rounds or chamfers the selected **edges** (Edges mode), or cuts
off the selected **points** (Points mode):

1. Select the edges or points to bevel.
2. Press M, then S (or Mesh ▸ Bevel).
3. Drag left or right in the viewport (Esc cancels), or click **Apply** in the
   Attribute Manager.

![Bevel options](images/manual/bevel.png)

| Option | Meaning |
|--------|---------|
| Offset | Width of the bevel. A drag writes it here. |
| Subdivision | 0 gives a flat chamfer; more gives rows across the bevel, along a rounded profile. |
| Depth | How round the profile is: 100 % round, 0 % flat, negative hollow. |
| Limit | Keeps the new points from passing each other (no overlapping polygons). |

- Where three bevelled edges meet (a cube's corner) a small polygon closes the
  corner.
- Border edges (edges with a single polygon) are not bevelled.
- The long edges of the new strips become the selection, so another bevel can round
  them further.

**Bridge** (**B**, or M~B) joins two components with new polygons:

- **Polygons mode**: drag from one polygon to another. A band of polygons joins
  their outlines, and the two polygons are removed, so the bridge opens through them
  (a hole through an object, a handle between two parts). They need the same number
  of points.
- **Edges mode**: drag from one border edge to another. One polygon fills the gap.
- Or select the two polygons or edges and click **Apply**.
- Options: **Subdivision** (rows along the bridge), **Spin Steps** (turns the band
  if it comes out twisted), **Delete Original Polygons**.

**Weld** (Mesh ▸ Weld) merges points:

1. Select the points (in Edges or Polygons mode, the points of the selection count).
2. With the Weld tool, **click one of the selected points** to weld everything onto
   it, or **click anywhere else** to weld at their centre.

Polygons that collapse are removed.

**Optimize** (**U~O**, Mesh ▸ Optimize…) cleans up a mesh:

![The Optimize dialog](images/manual/optimize.png)

- **Points** merges points closer than **Tolerance**. Use it after modelling steps
  that leave points on top of each other, or to close an editable cylinder's
  separate caps.
- **Unused Points** removes points that no polygon uses.
- It works on the selected points (or those of the selected edges or polygons), or
  on the whole object when nothing is selected.

## 12. Generators: Subdivision Surface

A **generator** is an object that builds its shape from its children. The first
one is the **Subdivision Surface**. It turns a coarse polygon mesh, the **cage**,
into a smooth, rounded surface. The result updates live while you keep modelling
the few points of the cage.

![A cube smoothed by a Subdivision Surface, with its parameters in the Attribute Manager](images/manual/sds_object.png)

### 12.1 Creating one

- Select the object to smooth, then **Alt + click** the Subdivision Surface button
  (the green button at the right end of the command palette), or hold Alt while you
  choose **Create ▸ Generator ▸ Subdivision Surface**. The generator takes the
  object's place in the hierarchy and the object becomes its child. The object
  stays where it was. This is one undo step.
- Without Alt you get an empty generator at the top of the Object Manager. To give
  it a cage, drag an object onto it in the Object Manager.

![Create ▸ Generator](images/manual/menu_generator.png)

What gets smoothed:

- The generator smooths its **first child**, the cage. The cage can be a polygon
  object or a primitive (a Cube works without Make Editable).
- If the cage has children of its own, they are merged with it and smoothed as one
  mesh.
- Further children of the generator are drawn as they are.
- Pieces of the cage that are not joined are smoothed separately, for example the
  caps of an editable cylinder. Run **Optimize** (U~O) on the cage first to merge
  them.

### 12.2 Working with the cage

- In Model mode, clicking the smooth surface selects the **generator**: move it, or
  change its parameters.
- To reshape the result, select the **cage** in the Object Manager and switch to
  Points, Edges or Polygons mode. The cage's points and edges are drawn over the
  smooth surface, every modelling tool works on them, and the surface follows each
  change at once.

![Editing the cage in Points mode: the cage's points and edges around the smooth surface](images/manual/sds_cage.png)

### 12.3 Parameters

Select the generator and open **Attribute Manager ▸ Object**:

| Parameter | Meaning |
|-----------|---------|
| Type | **Catmull-Clark (N-Gons)**: every polygon with *n* corners becomes *n* quads, then the surface is smoothed. It is the only type for now. |
| Subdivision Viewport | How many times the viewport subdivides (0 shows the cage itself, 6 at most). Each level multiplies the polygons by about four, so keep it low (2) while modelling. |
| Subdivision Renderer | The level for final renders. It will be used by the offline renderer (*planned*). |
| Linear Pre-Subdivision | Splits the cage this many times *without* smoothing before the smoothing levels, which keeps the result closer to the cage. |
| Boundary Interpolation | How open borders (the edge of a plane) are smoothed. **Edge and Corner** keeps a border's corners in place; **Edge** rounds them off as well. |

### 12.4 Switching it off and baking it

- **Q** switches the generator off and on, and so does its check mark in the Object
  Manager. While it is off, the cage shows as a normal object. If the cage (or
  anything else inside the generator) is selected, Q switches the generator above
  it.

  ![The generator switched off with Q: the cage shows as a plain cube, and the Object Manager marks the generator with a red cross](images/manual/sds_disabled.png)

- **Make Editable** (C) on the generator replaces it with one polygon object holding
  the smooth mesh, at the **Subdivision Viewport** level. The cage is removed. One
  undo step brings both back. A generator that is switched off is left as it is.

## 13. Snapping

With snapping on, a moved object or selection jumps onto nearby geometry or onto
the floor grid, so you can place things exactly: an object's axis level with a
corner, a point onto another point, a selection onto a grid crossing.

**Switching it on:** press **Shift+S**, or click the magnet button at the bottom of
the mode palette (it stays highlighted while snapping is on).

**Using it:**

1. Drag a **Move** handle (an arrow of the gizmo), or with **Tweak Mode** drag a
   point, edge or polygon directly ([§11.4](#114-transforming-components)).
2. Move the **cursor** over what you want to snap to: a corner, an edge, the middle
   of an edge, the centre of a polygon, an object's axis or a grid crossing. The
   target is highlighted with its name next to it.
3. Release the mouse button. The move is one undo step, as always.

![Dragging the cube's X arrow with the cursor over the middle of an edge: the cube lines up with it along X](images/manual/snap_drag.png)

How the element is placed:

- **Along a gizmo arrow** the move stays on that axis. The object (or the centre of
  the selection) lines up with the target along the axis.
- **With Tweak** the move is free: the grabbed element lands right on the target.
- Nothing that moves with the drag is a target: the moved object and its children,
  or the moved points and the edges and polygons that use them. So you can put a
  point onto another point of the same object.
- Scale and Rotate do not snap.

**Settings:** right-click the magnet button, open **Tools ▸ Snap**, or select the
Move tool and look at the **Snap** group in the Attribute Manager's Tool mode
(Attribute Manager ▸ Mode ▸ Tool). All three show the same settings.

![Tools ▸ Snap](images/manual/menu_snap.png) ![The Move tool's Snap options in the Attribute Manager](images/manual/snap_options.png)

| Setting | Meaning |
|---------|---------|
| Auto / 2D / 3D Snapping | **3D** moves the element onto the target in space. **2D** only lines it up on screen and keeps its depth. **Auto** is 3D in the perspective view, the only view so far. |
| Vertex, Edge, Edge Mid Point, Polygon Center, Polygon | Points, the nearest point along an edge, the middle of an edge, a polygon's centre, the surface under the cursor. All but Polygon are on at first. |
| Axis | Objects' origins (on at first). |
| Grid Point, Grid Line | Crossings or lines of the floor grid, at the spacing shown in the corner of the view. Grid Point is on at first. |
| Snap Radius | How close the cursor must be to a target, in screen points (10 at first). |

Within the radius, points win over edges and grid lines, which win over surfaces,
so a corner can be caught right next to its edges.

## 14. Tags

Tags add a property or a behaviour to an object without changing what kind of
object it is. They show as small icons to the right of the object's name in the
Object Manager.

![Tags in the Object Manager, and the selected Phong tag in the Attribute Manager](images/manual/tags_phong.png)

**Adding a tag:** select the objects, then choose a tag in the Object Manager's
**Tags** menu (or right-click an object ▸ *Add Tag*). Every selected object gets
the tag, in one undo step. An object has at most one tag of each kind.

![The Object Manager's Tags menu](images/manual/menu_tags.png)

**Editing:** click a tag icon. The Attribute Manager shows the tag's settings
instead of the object's (click the object's name to get back to the object). Every
change can be undone and is saved with the document.

**Removing:** click the tag icon, then press **Delete** (or right-click the icon ▸
*Delete Tag*). With a tag selected, Delete removes only the tag, not its object.

### 14.1 Phong

Controls how smooth the object looks.

- **Angle Limit** on: edges between polygons that meet at more than the **Phong
  Angle** stay hard, flatter ones are shaded smooth. 0° gives faceted, flat shading.
- **Angle Limit** off: everything connected is shaded smooth.
- The setting also decides the normals written by glTF and OBJ export.
- Objects without a Phong tag are smoothed up to 80°. (Cinema 4D instead shades
  objects without a Phong tag flat.)

![A sphere with a Phong tag at 0°: flat shading](images/manual/tags_flat.png)

### 14.2 Protection

Guards an object against accidental changes. Tick **Position**, **Scale** and/or
**Rotation**. A protected channel cannot be changed with the gizmo (the Move, Scale
or Rotate gizmo does not appear for the object) or in the Coordinates fields of the
Attribute Manager, which turn grey. The object's points, edges and polygons can still
be edited in the component modes.

### 14.3 Target

Turns the object to face another object.

- **Target Object:** the object to face. The object's forward axis (−Z, the
  direction Grid4D's cameras look) turns towards it, and its children turn along
  with it.
- **Up Vector:** optionally, an object the object's top turns towards (otherwise
  it keeps world up).
- **Pitch** off: the object only turns around the vertical axis and stays level.
- The object's own rotation values are not changed; the tag overrides them while
  it has a target. A target that is the object itself, one of its children, or
  hidden has no effect.

![The Target tag's settings](images/manual/tags_target.png)

### 14.4 Material

Links a material to the object; see [§15](#15-materials).

## 15. Materials

A **material** gives objects their surface. Materials live in the **Material
Manager**, the **Materials** tab at the bottom (beside Timeline and F-Curve;
**Shift+F2**, or Window ▸ Material Manager). An object shows a material through a
**Material tag**.

![A yellow material on the Sphere and on the Group, whose Cylinder and Box inherit it; the material in the Material Manager and the Attribute Manager](images/manual/materials.png)

**Making materials**

- **Create ▸ New Material** in the Material Manager, **Ctrl+N** with the pointer
  over it, a **double-click on its empty space**, or the main menu's
  **Create ▸ Material**. New materials are called Mat, Mat.1, …
- **Click** a material to select it: the Attribute Manager shows its settings.
  Selecting an object shows the object again.

**Settings** (Attribute Manager)

| Setting | Meaning |
|---------|---------|
| Name | shown under the preview |
| Color | the surface colour; the viewport shows it |
| Roughness | 0 % mirror-like … 100 % matte |
| Metallic | 0 % plastic, stone, wood … 100 % metal |
| Luminance | light the surface gives off |

In the viewport ([§4.1](#41-lights)) the colour is the surface's colour,
**Roughness** the size of its highlights (sharp and bright near 0 %, wide and faint
near 100 %), **Metallic** makes highlights and reflections take the colour (a metal
has no colour of its own otherwise) and **Luminance** adds glow that no light
needs. Constant Shading shows the colour and the luminance only.

**Giving an object a material**

- **Drag** the material from the Material Manager onto the object's row in the
  Object Manager, or onto the object in the viewport.
- Or select the objects and choose **Material ▸ Apply** in the Material Manager
  (or right-click the material ▸ *Apply to Selected Objects*).
- The object gets a **Material tag** (its icon shows the material's colour); if it
  has one already, the tag gets the new material. In the Attribute Manager the tag
  has a **Material** list to pick another one. **Double-click** the tag icon to see
  its material.
- **Children inherit**: an object without its own Material tag shows the material
  of its nearest parent that has one. A Subdivision Surface shows its own material,
  else its cage's.

**Deleting**: click a material, then press **Delete** (or Edit ▸ Delete, or
right-click ▸ *Delete*). Material tags that used it stay, without a material, and
their objects show their display colour again; undo brings everything back.

Every change is one undo step. Materials are saved with the document, travel with
glTF and OBJ files ([§9](#9-files)), and come along when you merge a document.

## 16. Keyboard shortcuts

Shortcuts follow Cinema 4D. **Chords** like `U~L` mean: press U, release it, then
press L. The status bar shows the pending first key and its choices. Any other key or
Esc cancels a chord.

| Keys | Action |
|------|--------|
| Alt + left / middle / right drag, wheel | Orbit / pan / zoom |
| H / O | Frame scene / frame selected objects |
| F1–F4 / F5 | View 1–4 alone / all four views |
| Middle click on a view | Show it alone / back among the others |
| N~A / N~B / N~C / N~D | Gouraud Shading / Gouraud Shading (Lines) / Quick Shading / Quick Shading (Lines) |
| N~E / N~F / N~G | Constant Shading / Hidden Line / Lines |
| N~H / N~K | Wireframe / Box |
| E / T / R | Move / Scale / Rotate tool |
| 9 / 0 | Live / Rectangle Selection tool |
| U~L / U~B | Loop / Ring Selection tool |
| U~I / U~W / U~Y / U~K | Invert / Select Connected / Grow / Shrink selection |
| C | Make Editable |
| Q | Switch the selected generator (or the one above the selection) off / on |
| Shift+S | Snapping on / off |
| Alt + click a generator button | Create the generator around the selected object |
| D / I | Extrude / Inset tool |
| M~T / M~W | Extrude / Inset tool (alternatives) |
| K~L (M~L) / K~K (M~K) | Loop/Path Cut / Line Cut tool |
| M~S | Bevel tool |
| B (M~B) | Bridge tool |
| M~M | Connect Points/Edges |
| M~N or Ctrl+Backspace | Dissolve |
| U~Z or Alt+Backspace | Melt |
| U~O | Optimize… |
| Delete / Backspace | Delete (objects in Model mode, components otherwise; keys after a click in the timeline's key strip) |
| Ctrl+A / Ctrl+Shift+A | Select all / deselect all |
| Ctrl+Z / Ctrl+Y (Ctrl+Shift+Z) | Undo / redo |
| Ctrl+N / Ctrl+O / Ctrl+S / Ctrl+Shift+S | New / open / save / save as |
| Ctrl+Shift+O | Merge a file into the document |
| F8 / F6 / F7 | Play / play backwards / stop |
| F / G | Previous / next frame |
| Ctrl+F / Ctrl+G | Previous / next key |
| F9 | Record Active Objects |
| Ctrl+F9 | Autokeying on / off |
| Shift+F5 / Shift+F7 | Attribute Manager / Coordinate Manager |
| Shift+F2 | Material Manager |
| Ctrl+N (pointer over the Material Manager) | New material |
| Tab (pointer over the timeline) | Timeline / F-Curve tab |
| H / S (pointer over the F-Curve graph) | Frame all curves / the selected keys |

**Help ▸ Keyboard Shortcuts & About** shows this list inside the application.

![The shortcuts dialog](images/manual/shortcuts.png)

## 17. Limitations and what is planned

The roadmap is in [`IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md). Not there yet:

- **Snapping**: Quantize (stepped moves, Shift+Q), per-tool settings, the workplane,
  guides and snapping for tools other than Move.
- **Modelling**:
  Extrude on points; Path mode for Loop/Path Cut; Bevel in Polygons mode and its
  Solid, Proportional and Radial modes; Bridge point by point.
- **Generators**: only the Subdivision Surface, with one type (Catmull-Clark) and
  without edge weights (creases) or UV options. Extrude, Lathe, Loft, Sweep and
  Cloner are planned.
- **Tags**: Protection limits, Phong edge breaks, copying tags between objects.
- **Materials**: one Material tag per object (no layering), projection settings,
  textures, rendered previews; the viewport shows only the colour.
- **Animation**: Ease presets, mirroring and quantizing keys, keeping a handle's
  angle or length while dragging it (Ctrl / Alt), normalised curves, copying and
  pasting keys, animating object parameters such as a cube's size, Frame
  Scene/Selected following the animation.
- **Coordinate Manager**: frozen coordinates (Object Abs), changing primitives'
  size parameters rather than their scale, editing several objects at once.
- **Viewport**: the Isoparms and Skeleton display styles, separate display
  settings for selected objects, backface culling, X-Ray and textures; lines are
  one pixel wide; three-view and the other four-view arrangements; the parallel
  and axonometric projections; the views and their arrangement are not saved with
  the document, and only View 1 can look through a camera; no shadows; no render
  resolution (camera pyramids use the view's proportions) and no depth-of-field
  handles.
- **Filter**: entries for kinds of objects Grid4D does not have yet (splines,
  deformers, …) and for the axes, handles, guides, SDS cage, n-gon lines and
  horizon; Default, Geometry Only and presets; the filter is not saved with the
  document.
- **Lights**: the Square Spot, Area, Parallel and IES types, shadows, visible
  light, and the per-light switches (no illumination, ambient, diffuse or specular
  only).
- **Interchange and rendering**: textures, animation in glTF, import
  and export dialogs (scale, axis flips, visible or selected objects only), the
  offline renderer (the Render menu is disabled).
- The Takes, Layers and Filter panels are placeholders.

See [`progress.md`](progress.md) for what changed in each step.
