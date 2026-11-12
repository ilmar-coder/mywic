//! Canon IVEC pelo USB: status com o código de erro e manutenção (limpeza etc.).
//!
//! IVEC é o protocolo XML das Canon PIXMA/MegaTank desde ~2013 (aparece como
//! "IVEC" no campo CMD do Device ID). Mensagens montadas a partir da descrição
//! pública do protocolo (canon-universal-driver, docs/PROTOCOL_IVEC.md).

use std::time::{Duration, Instant};

use regex::Regex;

use crate::canon::CanonDiagnosis;
use crate::d4::UsbIo;
use crate::errors::explain_canon;
use crate::usb::{CANON_VENDOR, open_printer_interfaces};
use crate::{Error, Result, bail};

const XML_DECL: &str = r#"<?xml version="1.0" encoding="utf-8" ?>"#;
const XML_DECL2: &str = r#"<?xml version="1.0" encoding="utf-8"?>"#;
const NS_IVEC: &str = r#"xmlns:ivec="http://www.canon.com/ns/cmd/2008/07/common/""#;
const NS_VCN: &str = r#"xmlns:vcn="http://www.canon.com/ns/cmd/2008/07/canon/""#;
const CLOSE: &str = "</ivec:param_set></ivec:contents></cmd>";
const JOB_ID: &str = "00000002";

pub fn get_status() -> Vec<u8> {
    format!(
        "{XML_DECL}<cmd {NS_IVEC}><ivec:contents><ivec:operation>GetStatus</ivec:operation>\
         <ivec:param_set servicetype=\"print\"></ivec:param_set></ivec:contents></cmd>"
    )
    .into_bytes()
}

/// Operações de manutenção: (nome, operação, tipo, grupo de tinta).
pub const MAINTENANCE: &[(&str, &str, &str, &str)] = &[
    ("limpeza", "Cleaning", "regular", "all"),
    ("limpeza-preto", "Cleaning", "regular", "group1"),
    ("limpeza-cor", "Cleaning", "regular", "group2"),
    ("limpeza-profunda", "Cleaning", "deep", "all"),
    ("ink-flush", "Cleaning", "choke", "all"),
    ("rolete", "RollerCleaning", "roller", ""),
    ("placa-inferior", "RollerCleaning", "platen", ""),
    ("teste-bicos", "TestPrint", "nozzle_check", ""),
];

pub fn maintenance_names() -> Vec<&'static str> {
    MAINTENANCE.iter().map(|m| m.0).collect()
}

fn status_text(s: &str) -> String {
    match s {
        "idle" => "Pronta",
        "processing" => "Trabalhando",
        "stopped" => "Parada (erro)",
        "notready" => "Não está pronta",
        "busying" => "Ocupada",
        "canceling" => "Cancelando",
        "waitingprint" => "Aguardando impressão",
        "printing" => "Imprimindo",
        "held" => "Retida",
        other => other,
    }
    .into()
}

fn detail_text(s: &str) -> String {
    match s {
        "MediaEmpty" => "Sem papel",
        "MediaJam" => "Papel enroscado",
        "DoorOpen" => "Tampa aberta",
        "MarkerSupplyAttention" => "Atenção à tinta",
        "DeviceBusy" => "Ocupada",
        "AttentionRequired" => "Precisa de atenção",
        other => other,
    }
    .into()
}

fn head(operation: &str, service: &str, vendor_ns: bool, decl: &str) -> String {
    format!(
        "{decl}<cmd {NS_IVEC}{}><ivec:contents><ivec:operation>{operation}</ivec:operation>\
         <ivec:param_set servicetype=\"{service}\">",
        if vendor_ns { format!(" {NS_VCN}") } else { String::new() }
    )
}

/// Bytes de um trabalho de manutenção IVEC completo. `now`: AAAAMMDDhhmmss.
pub fn maintenance_job(name: &str, now: &str, description: &str) -> Result<Vec<u8>> {
    let Some(&(_, operation, kind, group)) = MAINTENANCE.iter().find(|m| m.0 == name) else {
        bail!("operação desconhecida: {name}");
    };
    let mut parts = vec![format!(
        "{}<ivec:jobID>{JOB_ID}</ivec:jobID><ivec:bidi>0</ivec:bidi><ivec:jobname/><ivec:username/>\
         <ivec:computername/><ivec:job_description><![CDATA[{description}]]></ivec:job_description>\
         <vcn:host_environment>linux</vcn:host_environment>{CLOSE}",
        head("StartJob", "maintenance", true, XML_DECL)
    )];
    parts.push(format!(
        "{}<ivec:jobID>{JOB_ID}</ivec:jobID><ivec:datetime>{now}</ivec:datetime>{CLOSE}",
        head("SetJobConfiguration", "maintenance", true, XML_DECL)
    ));
    // aqui os parâmetros da operação vêm antes do jobID
    let group = if group.is_empty() { String::new() } else { format!("<ivec:inkgroup>{group}</ivec:inkgroup>") };
    parts.push(format!(
        "{}{group}<ivec:type>{kind}</ivec:type><ivec:jobID>{JOB_ID}</ivec:jobID>{CLOSE}",
        head(operation, "maintenance", false, XML_DECL2)
    ));
    parts.push(format!("{}<ivec:jobID>{JOB_ID}</ivec:jobID>{CLOSE}", head("EndJob", "maintenance", false, XML_DECL)));
    Ok(parts.concat().into_bytes())
}

fn last(xml: &str, tag: &str) -> String {
    let re = Regex::new(&format!("<ivec:{tag}>([^<]*)</ivec:{tag}>")).unwrap();
    re.captures_iter(xml).last().map(|c| c[1].trim().to_string()).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
pub struct IvecStatus {
    pub status: String,
    pub detail: String,
    pub support_code: String,
    pub error_codes: Vec<String>,
    /// (modelo da caixa, nível %, ícone)
    pub maintenance_box: Vec<(String, Option<i32>, String)>,
}

/// GetStatusResponse (o último valor lido vale).
pub fn parse_status(data: &[u8]) -> Option<IvecStatus> {
    let xml = String::from_utf8_lossy(data);
    let status = last(&xml, "status");
    if status.is_empty() {
        return None;
    }
    let mut code = last(&xml, "current_support_code");
    if code.is_empty() {
        code = last(&xml, "error_code");
    }
    let waste = Regex::new(r"(?s)<ivec:wasteink>(.*?)</ivec:wasteink>").unwrap();
    let item = Regex::new(r"(?s)<ivec:item>(.*?)</ivec:item>").unwrap();
    let mut boxes = Vec::new();
    if let Some(w) = waste.captures(&xml) {
        for it in item.captures_iter(&w[1]) {
            let model = last(&it[1], "model");
            boxes.push((
                if model.is_empty() { "caixa".into() } else { model },
                last(&it[1], "level").parse().ok(),
                last(&it[1], "icon"),
            ));
        }
    }
    let errors = Regex::new(r"<ivec:error_code>([^<]+)</ivec:error_code>").unwrap();
    let error_codes = errors
        .captures_iter(&xml)
        .map(|c| c[1].to_string())
        .filter(|e| e != "0" && *e != code)
        .map(|e| e.to_uppercase())
        .collect();
    Some(IvecStatus {
        status,
        detail: last(&xml, "status_detail"),
        support_code: if code.is_empty() || code == "0" { String::new() } else { code.to_uppercase() },
        error_codes,
        maintenance_box: boxes,
    })
}

/// Canon IVEC ligada no USB, pela interface de impressora bidirecional.
pub struct CanonUsb {
    io: Box<dyn UsbIo>,
    pub product: String,
}

impl CanonUsb {
    pub fn open() -> Result<CanonUsb> {
        let mut list = open_printer_interfaces(CANON_VENDOR)?;
        if list.is_empty() {
            bail!("nenhuma Canon IVEC disponível no USB (está ligada? outro programa está imprimindo?)");
        }
        let (info, io) = list.remove(0);
        Ok(CanonUsb { io, product: info.product })
    }

    pub fn from_io(io: Box<dyn UsbIo>, product: &str) -> CanonUsb {
        CanonUsb { io, product: product.into() }
    }

    /// Primeiro GetStatusResponse completo. A impressora responde com o status a
    /// qualquer leitura; só pedimos com GetStatus se ela ficar quieta.
    pub fn status(&mut self) -> Result<IvecStatus> {
        let mut buffer = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut asked = false;
        while Instant::now() < deadline {
            let chunk = self.io.read(700)?;
            if !chunk.is_empty() {
                buffer.extend(chunk);
                if buffer.windows(6).any(|w| w == b"</cmd>")
                    && let Some(st) = parse_status(&buffer) {
                        return Ok(st);
                    }
            } else if !asked {
                asked = true;
                // com erro grave ela pode recusar a escrita, mas continua respondendo
                let _ = self.io.write(&get_status());
            }
        }
        Err(Error::new("a impressora não respondeu ao pedido de status"))
    }

    /// Envia a operação, recusando se a impressora estiver com erro.
    pub fn maintenance(&mut self, name: &str, force: bool) -> Result<IvecStatus> {
        let st = self.status()?;
        if !st.support_code.is_empty() && !force {
            bail!("a impressora está com o erro {}; resolva antes de fazer manutenção", st.support_code);
        }
        let now = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
        let job = maintenance_job(name, &now, &uuid::Uuid::new_v4().to_string())?;
        self.io.write(&job)?;
        Ok(st)
    }

    /// Mesmo formato do diagnóstico pelo IPP, mas com o código exato.
    pub fn diagnose(&mut self) -> Result<CanonDiagnosis> {
        let st = self.status()?;
        let mut codes: Vec<String> = Vec::new();
        for c in std::iter::once(&st.support_code).chain(st.error_codes.iter()) {
            if !c.is_empty() && !codes.contains(c) {
                codes.push(c.clone());
            }
        }
        Ok(CanonDiagnosis {
            modelo: self.product.clone(),
            estado: status_text(&st.status),
            mensagem: detail_text(&st.detail),
            motivos: Vec::new(),
            problemas: codes.iter().map(|c| explain_canon(c)).collect(),
            tintas: st
                .maintenance_box
                .iter()
                .map(|(m, lvl, _)| (format!("Caixa de manutenção {m}"), lvl.map(|l| format!("{l}%")).unwrap_or("n/d".into())))
                .collect(),
        })
    }
}
