//! Linha de comando: ecoreset-cli <comando> <ip|usb> ...

use std::io::Write;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use clap::{Parser, Subcommand};
use ecoreset_core::errors::{Problem, explain_epson_error, search};
use ecoreset_core::models::{Model, by_name};
use ecoreset_core::printer::{EpsonPrinter, load_image, parse_int};
use ecoreset_core::{Error, Result, all_models, canon, ipp, ivec, keyfinder, library, maintenance, server, snapshot, usb, wiclog};

#[derive(Parser)]
#[command(name = "ecoreset-cli", version, about = "EcoReset: manutenção de impressoras Epson L/EcoTank e Canon")]
struct Cli {
    /// Modelo (senão, detectado pela impressora)
    #[arg(long, global = true)]
    model: Option<String>,
    /// Prazo da rede em segundos
    #[arg(long, global = true, default_value_t = 2.0)]
    timeout: f64,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Identifica a impressora ("usb" no lugar do IP usa o cabo)
    Detect { host: String },
    /// Lê os contadores da almofada
    Levels { host: String },
    /// Zera os contadores (com backup)
    Reset {
        host: String,
        #[arg(long)]
        yes: bool,
    },
    /// Teste de bicos pela rede (IP), direto pelo cabo ("usb") ou por uma impressora instalada (--printer)
    Nozzle { host: Option<String>, #[arg(long)] printer: Option<String> },
    /// Limpeza de cabeça pela rede (IP), direto pelo cabo ("usb") ou por uma impressora instalada (--printer)
    Clean { host: Option<String>, #[arg(long)] printer: Option<String> },
    /// Reset temporário (firmware bloqueado)
    TempReset {
        host: String,
        /// Número de série (lido da impressora se omitido)
        #[arg(long)]
        serial: Option<String>,
    },
    /// Lê endereços da EEPROM
    Read { host: String, start: String, end: Option<String> },
    /// Grava um endereço da EEPROM
    Write { host: String, address: String, value: String },
    /// Erros da Epson, com causa e correção
    Diagnose { host: String },
    /// Erros da Canon: usb (IVEC, código exato), IP ou nome da fila
    CanonStatus { r#where: String },
    /// Manutenção da Canon pelo USB (IVEC)
    CanonMaint {
        #[arg(value_parser = ivec::maintenance_names())]
        operation: String,
    },
    /// Explica um código de erro (ex.: 1726, 0x10) ou busca por palavra
    Code { code: String },
    /// Leitura completa da EEPROM salva em arquivo
    Snapshot {
        host: String,
        #[arg(short, long)]
        output: String,
        #[arg(long, default_value = "0x7FF")]
        end: String,
        #[arg(long, default_value = "")]
        label: String,
        /// Chave de leitura para modelo fora da lista, ex.: 74,54
        #[arg(long)]
        key: Option<String>,
    },
    /// Descobre a chave de leitura (só lê)
    FindKey {
        host: String,
        /// Testa também as chaves de gravação conhecidas (grava o mesmo valor de volta)
        #[arg(long)]
        write: bool,
    },
    /// Compara duas leituras (antes e depois)
    Compare {
        before: String,
        after: String,
        #[arg(long)]
        save: Option<String>,
    },
    /// Regrava a EEPROM a partir de um arquivo (.json do EcoReset ou binário)
    WriteImage {
        host: String,
        file: String,
        #[arg(long, default_value = "0")]
        start: String,
        /// Grava também série/MAC (só para backup da mesma impressora)
        #[arg(long)]
        include_protected: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Aprende um modelo pelo log de um reset do WICReset
    WicLog {
        log: String,
        #[arg(long)]
        name: Option<String>,
        /// Porcentagem em que o reset do log deixou a almofada (trial: 80)
        #[arg(long, default_value_t = wiclog::TRIAL_PERCENT)]
        percent: f64,
        /// Grava no banco (e envia ao servidor, se configurado)
        #[arg(long)]
        save: bool,
    },
    /// Banco de modelos aprendidos
    Library {
        /// Usa esta pasta (ex.: uma pasta do Google Drive)
        #[arg(long)]
        folder: Option<String>,
        /// Junta outro modelos.json neste
        #[arg(long)]
        merge: Option<String>,
        /// Apaga um modelo aprendido
        #[arg(long)]
        remove: Option<String>,
        /// Usa o banco do servidor (pede a chave)
        #[arg(long, num_args = 0..=1, default_missing_value = server::DEFAULT_URL)]
        server: Option<String>,
        /// Atualizar: envia os pendentes e baixa o banco do servidor
        #[arg(long)]
        sync: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Erro: {e}");
            ExitCode::from(2)
        }
    }
}

fn num(text: &str, what: &str) -> Result<u64> {
    parse_int(text).ok_or_else(|| Error::new(format!("{what} inválido: {text} (use decimal ou 0x..)")))
}

fn connect(host: &str, timeout: f64) -> Result<EpsonPrinter> {
    if host.eq_ignore_ascii_case("usb") {
        let (p, info) = usb::open_printer()?;
        if !info.port.is_empty() {
            eprintln!("USB, porta {}", info.port);
        }
        return Ok(p);
    }
    let mut p = EpsonPrinter::network(host);
    if timeout > 0.0 {
        let mut c = ecoreset_core::snmp::SnmpClient::new(host);
        c.timeout = Duration::from_secs_f64(timeout);
        p = EpsonPrinter::new(host, Box::new(ecoreset_core::printer::SnmpTransport(c)));
    }
    Ok(p)
}

fn printer(cli: &Cli, host: &str, models: &[Model], key: Option<&str>) -> Result<EpsonPrinter> {
    let mut p = connect(host, cli.timeout)?;
    if let Some(name) = &cli.model {
        p.set_model(by_name(name, models).ok_or_else(|| Error::new(format!("modelo desconhecido: {name}")))?);
    } else {
        let (text, name) = p.detect(models)?;
        println!("Impressora: {}", if text.is_empty() { "?" } else { &text });
        if name.is_none() {
            match key {
                // modelo fora da lista: só leitura, com a chave dada
                Some(k) => {
                    let parts: Vec<u8> = k.split(',').filter_map(|x| parse_int(x).map(|n| n as u8)).collect();
                    if parts.len() != 2 {
                        return Err(Error::new("chave inválida (ex.: 74,54)"));
                    }
                    p.read_key = Some([parts[0], parts[1]]);
                }
                None => {
                    let names = ecoreset_core::models::names(models).join(", ");
                    return Err(Error::new(format!("modelo não suportado; use --model ({names})")));
                }
            }
        }
    }
    match (&p.model, p.read_key) {
        (Some(m), _) => println!("Modelo: {}", m.name),
        (None, Some(k)) => println!("Modelo: fora da lista (chave {},{})", k[0], k[1]),
        _ => {}
    }
    Ok(p)
}

fn confirm(question: &str) -> bool {
    print!("{question} [s/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().read_line(&mut answer);
    answer.trim().eq_ignore_ascii_case("s")
}

/// AAAAMMDD-hhmmss, para o nome dos backups.
fn stamp() -> String {
    ecoreset_core::printer::now_iso().replace(['-', ':'], "").replace('T', "-")
}

fn print_problem(p: &Problem) {
    println!("\n{} {}: {}", p.brand, p.code, p.title);
    println!("  Causa: {}", p.cause);
    println!("  Correção: {}", p.fix);
    if p.service {
        println!("  Pode exigir assistência técnica.");
    }
}

fn print_canon(d: &canon::CanonDiagnosis) {
    println!("Modelo: {}\nEstado: {}", d.modelo, d.estado);
    for line in std::iter::once(&d.mensagem).chain(d.motivos.iter()) {
        if !line.is_empty() {
            println!("{line}");
        }
    }
    for (n, v) in &d.tintas {
        println!("Nível {n}: {v}");
    }
    d.problemas.iter().for_each(print_problem);
}

fn describe(m: &Model) {
    println!("Modelo: {}", m.name);
    println!("  Chave de leitura: {},{}", m.read_key[0], m.read_key[1]);
    println!("  Chave de gravação: {}", m.write_key_text());
    let reset: Vec<String> = m.raw_waste_reset.iter().map(|(a, v)| format!("{a}: {v}")).collect();
    println!("  Reset grava: {{{}}}", reset.join(", "));
    for (label, c) in m.counters() {
        println!("  Contador {label}: endereços {:?}, divisor {}", c.oids, c.divider);
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let models = all_models();
    match &cli.cmd {
        Cmd::Code { code } => {
            let found = if code.to_lowercase().starts_with("0x") {
                vec![explain_epson_error(num(code, "código")? as u8)]
            } else {
                search(code)
            };
            found.iter().for_each(print_problem);
            return Ok(if found.is_empty() { ExitCode::from(1) } else { ExitCode::SUCCESS });
        }
        Cmd::Compare { before, after, save } => {
            let (b, a) = (snapshot::Snapshot::load(before)?, snapshot::Snapshot::load(after)?);
            let (changes, warnings) = snapshot::compare(&b, &a);
            warnings.iter().for_each(|w| println!("Atenção: {w}"));
            println!("{} endereço(s) mudaram:", changes.len());
            for (addr, old, new) in &changes {
                println!("{addr:5} (0x{addr:04X}): {old:3} -> {new:3}   {}", snapshot::classify(*addr, *old, *new));
            }
            if !changes.is_empty() {
                let range = a.model.as_deref().and_then(|n| by_name(n, &models)).map(|m| m.serial_range()).unwrap_or_default();
                let s = snapshot::suggest_model(&b, &a, &changes, &range);
                println!("\nReset sugerido: {}", s["raw_waste_reset"]);
                if let Some(path) = save {
                    std::fs::write(path, serde_json::to_string_pretty(&s).unwrap())?;
                    println!("Sugestão salva em {path}");
                }
            }
        }
        Cmd::CanonMaint { operation } => {
            ivec::CanonUsb::open()?.maintenance(operation, false)?;
            println!("{operation} enviado para a Canon.");
        }
        Cmd::CanonStatus { r#where } => {
            let d = if r#where.eq_ignore_ascii_case("usb") {
                ivec::CanonUsb::open()?.diagnose()?
            } else if r#where.chars().all(|c| c.is_ascii_digit() || c == '.') {
                canon::diagnose(&format!("ipp://{}:631/ipp/print", r#where))?
            } else {
                canon::diagnose(&ipp::find_queue_url(r#where)?)?
            };
            print_canon(&d);
        }
        Cmd::Nozzle { host, printer } | Cmd::Clean { host, printer } => {
            let nozzle = matches!(cli.cmd, Cmd::Nozzle { .. });
            let job = if nozzle { maintenance::nozzle_check_job() } else { maintenance::head_cleaning_job() };
            match (host, printer) {
                (_, Some(q)) => maintenance::send_raw_installed(q, &job)?,
                (Some(h), None) if h.eq_ignore_ascii_case("usb") => {
                    maintenance::send_raw_usb(usb::EPSON_VENDOR, &job)?;
                }
                (Some(h), None) => maintenance::send_raw_network(h, &job)?,
                _ => return Err(Error::new("informe o IP ou --printer NOME")),
            }
            println!("{} enviado.", if nozzle { "Teste de bicos" } else { "Limpeza de cabeça" });
        }
        Cmd::TempReset { host, serial } => {
            let mut p = connect(host, cli.timeout)?;
            let serial = p.temporary_reset(serial.as_deref())?;
            println!("Impressora {serial} liberada. Desligue e ligue de novo.");
        }
        Cmd::FindKey { host, write } => {
            let mut p = connect(host, cli.timeout)?;
            let (text, name) = p.detect(&models)?;
            println!("Impressora: {}", if text.is_empty() { "?" } else { &text });
            if let Some(n) = name {
                let k = p.read_key.unwrap();
                println!("Este modelo já está na lista ({n}), chave {},{}.", k[0], k[1]);
                return Ok(ExitCode::SUCCESS);
            }
            let mut progress = |i: usize, n: usize, k: [u8; 2]| {
                if i.is_multiple_of(1024) {
                    eprintln!("  {i}/{n} (testando {},{})", k[0], k[1]);
                }
            };
            let Some(r) = keyfinder::find_read_key(&mut p, &models, 0, &mut progress, &AtomicBool::new(false))? else {
                return Ok(ExitCode::from(1));
            };
            println!("Chave de leitura: {},{} ({} tentativas)", r.read_key[0], r.read_key[1], r.tries);
            if r.accepts_any {
                println!("Atenção: a impressora também aceitou outra chave.");
            }
            if *write {
                match keyfinder::find_write_key(&mut p, &models, 0)? {
                    Some(k) => println!("Chave de gravação: {}", String::from_utf8_lossy(&k)),
                    None => println!("Chave de gravação: nenhuma das conhecidas foi aceita"),
                }
            }
        }
        Cmd::WicLog { log, name, percent, save } => {
            let (m, warnings) = wiclog::learn_file(log, name.as_deref(), *percent, &models)?;
            describe(&m);
            warnings.iter().for_each(|w| println!("Atenção: {w}"));
            if *save {
                let problem = server::publish(&m)?;
                println!("Salvo no banco: {}", library::db_path().display());
                if server::configured() {
                    match problem {
                        Some(p) => println!("Atenção: não foi ao servidor: {p}"),
                        None => println!("Enviado ao servidor."),
                    }
                }
            } else {
                println!("(use --save para gravar no banco)");
            }
        }
        Cmd::Library { folder, merge, remove, server: srv, sync } => {
            if let Some(f) = folder {
                println!("Pasta do banco: {}", library::set_folder(f)?.display());
            }
            let mut sync = *sync;
            if let Some(url) = srv {
                let token = rpassword::prompt_password("Chave do EcoReset no servidor: ")?;
                server::configure(url, &token)?;
                sync = true;
            }
            if sync {
                let (sent, total, new) = server::sync()?;
                let new = if new.is_empty() { "nenhum".into() } else { new.join(", ") };
                println!("Atualizado: {sent} enviado(s), {total} no servidor, novos: {new}");
            }
            if let Some(m) = merge {
                let new = library::merge_file(m)?;
                println!("Juntado. Novos: {}", if new.is_empty() { "nenhum".into() } else { new.join(", ") });
            }
            if let Some(r) = remove {
                println!("{}", if server::remove(r)? { "Apagado." } else { "Não está no banco." });
            }
            println!("Banco: {}", library::db_path().display());
            let (url, token) = server::settings();
            println!("Servidor: {}", if token.is_empty() { "não configurado (use --server)".into() } else { url });
            let learned = library::load()?;
            for m in &learned {
                let l = m.learned.clone().unwrap_or_default();
                println!(
                    "  {:12} chave {},{}  ({}, {}){}",
                    m.name, m.read_key[0], m.read_key[1], l.on, l.at, if l.pending { "  [pendente]" } else { "" }
                );
            }
            if learned.is_empty() {
                println!("  nenhum modelo aprendido ainda");
            }
        }
        Cmd::Detect { host } => {
            let mut p = connect(host, cli.timeout)?;
            let (text, name) = p.detect(&models)?;
            println!("Impressora: {}", if text.is_empty() { "?" } else { &text });
            println!("Modelo: {}", name.unwrap_or_else(|| "não está na lista".into()));
            if let Ok(serial) = p.serial_number() {
                println!("Série: {serial}");
            }
        }
        Cmd::Diagnose { host } => {
            let d = connect(host, cli.timeout)?.diagnose()?;
            println!("Estado: {}", d.estado);
            d.avisos.iter().for_each(|a| println!("Aviso: {a}"));
            d.problemas.iter().for_each(print_problem);
            if d.problemas.is_empty() {
                println!("Nenhum erro.");
            }
        }
        Cmd::Levels { host } => {
            let mut p = printer(&cli, host, &models, None)?;
            for (label, pct) in p.waste_levels()? {
                println!("Almofada {label}: {pct:.2}%");
            }
        }
        Cmd::Reset { host, yes } => {
            let mut p = printer(&cli, host, &models, None)?;
            if !yes && !confirm("Zerar os contadores?") {
                return Ok(ExitCode::from(1));
            }
            let path = format!("backup-{}-{}.json", p.model_name().unwrap_or_default(), stamp());
            p.reset_waste(Some(&path))?;
            println!("Contadores zerados. Backup em {path}. Desligue e ligue a impressora.");
        }
        Cmd::Read { host, start, end } => {
            let mut p = printer(&cli, host, &models, None)?;
            let start = num(start, "endereço")? as u16;
            let end = end.as_deref().map(|e| num(e, "endereço final")).transpose()?.map(|e| e as u16).unwrap_or(start);
            for (a, v) in p.dump(start, end)? {
                println!("{a:5} (0x{a:04X}): {v:3} (0x{v:02X})");
            }
        }
        Cmd::Write { host, address, value } => {
            let mut p = printer(&cli, host, &models, None)?;
            let value = num(value, "valor")?;
            if value > 255 {
                return Err(Error::new("o valor precisa estar entre 0 e 255"));
            }
            p.write_eeprom(num(address, "endereço")? as u16, value as u8, true)?;
            println!("Gravado.");
        }
        Cmd::Snapshot { host, output, end, label, key } => {
            let mut p = printer(&cli, host, &models, key.as_deref())?;
            let end = num(end, "fim")? as u16;
            let mut progress = |i: usize, n: usize| {
                if i.is_multiple_of(128) || i == n {
                    eprintln!("  {i}/{n}");
                }
            };
            let snap = snapshot::take(&mut p, 0, end, label, &mut progress)?;
            snap.save(output)?;
            println!("Leitura salva em {output}: {} endereços lidos, {} recusados.", snap.eeprom.len(), snap.refused.len());
        }
        Cmd::WriteImage { host, file, start, include_protected, yes } => {
            let mut p = printer(&cli, host, &models, None)?;
            let (image, file_model) = load_image(file, num(start, "início")? as u16)?;
            if let Some(fm) = file_model.filter(|fm| Some(fm) != p.model_name().as_ref()) {
                println!("Atenção: o arquivo é de {fm} e a impressora é {}.", p.model_name().unwrap_or_default());
            }
            let (changes, skipped) = p.plan_image(&image, *include_protected)?;
            println!("{} endereço(s) a gravar, {} protegido(s) ignorado(s).", changes.len(), skipped.len());
            for (a, old, new) in &changes {
                println!("{a:5} (0x{a:04X}): {old:3} -> {new:3}");
            }
            if changes.is_empty() {
                return Ok(ExitCode::SUCCESS);
            }
            if !yes && !confirm("Gravar?") {
                return Ok(ExitCode::from(1));
            }
            let path = format!("backup-antes-regravar-{}.json", stamp());
            p.write_image(&changes, Some(&path))?;
            println!("Regravado e conferido. Backup em {path}.");
        }
    }
    Ok(ExitCode::SUCCESS)
}
