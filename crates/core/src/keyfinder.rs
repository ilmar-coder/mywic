//! Descobrir as chaves de uma Epson que não está na lista de modelos.
//!
//! Chave de leitura: 2 bytes (65.536 combinações). Testamos lendo um endereço,
//! o que é inofensivo: a chave errada só recebe "NA". Primeiro as chaves dos
//! modelos conhecidos, depois as com o segundo byte baixo, por fim o resto.
//! Chave de gravação: 8 bytes, então só testamos as conhecidas, gravando no
//! endereço o MESMO valor que já está lá (não muda nada).

use std::sync::atomic::{AtomicBool, Ordering};

use crate::models::Model;
use crate::printer::EpsonPrinter;
use crate::{Result, bail};

pub const TOTAL_KEYS: usize = 256 * 256;

pub fn known_read_keys(models: &[Model]) -> Vec<[u8; 2]> {
    let mut keys: Vec<[u8; 2]> = Vec::new();
    for m in models {
        if !keys.contains(&m.read_key) {
            keys.push(m.read_key);
        }
    }
    keys
}

pub fn known_write_keys(models: &[Model]) -> Vec<[u8; 8]> {
    let mut keys: Vec<[u8; 8]> = Vec::new();
    for m in models {
        if !keys.contains(&m.write_key) {
            keys.push(m.write_key);
        }
    }
    keys
}

/// Todas as 65.536 chaves, as mais prováveis primeiro, sem repetir.
pub fn candidate_keys(models: &[Model]) -> Vec<[u8; 2]> {
    let known = known_read_keys(models);
    let mut keys = known.clone();
    for second in 0..=255u8 {
        for first in 0..=255u8 {
            if !known.contains(&[first, second]) {
                keys.push([first, second]);
            }
        }
    }
    keys
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyResult {
    pub read_key: [u8; 2],
    pub tries: usize,
    /// A chave também funcionou num segundo endereço.
    pub confirmed: bool,
    /// O firmware aceitou uma chave certamente errada: não confere a chave.
    pub accepts_any: bool,
    pub known: bool,
}

fn accepts(p: &mut EpsonPrinter, key: [u8; 2], address: u16) -> Result<bool> {
    p.read_key = Some(key);
    match p.read_eeprom(address) {
        Ok(_) => Ok(true),
        Err(e) if e.message.contains("recusou") || e.message.contains("resposta") => Ok(false),
        Err(e) => Err(e), // impressora sumiu: interrompe
    }
}

/// Procura a chave de leitura. Ok(None) se cancelado.
/// progress(tentativas, total, chave) é chamado a cada tentativa.
pub fn find_read_key(
    p: &mut EpsonPrinter,
    models: &[Model],
    address: u16,
    progress: &mut dyn FnMut(usize, usize, [u8; 2]),
    cancel: &AtomicBool,
) -> Result<Option<KeyResult>> {
    let original = p.read_key;
    let known = known_read_keys(models);
    for (i, key) in candidate_keys(models).into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            p.read_key = original;
            return Ok(None);
        }
        progress(i + 1, TOTAL_KEYS, key);
        let ok = match accepts(p, key, address) {
            Ok(ok) => ok,
            Err(e) => {
                p.read_key = original;
                return Err(e);
            }
        };
        if !ok {
            continue;
        }
        // confere com outro endereço e com uma chave certamente diferente
        let confirmed = accepts(p, key, address + 1).unwrap_or(false);
        let accepts_any = accepts(p, [key[0].wrapping_add(1), key[1]], address).unwrap_or(false);
        p.read_key = Some(key);
        return Ok(Some(KeyResult { read_key: key, tries: i + 1, confirmed, accepts_any, known: known.contains(&key) }));
    }
    p.read_key = original;
    bail!("nenhuma chave funcionou. Se for pela rede, o firmware pode estar bloqueado: tente pelo cabo USB.")
}

/// Testa as chaves de gravação conhecidas gravando o valor atual de volta.
pub fn find_write_key(p: &mut EpsonPrinter, models: &[Model], address: u16) -> Result<Option<[u8; 8]>> {
    if p.read_key.is_none() {
        bail!("descubra a chave de leitura primeiro");
    }
    let value = p.read_eeprom(address)?;
    for key in known_write_keys(models) {
        p.write_key = Some(key);
        if p.write_eeprom(address, value, true).is_ok() {
            return Ok(Some(key));
        }
    }
    p.write_key = None;
    Ok(None)
}
