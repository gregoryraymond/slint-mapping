# slint-mapping-wasm-demo

Browser build of the [slint-mapping](https://github.com/gregoryraymond/slint-mapping)
live demo — pan / zoom OSM tiles with marker and polyline overlays,
compiled to wasm.

This crate exists only to be built by `wasm-pack`; the API is the
parent crate's. **Not published to crates.io.**

## Build + serve

Uses [trunk](https://trunkrs.dev/) — one tool that builds the wasm,
serves it on `http://127.0.0.1:8765/`, and live-reloads on file
changes. Install once with `cargo install trunk` (or
`just install-publish-tools` from the repo root, which now bundles
it in).

```sh
# Dev loop — build + serve + reload on save:
cd wasm-demo && trunk serve

# One-off release build into wasm-demo/dist/:
cd wasm-demo && trunk build --release
```

The [`pages.yml`](../.github/workflows/pages.yml) GitHub Actions
workflow runs `trunk build --release --public-url /slint-mapping/`
on every push to `main` and deploys the result to
<https://gregoryraymond.github.io/slint-mapping/>.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE),
at your option — same as the parent crate.
