# slint-mapping-wasm-demo

Browser build of the [slint-mapping](https://github.com/gregoryraymond/slint-mapping)
live demo — pan / zoom OSM tiles with marker and polyline overlays,
compiled to wasm.

This crate exists only to be built by `wasm-pack`; the API is the
parent crate's. **Not published to crates.io.**

## Build

```sh
# from the slint-mapping repo root
wasm-pack build --release --target web wasm-demo \
  --out-dir ../dist/pkg --no-typescript
cp wasm-demo/web/index.html dist/index.html
# serve dist/ with any static HTTP server
```

The [`pages.yml`](../.github/workflows/pages.yml) GitHub Actions
workflow runs these exact commands on every push to `main` and
deploys the result to <https://gregoryraymond.github.io/slint-mapping/>.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE),
at your option — same as the parent crate.
