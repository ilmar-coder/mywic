//! Banco local e sincronização com o servidor (um servidor HTTP simulado).
//! Tudo num teste só: o banco usa variáveis de ambiente, que valem para o processo.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use ecoreset_core::models::{builtin, find_model, merge};
use ecoreset_core::{library, server, wiclog};
use serde_json::{Value, json};

const TOKEN: &str = "chave-de-teste-1234567890";

#[derive(Default)]
struct Api {
    store: BTreeMap<String, Value>,
    online: bool,
}

fn serve(api: Arc<Mutex<Api>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let (mut token, mut len) = (String::new(), 0usize);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let l = line.trim_end();
                if l.is_empty() {
                    break;
                }
                let (k, v) = l.split_once(": ").unwrap_or((l, ""));
                match k.to_lowercase().as_str() {
                    "x-ecoreset-token" => token = v.into(),
                    "content-length" => len = v.parse().unwrap(),
                    _ => {}
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            let mut parts = first.split_whitespace();
            let (method, path) = (parts.next().unwrap().to_string(), parts.next().unwrap().to_string());
            let name = path.trim_start_matches("/api/ecoreset/models").trim_start_matches('/').replace("%20", " ");
            let mut a = api.lock().unwrap();
            let (code, reply) = if !a.online {
                (503, json!({"error": "fora do ar"}))
            } else if token != TOKEN {
                (401, json!({"error": "chave do EcoReset inválida"}))
            } else if method == "GET" {
                (200, json!({"type": "ecoreset-modelos", "modelos": a.store}))
            } else if method == "PUT" {
                let mut v: Value = serde_json::from_slice(&body).unwrap();
                v["updated_at"] = json!("2026-11-12T10:00:00Z");
                a.store.insert(name, v);
                (200, json!({"ok": true}))
            } else if a.store.remove(&name).is_some() {
                (200, json!({"ok": true}))
            } else {
                (404, json!({"error": "modelo não encontrado"}))
            };
            let text = reply.to_string();
            let _ = write!(s, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
        }
    });
    url
}

fn trial_model() -> ecoreset_core::models::Model {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/wicreset_l3250_trial.log");
    let (mut m, _) = wiclog::learn_file(path, Some("XP-9999"), 80.0, &builtin()).unwrap();
    m.aliases.clear();
    m
}

#[test]
fn library_and_server_sync() {
    let dir = tempfile::tempdir().unwrap();
    let set_machine = |n: &str| unsafe { std::env::set_var("ECORESET_PASTA", dir.path().join(n)) };
    unsafe { std::env::set_var("ECORESET_CONFIG", dir.path().join("config.json")) };
    set_machine("maquina1");

    // sem servidor: só o banco local
    let m = trial_model();
    assert_eq!(server::publish(&m).unwrap(), None);
    let loaded = library::load().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].raw_waste_reset, m.raw_waste_reset);
    let all = merge(loaded);
    assert_eq!(find_model("EPSON XP-9999 Series", &all).unwrap().name, "XP-9999");
    assert!(library::remove("XP-9999", None).unwrap());
    assert!(library::load().unwrap().is_empty());

    // com servidor: publica e outra máquina recebe
    let api = Arc::new(Mutex::new(Api { online: true, ..Default::default() }));
    let url = serve(api.clone());
    assert!(server::configure("http://exemplo.com", TOKEN).unwrap_err().message.contains("https"));
    server::configure(&url, TOKEN).unwrap();
    assert_eq!(server::settings(), (url.clone(), TOKEN.to_string()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("config.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert_eq!(server::publish(&m).unwrap(), None);
    assert!(!api.lock().unwrap().store["XP-9999"].as_object().unwrap().contains_key("pendente"));
    set_machine("maquina2");
    assert_eq!(server::sync().unwrap(), (0, 1, vec!["XP-9999".to_string()]));
    assert_eq!(library::load().unwrap()[0].write_key, *b"Maribaya");

    // sem conexão: fica pendente e sobe no próximo Atualizar
    api.lock().unwrap().online = false;
    let mut other = m.clone();
    other.name = "XP-8888".into();
    assert!(server::publish(&other).unwrap().unwrap().contains("503"));
    assert!(library::load().unwrap().iter().any(|x| x.name == "XP-8888" && x.learned.as_ref().unwrap().pending));
    api.lock().unwrap().online = true;
    assert_eq!(server::sync().unwrap(), (1, 2, vec![]));
    assert!(library::load().unwrap().iter().all(|x| !x.learned.as_ref().unwrap().pending));

    // apagar chega nas outras máquinas
    assert!(server::remove("XP-8888").unwrap());
    set_machine("maquina1");
    server::sync().unwrap();
    assert_eq!(library::load().unwrap().len(), 1);

    // chave errada
    server::configure(&url, "errada").unwrap();
    assert!(server::sync().unwrap_err().message.contains("401"));

    // juntar o arquivo de outra máquina
    let other_db = dir.path().join("maquina2").join("modelos.json");
    set_machine("maquina3");
    assert_eq!(library::merge_file(other_db.to_str().unwrap()).unwrap(), vec!["XP-9999".to_string()]);
}
