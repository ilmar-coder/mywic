//! Banco de modelos no servidor (o site no Raspberry): Atualizar e Enviar.
//!
//! ```text
//! GET    {url}/api/ecoreset/models          -> {"modelos": {nome: modelo}}
//! PUT    {url}/api/ecoreset/models/{nome}   <- modelo
//! DELETE {url}/api/ecoreset/models/{nome}
//! ```
//!
//! com a chave no cabeçalho X-EcoReset-Token. O banco local é a cópia do
//! servidor; o que foi aprendido sem conexão fica "pendente" e sobe depois.

use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::models::Model;
use crate::{Error, Result, bail, library};

pub const DEFAULT_URL: &str = "https://download.ilmar.com.br";

/// (url, chave); chave vazia = servidor não configurado.
pub fn settings() -> (String, String) {
    let cfg = library::read_config();
    let url = cfg.get("servidor").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or(DEFAULT_URL);
    let token = cfg.get("token").and_then(|v| v.as_str()).unwrap_or_default();
    (url.trim_end_matches('/').to_string(), token.to_string())
}

pub fn configured() -> bool {
    !settings().1.is_empty()
}

pub fn configure(url: &str, token: &str) -> Result<()> {
    let url = if url.trim().is_empty() { DEFAULT_URL } else { url.trim() }.trim_end_matches('/');
    if !(url.starts_with("https://") || url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1")) {
        bail!("use um endereço https:// (a chave não pode ir sem criptografia)");
    }
    library::write_config(&[("servidor", json!(url)), ("token", json!(token.trim()))])
}

fn agent() -> ureq::Agent {
    // TLS do próprio sistema: usa os certificados do Windows, do Mac e do Linux
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        // os certificados do próprio sistema (com a lista embutida do ureq o Windows recusa)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .user_agent(concat!("EcoReset/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn request(method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
    let (url, token) = settings();
    if token.is_empty() {
        bail!("servidor não configurado: informe o endereço e a chave");
    }
    let full = format!("{url}{path}");
    let a = agent();
    let result = match (method, body) {
        ("GET", _) => a.get(&full).header("X-EcoReset-Token", &token).header("Accept", "application/json").call(),
        ("DELETE", _) => a.delete(&full).header("X-EcoReset-Token", &token).call(),
        (_, Some(b)) => a.put(&full).header("X-EcoReset-Token", &token).send_json(b),
        _ => bail!("pedido inválido"),
    };
    let mut resp = result.map_err(|e| Error::new(format!("sem conexão com o servidor ({e})")))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let data: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status == 404 && method == "DELETE" {
        return Ok(Value::Null);
    }
    if status != 200 {
        let msg = data["error"].as_str().unwrap_or_default();
        bail!("servidor respondeu {status} {msg}");
    }
    if data.is_null() {
        bail!("resposta inválida do servidor");
    }
    Ok(data)
}

fn model_path(name: &str) -> String {
    let encoded: String = name
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("/api/ecoreset/models/{encoded}")
}

pub fn fetch() -> Result<Map<String, Value>> {
    Ok(request("GET", "/api/ecoreset/models", None)?["modelos"].as_object().cloned().unwrap_or_default())
}

pub fn push(model: &Model) -> Result<()> {
    let mut body = model.to_json();
    body.as_object_mut().unwrap().remove("pendente");
    request("PUT", &model_path(&model.name), Some(&body)).map(|_| ())
}

/// Salva no banco local e envia ao servidor. Devolve None ou o motivo de não ter ido.
pub fn publish(model: &Model) -> Result<Option<String>> {
    let online = configured();
    library::save(model, None, online)?;
    if !online {
        return Ok(None);
    }
    match push(model) {
        Ok(()) => {
            library::mark_sent(&model.name)?;
            Ok(None)
        }
        Err(e) => Ok(Some(format!("{e}; vai no próximo Atualizar"))),
    }
}

pub fn remove(name: &str) -> Result<bool> {
    if configured() {
        request("DELETE", &model_path(name), None)?;
    }
    library::remove(name, None)
}

/// Envia os pendentes e baixa o banco do servidor. (enviados, total, novos)
pub fn sync() -> Result<(usize, usize, Vec<String>)> {
    let local = library::load()?;
    let pending: Vec<&Model> = local.iter().filter(|m| m.learned.as_ref().is_some_and(|l| l.pending)).collect();
    for m in &pending {
        push(m)?;
    }
    let remote = fetch()?;
    let mut new: Vec<String> = remote.keys().filter(|n| !local.iter().any(|m| &&m.name == n)).cloned().collect();
    new.sort();
    let total = remote.len();
    library::replace_all(remote)?;
    Ok((pending.len(), total, new))
}
