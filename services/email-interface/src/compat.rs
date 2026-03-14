//! Compatibility traits for native and WASM builds.
//!
//! Rig uses WASM-compatible send/sync traits to avoid over-constraining APIs on
//! single-threaded targets. This service is native-first, but the shared agent
//! abstractions still use the compatibility traits instead of raw `Send`/`Sync`.

#[cfg(not(target_family = "wasm"))]
pub trait WasmCompatSend: Send {}

#[cfg(not(target_family = "wasm"))]
impl<T> WasmCompatSend for T where T: Send {}

#[cfg(target_family = "wasm")]
pub trait WasmCompatSend {}

#[cfg(target_family = "wasm")]
impl<T> WasmCompatSend for T {}

#[cfg(not(target_family = "wasm"))]
pub trait WasmCompatSync: Sync {}

#[cfg(not(target_family = "wasm"))]
impl<T> WasmCompatSync for T where T: Sync {}

#[cfg(target_family = "wasm")]
pub trait WasmCompatSync {}

#[cfg(target_family = "wasm")]
impl<T> WasmCompatSync for T {}
