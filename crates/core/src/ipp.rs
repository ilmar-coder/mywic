//! Cliente IPP mínimo (Get-Printer-Attributes) para ler estado e alertas.
//!
//! Funciona com qualquer impressora AirPrint/IPP Everywhere, pela rede ou pelo
//! USB quando o sistema expõe o IPP-over-USB (ippusb no Mac).

use std::collections::BTreeMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use regex::Regex;

use crate::{Error, Result, bail};

const OPERATION_ATTRS: u8 = 0x01;
const END: u8 = 0x03;
const CHARSET: u8 = 0x47;
const LANGUAGE: u8 = 0x48;
const URI: u8 = 0x45;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i32),
    Bool(bool),
    Text(String),
    Raw(Vec<u8>),
}

impl Value {
    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn int(&self) -> Option<i32> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }
}

pub type Attrs = BTreeMap<String, Vec<Value>>;

/// Motivos padrão do IPP (RFC 8011 / PWG 5100) mais comuns, em português.
pub fn reason_text(key: &str) -> Option<&'static str> {
    Some(match key {
        "none" => "Sem problemas",
        "media-empty" => "Sem papel",
        "media-needed" => "Coloque papel",
        "media-jam" => "Papel enroscado",
        "media-low" => "Papel acabando",
        "cover-open" => "Tampa aberta",
        "door-open" => "Porta/tampa aberta",
        "input-tray-missing" => "Bandeja ausente",
        "marker-supply-low" => "Tinta baixa",
        "marker-supply-empty" => "Sem tinta",
        "marker-waste-almost-full" => "Almofada/caixa de manutenção quase cheia",
        "marker-waste-full" => "Almofada/caixa de manutenção cheia",
        "toner-low" => "Tinta/toner baixo",
        "toner-empty" => "Sem tinta/toner",
        "offline-report" => "Impressora desconectada ou desligada",
        "paused" => "Pausada",
        "moving-to-paused" => "Pausando",
        "shutdown" => "Desligando",
        "connecting-to-device" => "Conectando",
        "timed-out" => "Sem resposta",
        "stopped-partly" => "Parcialmente parada",
        "spool-area-full" => "Fila cheia",
        "interpreter-resource-unavailable" => "Recurso interno indisponível",
        "other" => "Outro problema (veja a mensagem)",
        _ => return None,
    })
}

fn attr(tag: u8, name: &str, value: &str) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend((name.len() as u16).to_be_bytes());
    out.extend(name.as_bytes());
    out.extend((value.len() as u16).to_be_bytes());
    out.extend(value.as_bytes());
    out
}

pub fn build_request(printer_uri: &str, request_id: u32) -> Vec<u8> {
    let mut out = vec![1, 1];
    out.extend(0x000Bu16.to_be_bytes());
    out.extend(request_id.to_be_bytes());
    out.push(OPERATION_ATTRS);
    out.extend(attr(CHARSET, "attributes-charset", "utf-8"));
    out.extend(attr(LANGUAGE, "attributes-natural-language", "en"));
    out.extend(attr(URI, "printer-uri", printer_uri));
    out.push(END);
    out
}

fn decode_value(tag: u8, raw: &[u8]) -> Value {
    match tag {
        0x21 | 0x23 if raw.len() == 4 => Value::Int(i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]])),
        0x22 if raw.len() == 1 => Value::Bool(raw[0] != 0),
        0x35 | 0x36 if raw.len() >= 4 => {
            let n = u16::from_be_bytes([raw[0], raw[1]]) as usize;
            let m = raw.get(2 + n..4 + n).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize).unwrap_or(0);
            Value::Text(String::from_utf8_lossy(raw.get(4 + n..4 + n + m).unwrap_or_default()).into_owned())
        }
        0x30..=0x4A => Value::Text(String::from_utf8_lossy(raw).into_owned()),
        _ => Value::Raw(raw.to_vec()),
    }
}

/// (status, {atributo: [valores]}) dos grupos de atributos.
pub fn parse_response(data: &[u8]) -> Result<(u16, Attrs)> {
    if data.len() < 8 {
        bail!("resposta IPP curta demais");
    }
    let status = u16::from_be_bytes([data[2], data[3]]);
    let mut attrs = Attrs::new();
    let (mut pos, mut name): (usize, Option<String>) = (8, None);
    let short = || Error::new("resposta IPP truncada");
    while pos < data.len() {
        let tag = data[pos];
        pos += 1;
        if tag == END {
            break;
        }
        if tag < 0x10 {
            continue; // delimitador de grupo
        }
        let n = data.get(pos..pos + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize).ok_or_else(short)?;
        pos += 2;
        if n > 0 {
            let key = String::from_utf8_lossy(data.get(pos..pos + n).ok_or_else(short)?).into_owned();
            attrs.entry(key.clone()).or_default();
            name = Some(key);
        }
        pos += n;
        let v = data.get(pos..pos + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize).ok_or_else(short)?;
        pos += 2;
        let raw = data.get(pos..pos + v).ok_or_else(short)?;
        if let Some(key) = &name {
            attrs.get_mut(key).unwrap().push(decode_value(tag, raw));
        }
        pos += v;
    }
    Ok((status, attrs))
}

/// url: ipp://host[:porta]/caminho ou ipps://...
pub fn get_printer_attributes(url: &str) -> Result<Attrs> {
    let (scheme, rest) = url.split_once("://").ok_or_else(|| Error::new(format!("endereço IPP inválido: {url}")))?;
    let secure = matches!(scheme, "ipps" | "https");
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/ipp/print"),
    };
    let hostport = if hostport.contains(':') && !hostport.ends_with(']') { hostport.to_string() } else { format!("{hostport}:631") };
    let http_url = format!("{}://{hostport}{path}", if secure { "https" } else { "http" });
    let printer_uri = format!("{}://{hostport}{path}", if secure { "ipps" } else { "ipp" });
    // impressoras usam certificado próprio: sem conferir a cadeia
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .disable_verification(true)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .tls_config(tls)
        .http_status_as_error(false)
        .build()
        .into();
    let host = hostport.split(':').next().unwrap_or_default().to_string();
    let mut resp = agent
        .post(&http_url)
        .header("Content-Type", "application/ipp")
        .send(&build_request(&printer_uri, 1)[..])
        .map_err(|e| Error::new(format!("não consegui falar IPP com {host}: {e}")))?;
    if resp.status() != 200 {
        bail!("a impressora respondeu HTTP {}", resp.status().as_u16());
    }
    let body = resp.body_mut().read_to_vec().map_err(|e| Error::new(format!("resposta IPP: {e}")))?;
    let (status, attrs) = parse_response(&body)?;
    if status >= 0x0400 {
        bail!("erro IPP 0x{status:04X}");
    }
    Ok(attrs)
}

/// Endereço IPP de uma impressora instalada no CUPS (rede, Bonjour ou ippusb).
pub fn find_queue_url(queue: &str) -> Result<String> {
    let out = Command::new("lpstat").args(["-v", queue]).output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let caps = Regex::new(r"(\w+)://(\S+)")
        .unwrap()
        .captures(&text)
        .ok_or_else(|| Error::new(format!("fila {queue} não encontrada")))?;
    let (scheme, rest) = (caps[1].to_string(), caps[2].to_string());
    if (scheme == "ipp" || scheme == "ipps") && !rest.contains("._tcp") {
        return Ok(format!("{scheme}://{rest}"));
    }
    let service = crate::maintenance::unquote(rest.split('/').next().unwrap_or_default());
    let service = service.trim_end_matches('.');
    let (name, kind) = service.split_once("._").unwrap_or((service, "ipp._tcp"));
    let kind = format!("_{}", kind.replace(".local", ""));
    resolve_bonjour(name, &kind, Duration::from_secs(4))
}

/// Resolve um serviço Bonjour com o dns-sd (Mac): ipp://host:porta/caminho.
pub fn resolve_bonjour(name: &str, kind: &str, timeout: Duration) -> Result<String> {
    let mut child = Command::new("dns-sd")
        .args(["-L", name, kind, "local"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::new("dns-sd não disponível para achar a impressora na rede"))?;
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let re = Regex::new(r"can be reached at (\S+?)\.?:(\d+)").unwrap();
    let deadline = Instant::now() + timeout;
    let mut out = String::new();
    while Instant::now() < deadline {
        if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
            out.push_str(&String::from_utf8_lossy(&chunk));
            if re.is_match(&out) && out.contains("rp=") {
                break;
            }
        }
    }
    let _ = child.kill();
    let caps = re.captures(&out).ok_or_else(|| Error::new(format!("não achei {name} na rede local (Bonjour)")))?;
    let rp = Regex::new(r"\brp=(\S+)").unwrap().captures(&out).map(|c| c[1].to_string());
    let scheme = if kind.starts_with("_ipps") { "ipps" } else { "ipp" };
    Ok(format!("{scheme}://{}:{}/{}", &caps[1], &caps[2], rp.unwrap_or_else(|| "ipp/print".into())))
}

pub fn texts<'a>(attrs: &'a Attrs, key: &str) -> Vec<&'a str> {
    attrs.get(key).map(|v| v.iter().filter_map(Value::text).collect()).unwrap_or_default()
}

/// Códigos de 4 caracteres (estilo Canon) citados nas mensagens/alertas.
pub fn support_codes(attrs: &Attrs) -> Vec<String> {
    let re = Regex::new(r"[0-9][0-9A-F]{3}|B20[0-9A-F]").unwrap();
    let mut codes: Vec<String> = Vec::new();
    for key in ["printer-state-message", "printer-alert-description", "printer-alert", "printer-state-reasons"] {
        for t in texts(attrs, key) {
            for m in re.find_iter(t) {
                let before = t[..m.start()].chars().next_back();
                let after = t[m.end()..].chars().next();
                let isolated = !before.is_some_and(|c| c.is_ascii_alphanumeric())
                    && !after.is_some_and(|c| c.is_ascii_alphanumeric());
                if isolated && !codes.iter().any(|c| c == m.as_str()) {
                    codes.push(m.as_str().to_string());
                }
            }
        }
    }
    codes
}

/// [(palavra-chave, explicação)] sem os sufixos -report/-warning/-error.
pub fn reasons(attrs: &Attrs) -> Vec<(String, String)> {
    let suffix = Regex::new(r"-(report|warning|error)$").unwrap();
    texts(attrs, "printer-state-reasons")
        .into_iter()
        .map(|r| {
            let base = suffix.replace(r, "").to_string();
            let text = reason_text(&base).or_else(|| reason_text(r)).map(String::from).unwrap_or_else(|| r.to_string());
            (r.to_string(), text)
        })
        .collect()
}
