# WisteriaSinensis

`mutil-engine` is a headless typesetting and interactive layout engine for multilingual text. It lays out paragraphs in horizontal and vertical writing modes, and its output is validated geometry that any renderer can draw.

- [SPECIFICATION.md](SPECIFICATION.md): the normative contract.
- [RESEARCH.md](RESEARCH.md): background, sources and design history.
- [docs/host-integration.md](docs/host-integration.md): non-normative notes for renderers.
- [CONTRIBUTION.md](CONTRIBUTION.md): how to contribute.
- [demo/](demo/README.md): a browser harness that draws the engine's
  validated output; it shows what Phase 1 actually does, including the
  features it rejects.

## Build

Rust 1.88 or later.

```sh
cd mutil-engine
cargo test --all-targets
python3 ../scripts/check_spec.py   # spec and crate agree
```

## Licence

GPL-3.0-or-later; see [LICENSE](LICENSE).
