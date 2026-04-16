// ACP Web – Leptos CSR WebAssembly initialisation.
//
// This script is the only authored JavaScript in the web frontend. It
// imports the stable backend-served alias (acp-web-frontend.js) for the
// wasm-bindgen generated loader and
// calls init(), which fetches and instantiates the compiled WebAssembly
// module.  The Rust #[wasm_bindgen(start)] entry-point (lib.rs::main)
// is invoked automatically once the module is ready.
//
// The loader and the .wasm binary are compiled from
// crates/acp-web-frontend/src/ via `trunk build --release`.

import init from "./acp-web-frontend.js";

await init();
