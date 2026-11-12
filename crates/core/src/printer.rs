//! Comandos EPSON-CTRL: identificação, EEPROM e contadores da almofada.
//!
//! Os mesmos quadros vão pela rede (dentro de um OID SNMP) ou pelo cabo USB
//! (canal EPSON-CTRL do D4); quem leva é um `Transport`.

use std::collections::BTreeMap;

use regex::bytes::Regex;
use serde_json::{Map, Value, json};
use sha1::{Digest, Sha1};

use crate::errors::{Problem, epson_status, epson_warning, explain_epson_error};
use crate::models::{Model, find_model};
use crate::snmp::SnmpClient;
use crate::{Error, Result, bail};

pub const EPCTRL_OID: &str = "1.3.6.1.4.1.1248.1.2.2.44.1.1.2.1";
pub const MODEL_OID: &str = "1.3.6.1.2.1.25.3.2.1.3.1";
/// MAC do Wi-Fi nos modelos que o guardam aqui.
pub const MAC_ADDRESSES: std::ops::Range<u16> = 0x780..0x786;
pub const MAX_IMAGE: usize = 4096;

/// Quem leva os quadros EPSON-CTRL até a impressora.
pub trait Transport: Send {
    /// Envia um quadro e devolve a resposta (None: a impressora não tem o objeto).
    fn command(&mut self, frame: &[u8]) -> Result<Option<Vec<u8>>>;
    /// Nome do modelo como a impressora informa.
    fn model_string(&mut self) -> Result<String>;
}

pub struct SnmpTransport(pub SnmpClient);

impl Transport for SnmpTransport {
    fn command(&mut self, frame: &[u8]) -> Result<Option<Vec<u8>>> {
        self.0.get(&frame_oid(frame))
    }

    fn model_string(&mut self) -> Result<String> {
        Ok(self.0.get(MODEL_OID)?.map(|v| latin1(&v).trim().to_string()).unwrap_or_default())
    }
}

pub fn frame_oid(frame: &[u8]) -> String {
    let mut oid = String::from(EPCTRL_OID);
    for b in frame {
        oid.push('.');
        oid.push_str(&b.to_string());
    }
    oid
}

pub fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|b| *b as char).collect()
}

/// Quadro EPSON-CTRL: 2 letras + tamanho (LE, 2 bytes) + dados.
pub fn ctrl_frame(cmd: &[u8; 2], payload: &[u8]) -> Vec<u8> {
    let mut f = cmd.to_vec();
    f.extend((payload.len() as u16).to_le_bytes());
    f.extend_from_slice(payload);
    f
}

/// A chave de gravação vai com cada byte somado de 1 (0 continua 0).
pub fn shift_key(key: &[u8]) -> Vec<u8> {
    key.iter().map(|b| if *b == 0 { 0 } else { b.wrapping_add(1) }).collect()
}

pub fn read_frame(read_key: [u8; 2], address: u16) -> Vec<u8> {
    let a = address.to_le_bytes();
    ctrl_frame(b"||", &[read_key[0], read_key[1], 0x41, 0xBE, 0xA0, a[0], a[1]])
}

pub fn write_frame(read_key: [u8; 2], write_key: &[u8; 8], address: u16, value: u8) -> Vec<u8> {
    let a = address.to_le_bytes();
    let mut p = vec![read_key[0], read_key[1], 0x42, 0xBD, 0x21, a[0], a[1], value];
    p.extend(shift_key(write_key));
    ctrl_frame(b"||", &p)
}

/// Decodifica a resposta "@BDC ST2" em {tipo: bytes}. 0x1F/0x40 é o número de série.
pub fn parse_status(reply: &[u8]) -> BTreeMap<u8, Vec<u8>> {
    let mut items = BTreeMap::new();
    let marker = b"BDC ST2\r\n";
    let Some(start) = reply.windows(marker.len()).position(|w| w == marker) else {
        return items;
    };
    let mut pos = start + marker.len() + 2; // pula o tamanho total
    while pos + 2 <= reply.len() {
        let (kind, len) = (reply[pos], reply[pos + 1] as usize);
        let end = (pos + 2 + len).min(reply.len());
        items.insert(kind, reply[pos + 2..end].to_vec());
        pos += 2 + len;
    }
    items
}

/// values: bytes na ordem dos endereços (menos significativo primeiro).
pub fn waste_percent(values: &[u8], divider: f64) -> f64 {
    let raw = values.iter().rev().fold(0u64, |acc, b| (acc << 8) | *b as u64);
    (raw as f64 / divider * 100.0).round() / 100.0
}

pub fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// Lê um arquivo de EEPROM: JSON (backup/leitura do EcoReset) ou binário.
/// No binário, o primeiro byte vai para o endereço `start`.
pub fn load_image(path: &str, start: u16) -> Result<(BTreeMap<u16, u8>, Option<String>)> {
    let raw = std::fs::read(path)?;
    let mut image = BTreeMap::new();
    let mut model = None;
    if path.to_lowercase().ends_with(".json") {
        let data: Value = serde_json::from_slice(&raw)?;
        let cells = data.get("eeprom").unwrap_or(&data);
        let obj = cells.as_object().ok_or_else(|| Error::new("arquivo sem a lista da EEPROM"))?;
        for (a, v) in obj {
            let addr = parse_int(a).filter(|n| *n <= 0xFFFF);
            let val = v.as_u64().filter(|n| *n <= 255);
            match (addr, val) {
                (Some(a), Some(v)) => {
                    image.insert(a as u16, v as u8);
                }
                _ => bail!("arquivo inválido: endereço {a} valor {v}"),
            }
        }
        model = data.get("model").and_then(|m| m.as_str()).map(String::from);
    } else {
        for (i, b) in raw.iter().enumerate() {
            let a = start as usize + i;
            if a > 0xFFFF {
                bail!("arquivo grande demais para a EEPROM");
            }
            image.insert(a as u16, *b);
        }
    }
    Ok((image, model))
}

/// "48", "0x30" ou "0b..": inteiro como no Python int(x, 0).
pub fn parse_int(text: &str) -> Option<u64> {
    let t = text.trim();
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else if let Some(b) = t.strip_prefix("0b") {
        u64::from_str_radix(b, 2).ok()
    } else {
        t.parse().ok()
    }
}

/// Backup dos endereços que serão alterados, para poder desfazer.
#[derive(Debug, Clone, PartialEq)]
pub struct Backup {
    pub printer: String,
    pub model: Option<String>,
    pub date: String,
    pub eeprom: Vec<(u16, u8)>,
}

impl Backup {
    pub fn to_json(&self) -> Value {
        let mut cells = Map::new();
        for (a, v) in &self.eeprom {
            cells.insert(a.to_string(), json!(v));
        }
        json!({"printer": self.printer, "model": self.model, "date": self.date, "eeprom": cells})
    }

    pub fn save(&self, path: &str) -> Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(&self.to_json())?)?;
        Ok(())
    }

    pub fn load(path: &str) -> Result<Backup> {
        let data: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let obj = data["eeprom"].as_object().ok_or_else(|| Error::new("não é um backup do EcoReset"))?;
        let mut eeprom = Vec::new();
        for (a, v) in obj {
            let a = parse_int(a).filter(|n| *n <= 0xFFFF).ok_or_else(|| Error::new("backup inválido"))?;
            let v = v.as_u64().filter(|n| *n <= 255).ok_or_else(|| Error::new("backup inválido"))?;
            eeprom.push((a as u16, v as u8));
        }
        Ok(Backup {
            printer: data["printer"].as_str().unwrap_or_default().into(),
            model: data["model"].as_str().map(String::from),
            date: data["date"].as_str().unwrap_or_default().into(),
            eeprom,
        })
    }

    fn value(&self, address: u16) -> Option<u8> {
        self.eeprom.iter().find(|(a, _)| *a == address).map(|(_, v)| *v)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnosis {
    pub estado: String,
    pub problemas: Vec<Problem>,
    pub avisos: Vec<String>,
}

pub struct EpsonPrinter {
    pub transport: Box<dyn Transport>,
    pub host: String,
    pub model: Option<Model>,
    pub read_key: Option<[u8; 2]>,
    pub write_key: Option<[u8; 8]>,
}

fn ee_regex() -> Regex {
    Regex::new(r"EE:([0-9A-Fa-f]{6})").unwrap()
}

impl EpsonPrinter {
    pub fn new(host: &str, transport: Box<dyn Transport>) -> Self {
        EpsonPrinter { transport, host: host.into(), model: None, read_key: None, write_key: None }
    }

    pub fn network(host: &str) -> Self {
        EpsonPrinter::new(host, Box::new(SnmpTransport(SnmpClient::new(host))))
    }

    // ---- identificação ----------------------------------------------------

    pub fn set_model(&mut self, model: &Model) {
        self.read_key = Some(model.read_key);
        self.write_key = Some(model.write_key);
        self.model = Some(model.clone());
    }

    pub fn model_name(&self) -> Option<String> {
        self.model.as_ref().map(|m| m.name.clone())
    }

    pub fn model_string(&mut self) -> Result<String> {
        self.transport.model_string()
    }

    /// Resposta do comando "di" (MFG, MDL, CMD...).
    pub fn device_id(&mut self) -> Result<BTreeMap<String, String>> {
        let Some(value) = self.transport.command(&ctrl_frame(b"di", &[1]))? else {
            return Ok(BTreeMap::new());
        };
        let re = regex::Regex::new(r"(MFG|CMD|MDL|CLS|DES):([^;]*);").unwrap();
        let text = latin1(&value);
        Ok(re.captures_iter(&text).map(|c| (c[1].to_string(), c[2].to_string())).collect())
    }

    /// Lê o nome do modelo e escolhe os parâmetros. Devolve (texto, nome do modelo).
    pub fn detect(&mut self, models: &[Model]) -> Result<(String, Option<String>)> {
        let mut text = self.model_string()?;
        if text.is_empty() {
            text = self.device_id()?.get("MDL").cloned().unwrap_or_default();
        }
        let found = find_model(&text, models).cloned();
        if let Some(m) = &found {
            self.set_model(m);
        }
        Ok((text, found.map(|m| m.name)))
    }

    // ---- EEPROM -----------------------------------------------------------

    fn need_keys(&self, write: bool) -> Result<()> {
        if self.read_key.is_none() {
            bail!("escolha o modelo (ou informe a chave de leitura)");
        }
        if write && self.write_key.is_none() {
            bail!("falta a chave de gravação para este modelo");
        }
        Ok(())
    }

    pub fn read_eeprom(&mut self, address: u16) -> Result<u8> {
        self.need_keys(false)?;
        let value = self.transport.command(&read_frame(self.read_key.unwrap(), address))?;
        let value = match value {
            Some(v) if !v.windows(4).any(|w| w == b":NA;") => v,
            _ => bail!(
                "a impressora recusou a leitura do endereço {address} (chave errada ou \
                 firmware bloqueado: tente pelo cabo USB, que costuma funcionar mesmo \
                 assim, ou use o reset temporário)"
            ),
        };
        let caps = ee_regex()
            .captures(&value)
            .ok_or_else(|| Error::new(format!("resposta inesperada: {}", latin1(&value))))?;
        let hex = std::str::from_utf8(&caps[1]).unwrap_or_default();
        let got = u16::from_str_radix(&hex[..4], 16).unwrap_or(0);
        if got != address {
            bail!("a impressora respondeu outro endereço ({})", &hex[..4]);
        }
        Ok(u8::from_str_radix(&hex[4..], 16).unwrap_or(0))
    }

    pub fn read_many(&mut self, addresses: &[u16]) -> Result<Vec<u8>> {
        addresses.iter().map(|a| self.read_eeprom(*a)).collect()
    }

    pub fn write_eeprom(&mut self, address: u16, value: u8, verify: bool) -> Result<()> {
        self.need_keys(true)?;
        let frame = write_frame(self.read_key.unwrap(), &self.write_key.unwrap(), address, value);
        let reply = self.transport.command(&frame)?;
        if !reply.is_some_and(|r| r.windows(4).any(|w| w == b":OK;")) {
            bail!(
                "a impressora recusou a gravação no endereço {address} (chave errada ou \
                 firmware bloqueado para gravação)"
            );
        }
        if verify {
            let got = self.read_eeprom(address)?;
            if got != value {
                bail!("gravação no endereço {address} não conferiu: lido {got}, esperado {value}");
            }
        }
        Ok(())
    }

    /// Lê um intervalo [start, end].
    pub fn dump(&mut self, start: u16, end: u16) -> Result<Vec<(u16, u8)>> {
        (start..=end).map(|a| Ok((a, self.read_eeprom(a)?))).collect()
    }

    // ---- almofada de tinta -------------------------------------------------

    fn config(&self) -> Result<Model> {
        self.model.clone().ok_or_else(|| Error::new("escolha o modelo primeiro"))
    }

    /// [("principal", %), ("sem margem", %)...] conforme o modelo.
    pub fn waste_levels(&mut self) -> Result<Vec<(String, f64)>> {
        let model = self.config()?;
        let mut levels = Vec::new();
        for (label, c) in model.counters() {
            let values = self.read_many(&c.oids)?;
            levels.push((label.to_string(), waste_percent(&values, c.divider)));
        }
        Ok(levels)
    }

    /// Endereços e valores que o reset vai gravar.
    pub fn reset_plan(&self) -> Result<Vec<(u16, u8)>> {
        let model = self.config()?;
        if !model.raw_waste_reset.is_empty() {
            return Ok(model.raw_waste_reset.clone());
        }
        let mut plan: Vec<(u16, u8)> = Vec::new();
        for (_, c) in model.counters() {
            for a in &c.oids {
                if !plan.iter().any(|(x, _)| x == a) {
                    plan.push((*a, 0));
                }
            }
        }
        Ok(plan)
    }

    pub fn backup(&mut self, addresses: &[u16]) -> Result<Backup> {
        let mut sorted: Vec<u16> = addresses.to_vec();
        sorted.sort();
        sorted.dedup();
        let mut eeprom = Vec::new();
        for a in sorted {
            eeprom.push((a, self.read_eeprom(a)?));
        }
        Ok(Backup { printer: self.host.clone(), model: self.model_name(), date: now_iso(), eeprom })
    }

    /// Faz backup (salvo em `backup_path`, se dado) e zera os contadores.
    pub fn reset_waste(&mut self, backup_path: Option<&str>) -> Result<Backup> {
        let plan = self.reset_plan()?;
        let commit = self.config()?.waste_reset_commit;
        let mut addresses: Vec<u16> = plan.iter().map(|(a, _)| *a).collect();
        addresses.extend(commit.iter().map(|(a, _)| *a));
        let saved = self.backup(&addresses)?;
        if let Some(path) = backup_path {
            saved.save(path)?;
        }
        for (a, v) in &plan {
            self.write_eeprom(*a, *v, true)?;
        }
        for (a, mask) in &commit {
            let current = saved.value(*a).unwrap_or(0);
            self.write_eeprom(*a, current & mask, true)?;
        }
        Ok(saved)
    }

    pub fn restore(&mut self, backup: &Backup) -> Result<()> {
        for (a, v) in &backup.eeprom {
            self.write_eeprom(*a, *v, true)?;
        }
        Ok(())
    }

    // ---- regravar a EEPROM a partir de arquivo -------------------------------

    /// Endereços que identificam esta unidade: série e MAC do Wi-Fi.
    pub fn protected_addresses(&self) -> Vec<u16> {
        let mut p = self.model.as_ref().map(|m| m.serial_range()).unwrap_or_default();
        p.extend(MAC_ADDRESSES);
        p
    }

    /// Compara o arquivo com a impressora: (mudanças [(end, atual, novo)], protegidos ignorados).
    pub fn plan_image(
        &mut self,
        image: &BTreeMap<u16, u8>,
        include_protected: bool,
    ) -> Result<(Vec<(u16, u8, u8)>, Vec<u16>)> {
        if image.len() > MAX_IMAGE {
            bail!("arquivo grande demais ({} endereços, máximo {MAX_IMAGE})", image.len());
        }
        let protected = if include_protected { Vec::new() } else { self.protected_addresses() };
        let (mut changes, mut skipped) = (Vec::new(), Vec::new());
        for (a, new) in image {
            if protected.contains(a) {
                skipped.push(*a);
                continue;
            }
            let current = self.read_eeprom(*a)?;
            if current != *new {
                changes.push((*a, current, *new));
            }
        }
        Ok((changes, skipped))
    }

    /// Grava as mudanças planejadas, salvando antes os valores atuais.
    pub fn write_image(&mut self, changes: &[(u16, u8, u8)], backup_path: Option<&str>) -> Result<Backup> {
        let saved = Backup {
            printer: self.host.clone(),
            model: self.model_name(),
            date: now_iso(),
            eeprom: changes.iter().map(|(a, old, _)| (*a, *old)).collect(),
        };
        if let Some(path) = backup_path {
            saved.save(path)?;
        }
        for (a, _, new) in changes {
            self.write_eeprom(*a, *new, true)?;
        }
        Ok(saved)
    }

    // ---- status e firmware bloqueado ----------------------------------------

    pub fn status(&mut self) -> Result<BTreeMap<u8, Vec<u8>>> {
        let reply = self.transport.command(&ctrl_frame(b"st", &[1]))?.unwrap_or_default();
        Ok(parse_status(&reply))
    }

    /// Estado, erro atual (com causa e correção) e avisos, pelo status ST2.
    pub fn diagnose(&mut self) -> Result<Diagnosis> {
        let st = self.status()?;
        if st.is_empty() {
            bail!("a impressora não devolveu o status");
        }
        let state = st.get(&0x01).and_then(|v| v.first().copied()).unwrap_or(0xFF);
        let mut d = Diagnosis {
            estado: epson_status(state),
            problemas: Vec::new(),
            avisos: st.get(&0x04).map(|w| w.iter().map(|c| epson_warning(*c)).collect()).unwrap_or_default(),
        };
        if let Some(code) = st.get(&0x02).and_then(|v| v.first()) {
            d.problemas.push(explain_epson_error(*code));
        }
        if let Some(b) = st.get(&0x37).filter(|b| b.len() >= 2 && matches!(b[1], 1 | 2)) {
            d.avisos.push(format!("Caixa de manutenção {}", if b[1] == 1 { "quase cheia" } else { "cheia" }));
        }
        Ok(d)
    }

    /// Número de série pelo status (funciona mesmo com a EEPROM bloqueada).
    pub fn serial_number(&mut self) -> Result<String> {
        let st = self.status()?;
        for kind in [0x1F, 0x40] {
            // o campo varia por modelo (0x40 na L3250)
            let serial = latin1(st.get(&kind).map(Vec::as_slice).unwrap_or_default());
            let serial = serial.trim_matches(|c| c == '\0' || c == ' ');
            if serial.len() == 10 && serial.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Ok(serial.to_string());
            }
        }
        bail!("não consegui ler o número de série; digite o da etiqueta")
    }

    /// Comando "rw": libera a impressora sem chave nem acesso à EEPROM.
    pub fn temporary_reset(&mut self, serial: Option<&str>) -> Result<String> {
        let serial = match serial {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => self.serial_number()?,
        };
        let mut payload = 1u16.to_le_bytes().to_vec();
        payload.extend(Sha1::digest(serial.as_bytes()));
        let reply = self.transport.command(&ctrl_frame(b"rw", &payload))?.unwrap_or_default();
        let has = |p: &[u8]| reply.windows(p.len()).any(|w| w == p);
        if !(has(b"rw:") && has(b":OK;")) {
            bail!("a impressora recusou o reset temporário (confira o número de série)");
        }
        Ok(serial)
    }
}
