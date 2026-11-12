//! Canon PIXMA / MegaTank: diagnóstico pelo IPP e manutenção por comandos BJL.
//!
//! O contador da almofada (erro 5B00) só é zerado pela ferramenta de serviço da
//! Canon. Modelos com caixa de manutenção (G2060, G3160...) não precisam de
//! reset: a caixa é trocável (códigos 1726/1727).

use crate::errors::{Problem, explain_canon};
use crate::ipp::{self, Attrs, Value};
use crate::{Result, bail};

pub const BJL_START: &[u8] = b"\x1b[K\x02\x00\x00\x1fBJLSTART\n";
pub const BJL_END: &[u8] = b"BJLEND\n";

pub fn bjl_job(commands: &[&str]) -> Vec<u8> {
    let mut job = BJL_START.to_vec();
    for c in commands {
        job.extend(c.as_bytes());
        job.push(b'\n');
    }
    job.extend(BJL_END);
    job
}

pub fn head_cleaning_job(black_only: bool) -> Vec<u8> {
    bjl_job(&[if black_only { "@Cleaning=1K" } else { "@Cleaning=1ALL" }])
}

pub fn nozzle_check_job() -> Vec<u8> {
    bjl_job(&["@TestPrint=NozzleCheck"])
}

/// Mesmo formato para o diagnóstico pelo IPP e pelo IVEC.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CanonDiagnosis {
    pub modelo: String,
    pub estado: String,
    pub mensagem: String,
    pub motivos: Vec<String>,
    pub problemas: Vec<Problem>,
    pub tintas: Vec<(String, String)>,
}

/// Linguagens que a impressora declara no Device ID (campo CMD).
pub fn command_sets(attrs: &Attrs) -> Vec<String> {
    let device_id = ipp::texts(attrs, "printer-device-id").first().copied().unwrap_or_default();
    for part in device_id.split(';') {
        if part.starts_with("CMD:") || part.starts_with("COMMAND SET:") {
            return part.split_once(':').unwrap().1.split(',').map(|c| c.trim().to_string()).collect();
        }
    }
    Vec::new()
}

/// Erro claro se o modelo não aceita BJL (os mais novos só falam IVEC).
pub fn check_bjl(url: &str) -> Result<()> {
    let sets = command_sets(&ipp::get_printer_attributes(url)?);
    if !sets.is_empty() && !sets.iter().any(|s| s == "BJL") {
        bail!(
            "este modelo não aceita comandos de manutenção BJL (declara {}). Use os botões da \
             impressora ou o utilitário da Canon (IJ Printer Assistant).",
            sets.join(", ")
        );
    }
    Ok(())
}

pub fn diagnose(url: &str) -> Result<CanonDiagnosis> {
    Ok(diagnose_attrs(&ipp::get_printer_attributes(url)?))
}

/// Níveis especiais do IPP (RFC 8011): -2 desconhecido, -3 há algum restante.
fn level_text(v: i32) -> String {
    match v {
        -1 => "n/d".into(),
        -2 => "sem sensor".into(),
        -3 => "tem tinta".into(),
        n => format!("{n}%"),
    }
}

pub fn diagnose_attrs(attrs: &Attrs) -> CanonDiagnosis {
    let state = attrs.get("printer-state").and_then(|v| v.first()).and_then(Value::int).unwrap_or(0);
    let first = |key: &str| ipp::texts(attrs, key).first().map(|s| s.to_string()).unwrap_or_default();
    let names = ipp::texts(attrs, "marker-names");
    let levels = attrs.get("marker-levels").cloned().unwrap_or_default();
    let mut d = CanonDiagnosis {
        modelo: first("printer-make-and-model"),
        estado: match state {
            3 => "Pronta",
            4 => "Imprimindo",
            5 => "Parada",
            _ => "desconhecido",
        }
        .into(),
        mensagem: first("printer-state-message"),
        motivos: ipp::reasons(attrs).into_iter().filter(|(k, _)| k != "none").map(|(_, t)| t).collect(),
        problemas: ipp::support_codes(attrs).iter().map(|c| explain_canon(c)).collect(),
        tintas: names
            .iter()
            .zip(levels.iter())
            .map(|(n, v)| (n.to_string(), v.int().map(level_text).unwrap_or_else(|| format!("{v:?}"))))
            .collect(),
    };
    if state == 5 && d.problemas.is_empty() && d.mensagem.is_empty() {
        d.mensagem = "A impressora está parada com erro, mas não informou o código pelo IPP. Veja o \
                      código no visor ou conte as piscadas da luz de alarme e consulte-o no campo \
                      'Consultar código'."
            .into();
    }
    d
}
