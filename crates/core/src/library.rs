//! Banco de modelos aprendidos (modelos.json na pasta do EcoReset).
//!
//! Com servidor configurado (server.rs), este arquivo é a cópia local do banco
//! do servidor; modelos com "pendente" ainda não foram enviados. Sem servidor,
//! a pasta pode ser sincronizada (Drive, OneDrive, Dropbox) entre máquinas.
//!
//! Pasta, por ordem: variável ECORESET_PASTA; a escolhida (guardada em
//! ~/.ecoreset.json); senão ~/EcoReset.

use std::path::PathBuf;

use serde_json::{Map, Value, json};

use crate::models::Model;
use crate::{Error, Result};

pub const DB_NAME: &str = "modelos.json";
pub const DB_TYPE: &str = "ecoreset-modelos";

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_path() -> PathBuf {
    std::env::var_os("ECORESET_CONFIG").map(PathBuf::from).unwrap_or_else(|| home().join(".ecoreset.json"))
}

pub fn read_config() -> Map<String, Value> {
    std::fs::read(config_path())
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Atualiza ~/.ecoreset.json mantendo o resto. Só o usuário lê (tem a chave).
pub fn write_config(changes: &[(&str, Value)]) -> Result<()> {
    let mut data = read_config();
    for (k, v) in changes {
        data.insert(k.to_string(), v.clone());
    }
    let path = config_path();
    std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(data))?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn folder() -> PathBuf {
    if let Some(env) = std::env::var_os("ECORESET_PASTA") {
        return PathBuf::from(env);
    }
    read_config()
        .get("pasta")
        .and_then(|p| p.as_str())
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("EcoReset"))
}

pub fn set_folder(path: &str) -> Result<PathBuf> {
    let path = std::path::absolute(path)?;
    std::fs::create_dir_all(&path)?;
    write_config(&[("pasta", json!(path.to_string_lossy()))])?;
    Ok(path)
}

pub fn db_path() -> PathBuf {
    folder().join(DB_NAME)
}

/// Entradas cruas do banco (formato JSON do servidor).
pub fn load_raw(path: &PathBuf) -> Result<Map<String, Value>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(e.into()),
    };
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|e| Error::new(format!("banco de modelos corrompido ({}): {e}", path.display())))?;
    Ok(data["modelos"].as_object().cloned().unwrap_or_default())
}

pub fn load_from(path: &PathBuf) -> Result<Vec<Model>> {
    load_raw(path)?.iter().map(|(n, v)| Model::from_json(n, v)).collect()
}

pub fn load() -> Result<Vec<Model>> {
    load_from(&db_path())
}

fn write(models: &[Model], path: &PathBuf) -> Result<()> {
    let mut sorted: Vec<&Model> = models.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let mut entries = Map::new();
    for m in sorted {
        entries.insert(m.name.clone(), m.to_json());
    }
    write_raw(entries, path)
}

fn write_raw(entries: Map<String, Value>, path: &PathBuf) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let data = json!({"type": DB_TYPE, "modelos": entries});
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(&data)?)?;
    std::fs::rename(&tmp, path)?; // troca de uma vez: quem lê nunca vê pela metade
    Ok(())
}

/// Grava (ou atualiza) um modelo, juntando com o que já está no banco.
pub fn save(model: &Model, path: Option<&PathBuf>, pending: bool) -> Result<PathBuf> {
    let path = path.cloned().unwrap_or_else(db_path);
    let mut models = load_from(&path)?;
    let mut m = model.clone();
    let learned = m.learned.get_or_insert_with(Default::default);
    if learned.from.is_empty() {
        learned.from = "EcoReset".into();
    }
    learned.at = crate::printer::now_iso();
    learned.on = gethostname::gethostname().to_string_lossy().into_owned();
    learned.pending = pending;
    models.retain(|x| x.name != m.name);
    models.push(m);
    write(&models, &path)?;
    Ok(path)
}

pub fn mark_sent(name: &str) -> Result<()> {
    let path = db_path();
    let mut models = load_from(&path)?;
    let mut changed = false;
    for m in models.iter_mut().filter(|m| m.name == name) {
        if let Some(l) = m.learned.as_mut().filter(|l| l.pending) {
            l.pending = false;
            changed = true;
        }
    }
    if changed {
        write(&models, &path)?;
    }
    Ok(())
}

/// Troca o banco local por estas entradas (no formato JSON do servidor).
pub fn replace_all(entries: Map<String, Value>) -> Result<()> {
    for (n, v) in &entries {
        Model::from_json(n, v)?; // não grava nada inválido
    }
    write_raw(entries, &db_path())
}

pub fn remove(name: &str, path: Option<&PathBuf>) -> Result<bool> {
    let path = path.cloned().unwrap_or_else(db_path);
    let mut models = load_from(&path)?;
    let before = models.len();
    models.retain(|m| m.name != name);
    if models.len() == before {
        return Ok(false);
    }
    write(&models, &path)?;
    Ok(true)
}

/// Junta outro banco (ex.: copiado de outra máquina). Devolve os nomes novos.
/// Na dúvida vale o aprendido mais recente; os que vierem sobem no próximo Atualizar.
pub fn merge_file(other: &str) -> Result<Vec<String>> {
    let path = db_path();
    let mut models = load_from(&path)?;
    let incoming = load_from(&PathBuf::from(other))?;
    let mut new = Vec::new();
    for mut m in incoming {
        let at = |x: &Model| x.learned.as_ref().map(|l| l.at.clone()).unwrap_or_default();
        match models.iter().position(|x| x.name == m.name) {
            Some(i) if at(&m) < at(&models[i]) => continue,
            Some(i) => {
                models.remove(i);
            }
            None => new.push(m.name.clone()),
        }
        m.learned.get_or_insert_with(Default::default).pending = true;
        models.push(m);
    }
    write(&models, &path)?;
    new.sort();
    Ok(new)
}
