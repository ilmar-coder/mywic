//! Leitura completa da EEPROM e comparação entre duas leituras.
//!
//! Para descobrir o reset de um modelo fora da lista: salve uma leitura antes,
//! zere a impressora com outra ferramenta, salve outra e compare.

use serde_json::{Map, Value, json};

use crate::printer::{EpsonPrinter, MAC_ADDRESSES, now_iso};
use crate::{Error, Result, bail};

pub const SNAPSHOT_TYPE: &str = "ecoreset-leitura";
pub const DEFAULT_END: u16 = 0x7FF;
/// Valor de "nível de manutenção" que os resets costumam gravar.
pub const WASTE_LEVEL: u8 = 94;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub label: String,
    pub printer: String,
    pub model: Option<String>,
    pub model_name: String,
    pub serial: String,
    pub read_key: Vec<u8>,
    pub date: String,
    pub range: (u16, u16),
    pub eeprom: Vec<(u16, u8)>,
    pub refused: Vec<u16>,
}

impl Snapshot {
    pub fn to_json(&self) -> Value {
        let mut cells = Map::new();
        for (a, v) in &self.eeprom {
            cells.insert(a.to_string(), json!(v));
        }
        json!({
            "type": SNAPSHOT_TYPE, "label": self.label, "printer": self.printer,
            "model": self.model, "model_name": self.model_name, "serial": self.serial,
            "read_key": self.read_key, "date": self.date, "range": [self.range.0, self.range.1],
            "eeprom": cells, "sem_leitura": self.refused,
        })
    }

    pub fn from_json(v: &Value) -> Result<Snapshot> {
        let cells = v["eeprom"].as_object().ok_or_else(|| Error::new("não é uma leitura do EcoReset"))?;
        let mut eeprom = Vec::new();
        for (a, x) in cells {
            let a = a.parse::<u16>().map_err(|_| Error::new("leitura inválida"))?;
            eeprom.push((a, x.as_u64().unwrap_or(0) as u8));
        }
        eeprom.sort();
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        Ok(Snapshot {
            label: s("label"),
            printer: s("printer"),
            model: v["model"].as_str().map(String::from),
            model_name: s("model_name"),
            serial: s("serial"),
            read_key: v["read_key"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).map(|n| n as u8).collect()).unwrap_or_default(),
            date: s("date"),
            range: (v["range"][0].as_u64().unwrap_or(0) as u16, v["range"][1].as_u64().unwrap_or(0) as u16),
            eeprom,
            refused: v["sem_leitura"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).map(|n| n as u16).collect()).unwrap_or_default(),
        })
    }

    pub fn save(&self, path: &str) -> Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(&self.to_json())?)?;
        Ok(())
    }

    pub fn load(path: &str) -> Result<Snapshot> {
        let v: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        Snapshot::from_json(&v).map_err(|_| Error::new(format!("{path} não é uma leitura do EcoReset")))
    }
}

/// Lê [start, end]. Endereços recusados ficam em `refused`; se os 16 primeiros
/// forem todos recusados, a chave está errada e a leitura para.
pub fn take(
    printer: &mut EpsonPrinter,
    start: u16,
    end: u16,
    label: &str,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Snapshot> {
    if end < start {
        bail!("intervalo inválido");
    }
    let total = (end - start) as usize + 1;
    let (mut cells, mut refused) = (Vec::new(), Vec::new());
    for (i, address) in (start..=end).enumerate() {
        match printer.read_eeprom(address) {
            Ok(v) => cells.push((address, v)),
            Err(e) if e.message.contains("recusou") || e.message.contains("respond") => {
                refused.push(address);
                if i == 15 && cells.is_empty() {
                    bail!(
                        "a impressora recusou os 16 primeiros endereços: a chave de leitura está \
                         errada ou o acesso está bloqueado (tente pelo cabo USB)"
                    );
                }
            }
            Err(e) => return Err(e),
        }
        progress(i + 1, total);
    }
    Ok(Snapshot {
        label: label.into(),
        printer: printer.host.clone(),
        model: printer.model_name(),
        model_name: printer.model_string().unwrap_or_default(),
        serial: printer.serial_number().unwrap_or_default(),
        read_key: printer.read_key.map(|k| k.to_vec()).unwrap_or_default(),
        date: now_iso(),
        range: (start, end),
        eeprom: cells,
        refused,
    })
}

/// (mudanças [(endereço, antes, depois)], avisos)
pub fn compare(before: &Snapshot, after: &Snapshot) -> (Vec<(u16, u8, u8)>, Vec<String>) {
    let mut warnings = Vec::new();
    if !before.serial.is_empty() && !after.serial.is_empty() && before.serial != after.serial {
        warnings.push(format!("As leituras são de impressoras diferentes (série {} x {}).", before.serial, after.serial));
    }
    if !before.model_name.is_empty() && !after.model_name.is_empty() && before.model_name != after.model_name {
        warnings.push(format!("Modelos diferentes: {} x {}.", before.model_name, after.model_name));
    }
    let b: std::collections::BTreeMap<u16, u8> = after.eeprom.iter().copied().collect();
    let common: Vec<(u16, u8, u8)> = before.eeprom.iter().filter_map(|(a, x)| b.get(a).map(|y| (*a, *x, *y))).collect();
    let largest = before.eeprom.len().max(after.eeprom.len());
    if common.len() < largest {
        warnings.push(format!("{} endereço(s) só existem numa das leituras e foram ignorados.", largest - common.len()));
    }
    (common.into_iter().filter(|(_, x, y)| x != y).collect(), warnings)
}

/// Palpite sobre o papel de um endereço que mudou num reset.
pub fn classify(address: u16, old: u8, new: u8) -> &'static str {
    if MAC_ADDRESSES.contains(&address) {
        "MAC (não deveria mudar)"
    } else if new == 0 && old != 0 {
        "provável contador zerado"
    } else if new == WASTE_LEVEL {
        "provável nível de manutenção (94)"
    } else if new < old {
        "diminuiu"
    } else {
        "aumentou (pode ser uso normal, não reset)"
    }
}

/// Entrada de modelo sugerida: só o que tem cara de reset (zerou ou foi para 94),
/// fora série e MAC.
pub fn suggest_model(before: &Snapshot, after: &Snapshot, changes: &[(u16, u8, u8)], serial_range: &[u16]) -> Value {
    let skip = |a: &u16| serial_range.contains(a) || MAC_ADDRESSES.contains(a);
    let mut reset = Map::new();
    let mut ignored = Vec::new();
    for (a, _, new) in changes {
        if !skip(a) && (*new == 0 || *new == WASTE_LEVEL) {
            reset.insert(a.to_string(), json!(new));
        } else {
            ignored.push(*a);
        }
    }
    let name = [&after.model_name, &before.model_name].into_iter().find(|n| !n.is_empty()).cloned().unwrap_or("?".into());
    let key = if after.read_key.is_empty() { &before.read_key } else { &after.read_key };
    json!({
        "modelo": name, "read_key": key, "raw_waste_reset": reset, "ignorados": ignored,
        "origem": format!("comparação de leituras do EcoReset ({} -> {})", before.date, after.date),
    })
}
