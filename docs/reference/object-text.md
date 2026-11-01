# Text spline

> Sources (read 2026-10-29): manual —
> [Text spline Object properties](https://help.maxon.net/c4d/en-us/Content/html/OSPLINETEXT-ID_OBJECTPROPERTIES.html).
> Glyph outlines: the TrueType / OpenType `glyf` and `CFF` outline formats as
> read by the `skrifa` crate (public specifications; `ttf-parser` until 14.3). Written in our own
> words — no copied text or images.
>
> Covers implementation-plan step **9.8**. Builds on
> [`object-spline.md`](object-spline.md) (spline primitives) and
> [`generator-extrude.md`](generator-extrude.md) (outlines with holes).

## Purpose

Writes text as splines: each letter's outline (with the holes of letters such
as *o*, *a* and *B*) becomes closed Bezier segments, so an Extrude turns the
text into solid 3D letters, a Loft or Sweep can use single letters, and Make
Editable gives an ordinary editable spline.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Text | multi-line text (Enter starts a new line) | "Text" | The characters written; lines run downwards |
| Font | — | Ubuntu Light (built in) | The typeface (one for now, see Decisions) |
| Align | Left · Center · Right | Left | Each line's position relative to the object's origin: starting at it, centred on it, ending at it |
| Height | cm, ≥ 0 | 200 cm | The font size: the height of the font's em square |
| Horizontal Spacing | cm, any | 0 cm | Added between letters (negative pulls them together) |
| Vertical Spacing | cm, any | 0 cm | Added between lines |
| Plane | XY · ZY · XZ | XY | As for the other spline primitives |
| Reverse | bool | off | Reverses every segment |
| Intermediate Points | as the other splines | Adaptive 5° | How the curves are cut into lines |

Not in 9.8 (later): choosing a font (system fonts or a file), automatic
kerning from the font, the per-letter Kerning tool and its 3D handles,
Separate Letters on Make Editable.

## Behaviour

- **Layout:** the first line's baseline lies on the X axis through the origin;
  each further line moves down by the font's line height (ascender − descender
  + line gap) plus Vertical Spacing. Letters advance by their own advance width
  plus Horizontal Spacing. Spaces and tabs move on without drawing; characters
  the font lacks are skipped (as wide as its "missing" glyph).
- **Outlines:** every contour of every glyph is one closed Bezier segment;
  straight parts get no tangents, quadratic curves (TrueType) are turned into
  the equal cubic ones. The spline is a Bezier spline in the object's space,
  so Make Editable keeps exactly the same curve.
- **Holes:** contours keep the font's own direction; the generators nest them
  even-odd, so holes are holes whichever way the font draws them.
- **In the viewport** the text is drawn as lines and picked near them, like
  every spline; framing uses the curve's bounds.
- **Undo, saving, generators:** edits are one undo step per interaction; the
  text is saved with its parameters (not its outline); the Extrude, Lathe,
  Sweep and Loft accept it as input.

## Edge cases

- Empty text, or only spaces: no segments (nothing drawn; an Extrude of it is
  empty).
- Height 0: every point at the origin; negative heights are not accepted.
- Very long text is limited when loading a file (10 000 characters), so a
  damaged file cannot make the app build a huge outline.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Text` (text as `Arc<str>`, height, align, spacings, plane, reverse, interpolation), `TextAlign` — plain data | `grid4d-geom::spline` |
| Outlines: `text_spline(&Text) -> Spline` (skrifa over the built-in font; ttf-parser until 14.3, same outlines), `FONT_NAME` | new crate `grid4d-text` (`crates/text`), depends on `grid4d-geom`, `skrifa`, `epaint_default_fonts` |
| `ObjectKind::Text(Text)`, tag `Text` in `SPLINES` | `scene/node.rs` |
| Curve cache (`CurveSource::Text`), bounds from the cached curve, spline inputs of generators | `grid4d-eval` |
| Make Editable → editable spline | `ui/src/commands.rs` |
| `.g4d` v3 `text` kind (additive; defaults omitted; length limited) | `grid4d-io::v3` |
| Icon, Create ▸ Spline ▸ Text, the toolbar spline list, Attribute Manager rows (multi-line text field) | `grid4d-ui` |

## Decisions (2026-10-29)

1. **The built-in font** is **Ubuntu Light**, the typeface egui already embeds
   in Grid4D for its interface (crate `epaint_default_fonts`, Ubuntu Font
   Licence 1.0): no new font file and no new licence enter the repository.
   The plan's "bundled OFL font" is met by this already-shipped open font; a
   font picker comes later.
2. **A new crate** keeps `grid4d-geom` "glam only": font parsing lives in
   `grid4d-text`, and the scene stores only the parameters.
3. **Height** is the em size, the usual meaning of a font size.
4. **Arc<str>** for the text keeps clones in evaluation cheap (no allocation
   when the scene is re-evaluated).

## Acceptance criteria

- [x] "o" gives two closed segments (outline and hole); "B" three; spaces
      none; two lines are one line height apart; Center and Right align each
      line on the origin.
- [x] Height scales the outline; Horizontal Spacing widens the text by
      (letters − 1) × spacing; Plane and Reverse as for other splines.
- [x] An Extrude of "Hello" is a set of closed solids with the holes cut out
      of the caps; the path tracer renders it.
- [x] Make Editable gives an editable spline with the same curve.
- [x] Saved and loaded with every parameter; idle frames with text do not
      touch the document.
