//! What fctl-control's tests share: a D1 database, a stand-in in memory
//! (`testing/d1.mjs`) with the migrations applied, that the control plane's
//! code runs against as it does against D1. They run in Node, under
//! wasm-bindgen-test (`cargo xtask check test`).

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::wasm_bindgen;
use worker::D1Database;

#[wasm_bindgen(module = "/src/testing/d1.mjs")]
extern "C" {
    #[wasm_bindgen(js_name = database)]
    fn stand_in(migrations: &str) -> wasm_bindgen::JsValue;
}

/// A fresh database, as D1 is once the migrations are applied.
pub fn database() -> D1Database {
    stand_in(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations")).unchecked_into()
}
