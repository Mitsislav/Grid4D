# Scene cameras in the viewport

> Sources (read 2026-10-21): Cinema 4D manual —
> [Cameras menu](https://help.maxon.net/c4d/r21/us/html/5858.html) (each view has
> its own editor camera; *Use Camera* links a scene camera so the view looks
> through it; *Set Active Object as Camera* puts the view on the active object's
> axis, looking along it — useful to aim lights; *Default Camera* returns to the
> editor camera) and
> [Camera Object](https://help.maxon.net/c4d/en-us/Content/html/OCAMERA.html)
> (a new camera takes the current view's settings; double-clicking a camera's icon
> in the Object Manager links it to the view; a linked camera is moved by the
> view's navigation; the viewport draws a camera as a body with a viewing cone).
> Written in our own words.
>
> Covers implementation-plan step **7.1**. The 4-view layout and orthographic views
> are 7.2.

## Purpose

Seeing the scene through a camera object — to frame a shot, to animate a camera
move, or to look from a light — and seeing where the cameras are and what they
see.

## Behaviour

- **Cameras menu** (viewport header):
  - **Default Camera**: the view's own editor camera (as before 7.1). Checked while
    no object is linked.
  - **Use Camera ▸** every camera object in the scene, by name; the linked one is
    checked.
  - **Selected Object as Camera**: links the active object, whatever its kind.
- **Object Manager**: double-clicking a camera's icon links it (again: back to the
  default camera); the linked camera's icon is highlighted.
- **Looking through** an object: the view sits at the object's position and looks
  along its viewing axis (−Z, Grid4D's camera axis, as for Target tags), with the
  object's bank. A camera's **field of view** is used; other objects keep the view's
  own. Animation, Target tags and parents are followed: the view is where the
  object is at the current frame.
- **Navigating moves the linked object**: orbit, pan, zoom (and the header's
  navigation icons, H / O framing) move it, like dragging it with the gizmo — a
  temporary value or a key for animated channels (Autokeying), one undo step per
  gesture ("Move Camera"). A Protection tag on position or rotation keeps it (and
  the view) still.
- **Leaving** (Default Camera, or the object deleted): the editor camera comes back
  where it was before linking.
- **The view's title** shows the linked object's name instead of "Perspective".
- **Frustums**: every visible camera object except the linked one is drawn with a
  wire pyramid showing its field of view (the view's aspect ratio) and a small
  triangle on its top edge marking up; selected cameras are highlighted.
- **New cameras** take the view's position, direction and field of view (as before,
  now including bank and field of view).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Bank on the view camera; conversions between a world matrix and the view | `grid4d-render::OrbitCamera` (`roll`, `from_world`, `eye_matrix`) |
| The linked object | `EditorState::view_camera` (editor state, not saved) |
| Following it and writing navigation back | `ViewportPanel` (`NodeData::set_transform_at`, one `CommitLive` per gesture) |
| Frustums | `viewport/overlay.rs` |
| Menus | viewport header (Cameras), Object Manager icon double-click |

Deliberate differences: the linked camera is not saved with the document (the
reference keeps it with each view); no render-resolution aspect yet (frustums use
the view's), no focus distance or depth-of-field handles.

## Acceptance criteria

- [x] Linking a camera shows the scene from it (position, direction, bank, field of
      view); unlinking restores the editor view.
- [x] Orbit, pan and zoom move the linked camera, one undo step per gesture;
      Autokeying keys it; a Protection tag holds it.
- [x] An animated or Target-tagged camera is followed during playback.
- [x] Cameras other than the linked one are drawn with their frustums.
- [x] New cameras clone the view (with bank and field of view).
- [x] Idle frames leave the document untouched.
