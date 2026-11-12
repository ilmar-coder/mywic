//! Base de erros: o que cada código significa e como resolver.

use crate::errors_data::{CANON_CODES, EPSON_ERRORS, EPSON_STATUS, EPSON_WARNINGS};

pub use crate::errors_data::CANON_SOURCE;

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub brand: String,
    pub code: String,
    pub title: String,
    pub cause: String,
    pub fix: String,
    /// Pode exigir assistência técnica.
    pub service: bool,
}

pub fn epson_status(code: u8) -> String {
    EPSON_STATUS
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, t)| t.to_string())
        .unwrap_or_else(|| format!("desconhecido (0x{code:02X})"))
}

pub fn epson_warning(code: u8) -> String {
    EPSON_WARNINGS
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, t)| t.to_string())
        .unwrap_or_else(|| format!("aviso 0x{code:02X}"))
}

pub fn explain_epson_error(code: u8) -> Problem {
    let (title, cause, fix, service) = EPSON_ERRORS
        .iter()
        .find(|e| e.0 == code)
        .map(|e| (e.1.to_string(), e.2.to_string(), e.3.to_string(), e.4))
        .unwrap_or_else(|| {
            (
                format!("Erro desconhecido (0x{code:02X})"),
                "Código não catalogado.".into(),
                "Desligue e ligue; se continuar, anote o código.".into(),
                false,
            )
        });
    Problem { brand: "Epson".into(), code: format!("0x{code:02X}"), title, cause, fix, service }
}

pub fn explain_canon(code: &str) -> Problem {
    let code = code.to_uppercase();
    match CANON_CODES.iter().find(|e| e.0 == code) {
        Some(e) => Problem {
            brand: "Canon".into(),
            code,
            title: e.1.into(),
            cause: e.2.into(),
            fix: e.3.into(),
            service: e.4,
        },
        None => Problem {
            brand: "Canon".into(),
            title: format!("Código {code} não catalogado"),
            code,
            cause: "Este código não está na lista do manual da G2060.".into(),
            fix: "Consulte o manual do seu modelo em ij.manual.canon.".into(),
            service: false,
        },
    }
}

/// Busca por código ou palavra nas duas bases.
pub fn search(text: &str) -> Vec<Problem> {
    let text = text.trim().to_uppercase();
    let mut found = Vec::new();
    let matches = |p: &Problem| text.len() > 2 && format!("{}{}", p.title, p.cause).to_uppercase().contains(&text);
    for e in CANON_CODES {
        let p = explain_canon(e.0);
        if text == e.0 || matches(&p) {
            found.push(p);
        }
    }
    for e in EPSON_ERRORS {
        let p = explain_epson_error(e.0);
        if text == p.code || text == format!("{:X}", e.0) || matches(&p) {
            found.push(p);
        }
    }
    found
}
