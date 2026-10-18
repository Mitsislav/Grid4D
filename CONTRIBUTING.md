# Contributing to Grid4D

Thanks for helping build an open-source 3D content creation suite!

## Before you start

* Read [`ARCHITECTURE.md`](docs/ARCHITECTURE.md) — it holds the **architectural rules** (crate
  dependency direction, the `EditorCommand` choke point, the allocation-free
  render loop, no `unsafe`, no `unwrap`, exhaustive `match`). Pull requests that
  break them will be asked to change.
* Read [`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md) to see what is
  planned and in which order. For larger work, open an issue first so we can agree
  on the design.

## Development

Requires Rust **1.95+**. On Windows you also need the MSVC build tools; on Linux the
usual X11/Wayland development packages (see the README).

```bash
cargo run                                              # dev build
cargo fmt --all                                        # format
cargo clippy --workspace --all-targets -- -D warnings  # lint
cargo test --workspace                                 # unit tests + WGSL validation
```

All three checks must pass before a change is considered done; CI runs them on
Windows, Linux and macOS.

## Pull requests

* One topic per pull request; keep them reviewable.
* Add tests next to the code you change. Hot-path buffers get a capacity-reuse
  test; `#[repr(C)]` structs get a size test; new shaders are added to
  `shaders_validate`.
* Update `ARCHITECTURE.md` (workspace tree, rules) and the implementation plan when your
  change adds a crate, panel or object type.

## Legal

Grid4D is *inspired by* the Cinema 4D workflow and must never copy it: no Maxon
icons, artwork, trademarks or file formats. All artwork must be original.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in Grid4D by you, as defined in the Apache-2.0 license, shall be
dual licensed as below, without any additional terms or conditions.

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
* MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
