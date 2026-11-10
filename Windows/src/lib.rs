//! Platform-free core of AirWand: wire protocol, validation, filter, screen geometry.
//! Everything Windows-specific lives in `src/win/`.
pub mod geometry;
pub mod mux;
pub mod net;
pub mod one_euro;
pub mod protocol;
pub mod server;
pub mod validate;
