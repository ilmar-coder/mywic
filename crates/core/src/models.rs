//! Parâmetros por modelo: chaves e endereços de EEPROM.
//!
//! Os da lista são fatos de hardware publicados pela comunidade (projeto
//! epson_print_conf). Cada contador é uma lista de endereços do byte menos para
//! o mais significativo; o valor bruto dividido por `divider` dá a porcentagem.
//! Os aprendidos (log do WICReset) vêm do banco (library.rs) e valem antes.

use serde_json::{Map, Value, json};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Counter {
    pub oids: Vec<u16>,
    pub divider: f64,
}

/// De onde veio um modelo aprendido.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Learned {
    pub from: String,
    pub at: String,
    pub on: String,
    /// Salvo sem conexão: ainda não foi para o servidor.
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub name: String,
    pub aliases: Vec<String>,
    pub read_key: [u8; 2],
    pub write_key: [u8; 8],
    /// Endereços [início, fim) do número de série.
    pub serial_number: Option<(u16, u16)>,
    pub main_waste: Option<Counter>,
    pub borderless_waste: Option<Counter>,
    pub third_waste: Option<Counter>,
    /// O que o reset grava, na ordem.
    pub raw_waste_reset: Vec<(u16, u8)>,
    /// Depois de zerar: endereço e máscara aplicada ao valor atual.
    pub waste_reset_commit: Vec<(u16, u8)>,
    pub may_be_locked: bool,
    pub learned: Option<Learned>,
}

impl Model {
    pub fn counters(&self) -> Vec<(&'static str, &Counter)> {
        let mut out = Vec::new();
        for (label, c) in [
            ("principal", &self.main_waste),
            ("sem margem", &self.borderless_waste),
            ("terceira", &self.third_waste),
        ] {
            if let Some(c) = c {
                out.push((label, c));
            }
        }
        out
    }

    pub fn serial_range(&self) -> Vec<u16> {
        self.serial_number.map(|(a, b)| (a..b).collect()).unwrap_or_default()
    }

    pub fn write_key_text(&self) -> String {
        String::from_utf8_lossy(&self.write_key).into_owned()
    }

    /// Formato JSON do banco (o mesmo da versão Python e do servidor).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("aliases".into(), json!(self.aliases));
        m.insert("read_key".into(), json!(self.read_key));
        m.insert("write_key".into(), json!(latin1(&self.write_key)));
        if let Some((a, b)) = self.serial_number {
            m.insert("serial_number".into(), json!([a, b]));
        }
        for (key, c) in [
            ("main_waste", &self.main_waste),
            ("borderless_waste", &self.borderless_waste),
            ("third_waste", &self.third_waste),
        ] {
            if let Some(c) = c {
                m.insert(key.into(), json!({"oids": c.oids, "divider": c.divider}));
            }
        }
        m.insert("raw_waste_reset".into(), pairs_json(&self.raw_waste_reset));
        if !self.waste_reset_commit.is_empty() {
            m.insert("waste_reset_commit".into(), pairs_json(&self.waste_reset_commit));
        }
        if self.may_be_locked {
            m.insert("may_be_locked".into(), json!(true));
        }
        if let Some(l) = &self.learned {
            m.insert("learned_from".into(), json!(l.from));
            if !l.at.is_empty() {
                m.insert("learned_at".into(), json!(l.at));
            }
            if !l.on.is_empty() {
                m.insert("learned_on".into(), json!(l.on));
            }
            if l.pending {
                m.insert("pendente".into(), json!(true));
            }
        }
        Value::Object(m)
    }

    pub fn from_json(name: &str, v: &Value) -> Result<Model> {
        let bad = |what: &str| Error::new(format!("modelo {name} no banco: {what} inválido"));
        let rk = v["read_key"].as_array().filter(|a| a.len() == 2).ok_or_else(|| bad("read_key"))?;
        let byte = |x: &Value| x.as_u64().filter(|n| *n < 256).map(|n| n as u8);
        let read_key = [byte(&rk[0]).ok_or_else(|| bad("read_key"))?, byte(&rk[1]).ok_or_else(|| bad("read_key"))?];
        let wk: Vec<u8> = v["write_key"].as_str().ok_or_else(|| bad("write_key"))?.chars().map(|c| c as u32 as u8).collect();
        let write_key: [u8; 8] = wk.try_into().map_err(|_| bad("write_key"))?;
        let counter = |key: &str| -> Option<Counter> {
            let c = v.get(key)?;
            Some(Counter {
                oids: c["oids"].as_array()?.iter().filter_map(|o| o.as_u64().map(|n| n as u16)).collect(),
                divider: c["divider"].as_f64()?,
            })
        };
        let learned = v.get("learned_from").and_then(|f| f.as_str()).map(|from| Learned {
            from: from.into(),
            at: v["learned_at"].as_str().unwrap_or_default().into(),
            on: v["learned_on"].as_str().unwrap_or_default().into(),
            pending: v["pendente"].as_bool().unwrap_or(false),
        });
        Ok(Model {
            name: name.into(),
            aliases: v["aliases"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            read_key,
            write_key,
            serial_number: v["serial_number"].as_array().and_then(|a| {
                Some((a.first()?.as_u64()? as u16, a.get(1)?.as_u64()? as u16))
            }),
            main_waste: counter("main_waste"),
            borderless_waste: counter("borderless_waste"),
            third_waste: counter("third_waste"),
            raw_waste_reset: pairs_from(&v["raw_waste_reset"]).ok_or_else(|| bad("raw_waste_reset"))?,
            waste_reset_commit: pairs_from(&v["waste_reset_commit"]).unwrap_or_default(),
            may_be_locked: v["may_be_locked"].as_bool().unwrap_or(false),
            learned,
        })
    }
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|b| *b as char).collect()
}

fn pairs_json(pairs: &[(u16, u8)]) -> Value {
    let mut m = Map::new();
    for (a, v) in pairs {
        m.insert(a.to_string(), json!(v));
    }
    Value::Object(m)
}

fn pairs_from(v: &Value) -> Option<Vec<(u16, u8)>> {
    let obj = v.as_object()?;
    obj.iter()
        .map(|(a, x)| Some((a.parse::<u16>().ok()?, x.as_u64().filter(|n| *n < 256)? as u8)))
        .collect()
}

pub fn builtin() -> Vec<Model> {
    crate::models_data::builtin()
}

/// Lista da lista + aprendidos (um aprendido com o mesmo nome toma o lugar).
pub fn merge(learned: Vec<Model>) -> Vec<Model> {
    let mut all = builtin();
    for m in learned {
        match all.iter_mut().find(|x| x.name == m.name) {
            Some(slot) => *slot = m,
            None => all.push(m),
        }
    }
    all
}

pub fn names(models: &[Model]) -> Vec<String> {
    let mut n: Vec<String> = models.iter().map(|m| m.name.clone()).collect();
    n.sort();
    n
}

/// Acha o modelo num texto como "EPSON L3150 Series". Os aprendidos vêm antes,
/// para valerem sobre um apelido da lista.
pub fn find_model<'a>(text: &str, models: &'a [Model]) -> Option<&'a Model> {
    let tokens = normalize(text);
    let mut ordered: Vec<&Model> = models.iter().filter(|m| m.learned.is_some()).collect();
    ordered.extend(models.iter().filter(|m| m.learned.is_none()));
    ordered.into_iter().find(|m| {
        std::iter::once(&m.name).chain(m.aliases.iter()).any(|c| {
            // nomes aprendidos podem ter espaço ("L3250 SERIES"): compara as palavras
            let want = normalize(c);
            !want.is_empty() && tokens.windows(want.len()).any(|w| w == want.as_slice())
        })
    })
}

fn normalize(text: &str) -> Vec<String> {
    text.to_uppercase()
        .replace("EPSON", " ")
        .replace("SERIES", " ")
        .split_whitespace()
        .map(String::from)
        .collect()
}

pub fn by_name<'a>(name: &str, models: &'a [Model]) -> Option<&'a Model> {
    models.iter().find(|m| m.name == name)
}
