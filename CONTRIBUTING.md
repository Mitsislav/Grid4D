# Contributing to Grid4D

Thanks for your interest in Grid4D! You can help by trying it and reporting
what goes wrong, by suggesting improvements, or with code, plugins, scripts
and documentation.

## Reporting a problem

Open an [issue](../../issues/new/choose) and include:

- what you did, what you expected and what happened instead;
- your system (Windows / macOS / Linux and its version) and graphics card;
- the Grid4D version or commit (Help ▸ Keyboard Shortcuts & About);
- a `.g4d` file or a screenshot when it helps.

If Grid4D closed unexpectedly, mention it: unsaved work is offered back on the
next start.

## Building from source

You need Rust **1.95 or newer** (`rustup update stable`). On Windows also the
MSVC build tools (installed with Visual Studio's "Desktop development with
C++"); on Linux the usual X11 / Wayland development packages:

```bash
sudo apt-get install libxkbcommon-dev libwayland-dev libx11-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
```

Then:

```bash
cargo run --release
```

## Making a change

Before larger work, open an issue so we can agree on the approach.
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) explains how the code is
organised and the rules every change follows — in short: dependencies point
one way between crates, every document change goes through an undoable
`SceneEdit`, the per-frame code does not allocate, and there is no `unsafe`
and no `unwrap()` outside tests. What is planned, and in which order, is in
[`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md).

Before you open a pull request, these must pass (CI runs them on Windows,
Linux and macOS):

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

And please:

- keep one topic per pull request;
- add tests next to the code you change;
- update the [manual](docs/manual.md) when users will notice the change, and
  give new interface texts their Greek in `crates/ui/src/i18n/el.rs` (a test
  lists the missing ones);
- update [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) when you add a crate,
  a panel or a kind of object.

Writing a plugin or a script? See [`docs/plugins.md`](docs/plugins.md) and
[`docs/scripting.md`](docs/scripting.md).

## Inspired by, never copied

Grid4D follows the workflow of Cinema 4D but must never copy it: no Maxon
icons, artwork, trademarks, text or file formats. All artwork is original, and
descriptions of behaviour are written in our own words.

## Licence

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in Grid4D by you, as defined in the Apache-2.0 license, shall be
dual licensed as below, without any additional terms or conditions.

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
* MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
