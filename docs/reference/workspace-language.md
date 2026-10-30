# The interface language

> Sources (read 2026-10-30): manual — the Preferences' Interface page (the
> language of menus, managers and messages, chosen by the user). Written in
> our own words. The Greek wording is our own translation.
>
> Covers implementation-plan step **12.6**.

## Purpose

Let people work in their own language. Grid4D speaks **English** (the
default) and **Greek**.

## Behaviour

- **Preferences ▸ Interface ▸ Language**: English or Ελληνικά. The change is
  immediate — no restart — and is remembered.
- Translated: the menus, the command palette's and every button's tooltips,
  the managers (Object, Attribute, Coordinate, Material, Layer and Take
  Managers, the timeline and F-Curve editor), the Render Settings and Picture
  Viewer, the Preferences, dialogs and alerts, the status bar's hints, the
  names of object types, tags and tools as the interface shows them, and the
  pane tabs and layouts.
- Not translated: what the document holds — object, material, layer and take
  names (a new Cube is called "Cube" in either language, so files read the
  same everywhere) — units and their symbols, shortcut keys, and the user
  manual (English for now).
- A text with no Greek yet shows in English.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Language` (English, Greek), `tr` (the English text is the key), `trf` (numbered placeholders `{0}`, `{1}`), the language in use kept in an atomic (`set_language`, `follow`) | `ui::i18n` |
| The Greek table: sorted pairs, looked up by binary search | `ui::i18n::el` |
| `Preferences::language`; Preferences ▸ Interface ▸ Language | `ui::preferences`, `ui::panels::preferences` |
| A test that finds every `tr` / `trf` literal (and every row label and enum label shown) missing from the Greek table, and every table entry no code uses | `ui::i18n` tests |

## Decisions (2026-10-30)

1. **English text as the key** (plan D15): code stays readable, and English
   is the fallback.
2. **One atomic** holds the language, as for the theme: reading costs nothing
   and switching needs no restart.
3. **Document data is never translated**; only what the interface says.
4. **Later:** more languages, a Greek manual, right-to-left scripts.

## Acceptance criteria

- [x] English by default; Greek chosen in the Preferences applies at once
      and after a restart.
- [x] The whole interface reads in Greek; no text the code shows is missing
      from the table (a test).
- [x] Documents saved in either language are the same.
