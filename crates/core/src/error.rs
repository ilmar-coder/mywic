//! Erro único do núcleo: uma mensagem em português para mostrar ao usuário.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub message: String,
    /// Código de resultado do D4 (IEEE 1284.4), quando o erro veio dele.
    pub d4_code: Option<u8>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Error { message: message.into(), d4_code: None }
    }

    pub fn d4(code: u8, message: impl Into<String>) -> Self {
        Error { message: message.into(), d4_code: Some(code) }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::new(format!("JSON inválido: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Atalho: `bail!("texto {x}")` devolve Err(Error).
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => { return Err($crate::Error::new(format!($($arg)*))) };
}
