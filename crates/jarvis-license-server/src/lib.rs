//! Jarvis License Server.
//!
//! The server is the source of truth: a key exists only if an administrator created it HERE; a subscription starts, ends,
//! is extended or revoked HERE; a license is bound to ONE installation HERE, in one database transaction. The program of the
//! customer only asks and verifies the signed answer.
//!
//! Layers: `store` (the database and every rule), `api` (the routes, the authentication, the rate limits, JSON),
//! `config` (environment, key files), `http` (a thin tiny_http loop). Tests call `api::Api::handle` with a fixed clock.

pub mod api;
pub mod config;
pub mod http;
pub mod store;
