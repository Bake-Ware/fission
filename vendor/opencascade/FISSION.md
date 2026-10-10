# Vendored `opencascade` crate

Copied from https://github.com/bschwind/opencascade-rs, crate `opencascade`,
at rev `32758df23a137f4e7c786c618b0317fc3badb3b2` (LGPL-2.1, see LICENSE). It
depends on `opencascade-sys` from that same rev, with the `builtin` feature
(bundled OCCT 7.8.1).

## Local changes

- `primitives/shape.rs`, end of file: `try_fillet_edges`, `try_chamfer_edges`,
  `try_union`, `try_subtract` and `try_intersect`. They check `IsDone()` before
  calling `Shape()`. OCCT throws `StdFail_NotDone` from `Shape()` when an
  algorithm fails, and a C++ exception crossing the cxx bridge calls
  `std::terminate`, so without these a too-large fillet aborts the whole app.
- `try_hollow` (shell) with the same IsDone check, and `volume()`.
- `lib.rs`: `Error::OperationFailed`, and re-export `opencascade_sys`; `Shape::ffi`/`from_ffi` for our C++ shim.
- `Cargo.toml`: sys and kicad-parser deps point at the pinned git rev.

Worth upstreaming. A complete fix would make the bridge functions return
`Result` with a cxx `trycatch` that also catches `Standard_Failure`, which
doesn't derive from `std::exception`.
