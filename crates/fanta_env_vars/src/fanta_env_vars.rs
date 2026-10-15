pub use env_var::{EnvVar, bool_env_var, env_var};
use std::sync::LazyLock;

/// Whether Fanta is running in stateless mode.
/// When true, Fanta will use in-memory databases instead of persistent storage.
pub static FANTA_STATELESS: LazyLock<bool> = bool_env_var!("FANTA_STATELESS");
