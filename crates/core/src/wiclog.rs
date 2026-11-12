//! Aprende um modelo Epson a partir do log de um reset feito no WICReset.
//!
//! O WICReset registra em texto (application.log) os comandos EPSON-CTRL que
//! mandou, em hexadecimal ou como OID decimal. Um comando de gravação é:
//!
//! ```text
//! "||" 10 00  rk0 rk1  42 BD 21  end_lo end_hi valor  chave(8 bytes, +1)
//! ```
//!
//! Dele saem a chave de leitura, a chave de gravação e o que o reset grava.
//! Só lemos o texto do log; nada da licença do WICReset é usado.

use regex::Regex;

use crate::models::{Counter, Learned, Model};
use crate::{Error, Result, bail};

const WRITE_HEAD: &[u8] = b"||\x10\x00";
const READ_HEAD: &[u8] = b"||\x07\x00";
/// "Nível de manutenção" que os resets gravam.
pub const LEVEL: u8 = 94;
/// O reset trial deixa a almofada em 80%.
pub const TRIAL_PERCENT: f64 = 80.0;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    pub model: Option<String>,
    pub read_key: Option<[u8; 2]>,
    pub write_key: Option<[u8; 8]>,
    pub writes: Vec<(u16, u8)>,
    pub reads: Vec<u16>,
}

/// Toda sequência de bytes do texto, vinda de hexadecimal ou de OID decimal.
fn byte_runs(text: &str) -> Vec<Vec<u8>> {
    let hex = Regex::new(r"(?:\b[0-9A-Fa-f]{2}\b[ :,-]?){11,}").unwrap();
    let pair = Regex::new(r"[0-9A-Fa-f]{2}").unwrap();
    let dec = Regex::new(r"\b\d{1,3}(?:\.\d{1,3}){11,}\b").unwrap();
    let mut runs = Vec::new();
    for line in text.lines() {
        for m in hex.find_iter(line) {
            runs.push(pair.find_iter(m.as_str()).map(|p| u8::from_str_radix(p.as_str(), 16).unwrap()).collect());
        }
        for m in dec.find_iter(line) {
            // partes acima de 255 (o 1248 do OID da Epson) não são bytes do comando
            runs.push(m.as_str().split('.').map(|p| p.parse::<u16>().ok().filter(|n| *n < 256).unwrap_or(0) as u8).collect());
        }
    }
    runs
}

fn unshift(key: &[u8]) -> [u8; 8] {
    let mut out = [0u8; 8];
    for (i, b) in key.iter().take(8).enumerate() {
        out[i] = if *b == 0 { 0 } else { b - 1 };
    }
    out
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

pub fn parse(text: &str) -> Result<Parsed> {
    let mut p = Parsed::default();
    for run in byte_runs(text) {
        let mut pos = find(&run, WRITE_HEAD, 0);
        while let Some(at) = pos {
            if at + 20 > run.len() {
                break;
            }
            let cmd = &run[at + 4..at + 20];
            if cmd[2..5] == [0x42, 0xBD, 0x21] {
                let (rk, wk) = ([cmd[0], cmd[1]], unshift(&cmd[8..16]));
                // a chave de leitura pode ter vindo antes, das leituras do log
                if p.read_key.is_some_and(|k| k != rk) || p.write_key.is_some_and(|k| k != wk) {
                    bail!("o log tem resets de mais de uma impressora; use um log com uma só");
                }
                p.read_key = Some(rk);
                p.write_key = Some(wk);
                p.writes.push((u16::from_le_bytes([cmd[5], cmd[6]]), cmd[7]));
            }
            pos = find(&run, WRITE_HEAD, at + 1);
        }
        if let Some(at) = find(&run, READ_HEAD, 0)
            && at + 11 <= run.len() && run[at + 6..at + 9] == [0x41, 0xBE, 0xA0] {
                p.read_key = p.read_key.or(Some([run[at + 4], run[at + 5]]));
                let a = u16::from_le_bytes([run[at + 9], run[at + 10]]);
                if !p.reads.contains(&a) {
                    p.reads.push(a);
                }
            }
    }
    p.reads.sort();
    for re in [r"GUID:\s*(.+?)\s+\d+\s+KEY", r"(?:EPSON|Epson)\s+((?:L|ET-|XP-|WF-|EP-|PX-)[\w-]+)"] {
        if let Some(c) = Regex::new(re).unwrap().captures(text) {
            p.model = Some(c[1].trim().to_string());
            break;
        }
    }
    Ok(p)
}

/// Contadores de 2 bytes: endereços consecutivos tomados de dois em dois.
/// [20, 21, 22, 23, 40] -> [[20, 21], [22, 23]].
fn counter_pairs(addresses: &[u16]) -> Vec<Vec<u16>> {
    let mut sorted = addresses.to_vec();
    sorted.sort();
    let mut runs: Vec<Vec<u16>> = Vec::new();
    for a in sorted {
        match runs.last_mut() {
            Some(r) if *r.last().unwrap() + 1 == a => r.push(a),
            _ => runs.push(vec![a]),
        }
    }
    runs.iter().flat_map(|r| r.as_chunks::<2>().0.iter().map(|c| c.to_vec())).collect()
}

/// Modelo da lista cujo reset grava os mesmos endereços: (modelo, mesmas_chaves).
/// Vários modelos dividem os endereços com divisores diferentes, então só um com
/// as mesmas chaves é tomado como igual.
fn similar_model<'a>(addresses: &[u16], rk: [u8; 2], wk: [u8; 8], models: &'a [Model]) -> (Option<&'a Model>, bool) {
    let mut wanted: Vec<u16> = addresses.iter().copied().filter(|a| *a < 0x100).collect();
    wanted.sort();
    let mut found = None;
    for m in models.iter().filter(|m| m.learned.is_none()) {
        let mut theirs: Vec<u16> = m.raw_waste_reset.iter().map(|(a, _)| *a).filter(|a| *a < 0x100).collect();
        theirs.sort();
        if theirs != wanted {
            continue;
        }
        if m.read_key == rk && m.write_key == wk {
            return (Some(m), true);
        }
        found = found.or(Some(m));
    }
    (found, false)
}

fn divider(oids: &[u16], counters: &[(u16, u8)], percent: f64) -> f64 {
    let raw = oids
        .iter()
        .rev()
        .fold(0u64, |acc, a| (acc << 8) | counters.iter().find(|(x, _)| x == a).map(|(_, v)| *v as u64).unwrap_or(0));
    (raw as f64 / percent * 100.0).round() / 100.0
}

/// Monta o modelo. Num reset trial os contadores ficam em `percent`%: os bytes
/// gravados com valor diferente de 0 e de 94 são contadores e, no reset
/// completo, viram 0. Endereços a partir de 0x100 são flags de estado (variam
/// por unidade) e não entram no reset; se o modelo for parecido com um da
/// lista, vale o dele. Devolve (modelo, avisos).
pub fn learn(text: &str, name: Option<&str>, percent: f64, models: &[Model]) -> Result<(Model, Vec<String>)> {
    let info = parse(text)?;
    if info.writes.is_empty() {
        bail!("o log não tem nenhuma gravação de EEPROM: faça um reset (nem que seja o trial) no WICReset e use o log de depois");
    }
    let name = name.filter(|n| !n.trim().is_empty()).map(String::from).or(info.model.clone()).unwrap_or_default();
    let name = name.trim().to_uppercase();
    if name.is_empty() {
        bail!("não achei o nome do modelo no log; informe o nome");
    }
    // a última gravação de cada endereço vale (na posição da primeira)
    let mut written: Vec<(u16, u8)> = Vec::new();
    for (a, v) in &info.writes {
        match written.iter_mut().find(|(x, _)| x == a) {
            Some(slot) => slot.1 = *v,
            None => written.push((*a, *v)),
        }
    }
    let flags: Vec<u16> = { let mut f: Vec<u16> = written.iter().map(|(a, _)| *a).filter(|a| *a >= 0x100).collect(); f.sort(); f };
    let (mut reset, mut counters) = (Vec::new(), Vec::new());
    for (a, v) in &written {
        if *a >= 0x100 {
            continue; // flag de estado: o valor depende de cada unidade
        }
        if *v == 0 || *v == LEVEL {
            reset.push((*a, *v));
        } else {
            reset.push((*a, 0));
            counters.push((*a, *v));
        }
    }
    let rk = info.read_key.ok_or_else(|| Error::new("o log não tem a chave de leitura"))?;
    let wk = info.write_key.unwrap();
    let mut m = Model {
        name,
        aliases: Vec::new(),
        read_key: rk,
        write_key: wk,
        serial_number: None,
        main_waste: None,
        borderless_waste: None,
        third_waste: None,
        raw_waste_reset: reset.clone(),
        waste_reset_commit: Vec::new(),
        may_be_locked: false,
        learned: Some(Learned { from: "log do WICReset".into(), ..Default::default() }),
    };
    let mut warnings = Vec::new();
    let addresses: Vec<u16> = reset.iter().map(|(a, _)| *a).collect();
    let (similar, same_keys) = similar_model(&addresses, rk, wk, models);
    if let Some(base) = similar {
        m.serial_number = base.serial_number;
        m.waste_reset_commit = base.waste_reset_commit.clone();
        m.main_waste = base.main_waste.clone();
        m.borderless_waste = base.borderless_waste.clone();
        m.third_waste = base.third_waste.clone();
        if same_keys {
            warnings.push(format!("mesmas chaves e endereços do {}: contadores lidos como nele", base.name));
        } else {
            warnings.push(format!("endereços iguais aos do {}, chaves diferentes", base.name));
            for c in [&mut m.main_waste, &mut m.borderless_waste, &mut m.third_waste].into_iter().flatten() {
                // divisor deste modelo, pelo valor do trial
                if c.oids.iter().any(|a| counters.iter().any(|(x, _)| x == a)) {
                    c.divider = divider(&c.oids, &counters, percent);
                }
            }
        }
    } else {
        let new_layout = addresses.iter().max().copied().unwrap_or(0) > 40; // série em 1604
        m.serial_number = Some(if new_layout { (1604, 1614) } else { (192, 202) });
        let addrs: Vec<u16> = counters.iter().map(|(a, _)| *a).collect();
        let found: Vec<Vec<u16>> = counter_pairs(&addrs).into_iter().take(3).collect();
        let slots = [&mut m.main_waste, &mut m.borderless_waste, &mut m.third_waste];
        for (slot, group) in slots.into_iter().zip(found.iter()) {
            *slot = Some(Counter { divider: divider(group, &counters, percent), oids: group.clone() });
        }
        if found.is_empty() {
            warnings.push("não deu para saber onde ficam os contadores; o reset funciona, mas a leitura da almofada não".into());
        }
    }
    if !counters.is_empty() && !m.counters().is_empty() && !same_keys {
        warnings.push(format!("divisores estimados pelo reset trial ({percent:.0}%): confira a leitura da almofada"));
    }
    if !flags.is_empty() && m.waste_reset_commit.is_empty() {
        let list: Vec<String> = flags.iter().map(|a| format!("0x{a:X}")).collect();
        warnings.push(format!(
            "o WICReset também gravou {} (flags de estado), que ficaram de fora; se o erro não sair, compare leituras antes/depois",
            list.join(", ")
        ));
    }
    if counters.is_empty() {
        warnings.push("o log é de um reset completo: será repetido igual".into());
    }
    Ok((m, warnings))
}

pub fn learn_file(path: &str, name: Option<&str>, percent: f64, models: &[Model]) -> Result<(Model, Vec<String>)> {
    let text = String::from_utf8_lossy(&std::fs::read(path)?).into_owned();
    learn(&text, name, percent, models)
}
