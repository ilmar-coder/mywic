//! EcoReset: manutenção de impressoras Epson L/EcoTank e Canon.
//!
//! Pela rede (SNMP) ou pelo cabo USB (D4 na Epson, IVEC na Canon): contadores da
//! almofada, reset, EEPROM, diagnóstico de erros, teste de bicos e limpeza, e o
//! banco de modelos aprendidos com o log do WICReset.

pub mod canon;
pub mod d4;
mod error;
pub mod errors;
mod errors_data;
pub mod ipp;
pub mod ivec;
pub mod keyfinder;
pub mod library;
pub mod maintenance;
pub mod models;
mod models_data;
pub mod printer;
pub mod server;
pub mod snapshot;
pub mod snmp;
pub mod usb;
pub mod wiclog;
pub mod winusb;

pub use error::{Error, Result};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Modelos da lista + os aprendidos do banco local (um aprendido vale no lugar).
pub fn all_models() -> Vec<models::Model> {
    models::merge(library::load().unwrap_or_default())
}
