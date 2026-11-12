//! EcoReset: tela (egui) para Windows, Mac e Linux.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, RichText};
use ecoreset_core::errors::{Problem, explain_epson_error, search};
use ecoreset_core::models::{Model, by_name, names};
use ecoreset_core::printer::{Backup, EpsonPrinter, load_image, parse_int};
use ecoreset_core::usb::UsbInfo;
use ecoreset_core::{Error, Result, all_models, canon, ipp, ivec, keyfinder, library, maintenance, server, snapshot, usb, wiclog};

const ACCENT: Color32 = Color32::from_rgb(76, 175, 80);
const WARN: Color32 = Color32::from_rgb(230, 120, 90);
const MUTED: Color32 = Color32::from_rgb(150, 156, 170);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 680.0])
            .with_min_inner_size([620.0, 480.0])
            .with_title(format!("EcoReset {}", ecoreset_core::VERSION)),
        ..Default::default()
    };
    eframe::run_native("EcoReset", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Diagnose,
    Waste,
    Maintenance,
    Eeprom,
    Library,
}

#[derive(Clone, Copy, PartialEq)]
enum Brand {
    Epson,
    Canon,
}

type Job = Box<dyn FnOnce(&mut App) + Send>;
type Then = Box<dyn FnOnce(&mut App)>;

/// Janelas que esperam o usuário.
enum Dialog {
    Message { text: String, error: bool },
    Confirm { text: String, yes: Then },
    Input { prompt: String, value: String, secret: bool, ok: Box<dyn FnOnce(&mut App, String)> },
}

/// Progresso que o trabalho em segundo plano mostra na barra de baixo.
#[derive(Clone)]
struct Progress {
    text: Arc<Mutex<String>>,
    ctx: egui::Context,
}

impl Progress {
    fn set(&self, s: String) {
        *self.text.lock().unwrap() = s;
        self.ctx.request_repaint();
    }
}

/// Onde e como abrir a Epson, montado na tela e usado no trabalho de fundo.
#[derive(Clone)]
struct Target {
    usb: bool,
    host: String,
    model: Option<Model>,
    key: Option<[u8; 2]>,
    models: Vec<Model>,
}

impl Target {
    /// Conecta (rede ou USB) e escolhe o modelo. O USB fica preso só enquanto
    /// o EpsonPrinter existir, para não atrapalhar a impressão.
    fn open(&self) -> Result<(EpsonPrinter, UsbInfo)> {
        let (mut p, info) = if self.usb { usb::open_printer()? } else { (EpsonPrinter::network(&self.host), UsbInfo::default()) };
        match &self.model {
            Some(m) => p.set_model(m),
            None => {
                p.detect(&self.models)?;
            }
        }
        if p.model.is_none() {
            p.read_key = self.key; // modelo fora da lista: só a leitura funciona
        }
        Ok((p, info))
    }
}

struct App {
    ctx: egui::Context,
    tx: Sender<Job>,
    rx: Receiver<Job>,
    busy: Option<String>,
    progress: Arc<Mutex<String>>,
    cancel: Arc<AtomicBool>,
    can_cancel: bool,
    status: String,
    dialogs: Vec<Dialog>,
    models: Vec<Model>,
    tab: Tab,
    // impressora
    usb: bool,
    host: String,
    brand: Brand,
    model: String,
    info: String,
    // diagnóstico
    diag: Vec<(String, Vec<Problem>)>,
    lookup: String,
    // almofada
    levels: Vec<(String, f64)>,
    // manutenção
    via_installed: bool,
    printers: Vec<String>,
    queue: String,
    ivec_op: String,
    // eeprom
    addr: String,
    addr_end: String,
    value: String,
    manual_key: String,
    ee_out: String,
    last_dump: Vec<(u16, u8)>,
    // banco
    learned: Vec<Model>,
    selected: Option<String>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel();
        let printers = maintenance::installed_printers();
        let queue = printers.iter().find(|n| n.to_uppercase().contains("EPSON")).or(printers.first()).cloned().unwrap_or_default();
        let mut app = App {
            ctx: cc.egui_ctx.clone(),
            tx,
            rx,
            busy: None,
            progress: Default::default(),
            cancel: Default::default(),
            can_cancel: false,
            status: "Pronto.".into(),
            dialogs: Vec::new(),
            models: all_models(),
            tab: Tab::Diagnose,
            usb: true,
            host: String::new(),
            brand: Brand::Epson,
            model: String::new(),
            info: "Escolha a conexão e clique em Detectar. Pelo USB funciona até em firmwares que \
                   bloqueiam a rede (como a L3250)."
                .into(),
            diag: Vec::new(),
            lookup: String::new(),
            levels: Vec::new(),
            via_installed: !printers.is_empty(),
            printers,
            queue,
            ivec_op: "limpeza".into(),
            addr: "0".into(),
            addr_end: String::new(),
            value: String::new(),
            manual_key: String::new(),
            ee_out: String::new(),
            last_dump: Vec::new(),
            learned: Vec::new(),
            selected: None,
        };
        app.refresh_library();
        if server::configured() {
            app.sync_library(true);
        }
        app
    }

    // ---- infraestrutura ---------------------------------------------------------

    fn message(&mut self, text: impl Into<String>) {
        self.dialogs.push(Dialog::Message { text: text.into(), error: false });
    }

    fn error(&mut self, e: impl std::fmt::Display) {
        self.status = "Erro.".into();
        self.dialogs.push(Dialog::Message { text: e.to_string(), error: true });
    }

    fn confirm(&mut self, text: impl Into<String>, yes: impl FnOnce(&mut App) + 'static) {
        self.dialogs.push(Dialog::Confirm { text: text.into(), yes: Box::new(yes) });
    }

    fn input(&mut self, prompt: &str, value: &str, secret: bool, ok: impl FnOnce(&mut App, String) + 'static) {
        self.dialogs.push(Dialog::Input { prompt: prompt.into(), value: value.into(), secret, ok: Box::new(ok) });
    }

    /// Roda `work` numa thread e entrega o resultado a `done` na tela.
    fn spawn<T: Send + 'static>(
        &mut self,
        label: &str,
        work: impl FnOnce(&Progress, &AtomicBool) -> Result<T> + Send + 'static,
        done: impl FnOnce(&mut App, T) + Send + 'static,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(format!("{label}…"));
        self.status = format!("{label}…");
        self.cancel.store(false, Ordering::Relaxed);
        *self.progress.lock().unwrap() = String::new();
        let (tx, cancel) = (self.tx.clone(), self.cancel.clone());
        let progress = Progress { text: self.progress.clone(), ctx: self.ctx.clone() };
        std::thread::spawn(move || {
            let result = work(&progress, &cancel);
            let job: Job = Box::new(move |app: &mut App| {
                app.busy = None;
                app.can_cancel = false;
                match result {
                    Ok(v) => {
                        app.status = "Pronto.".into();
                        done(app, v);
                    }
                    Err(e) => app.error(e),
                }
            });
            let _ = tx.send(job);
            progress.ctx.request_repaint();
        });
    }

    fn poll(&mut self) {
        while let Ok(job) = self.rx.try_recv() {
            job(self);
        }
    }

    fn manual_key(&self) -> Result<Option<[u8; 2]>> {
        let text = self.manual_key.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let parts: Vec<u8> = text.split(',').filter_map(|x| parse_int(x).filter(|n| *n < 256).map(|n| n as u8)).collect();
        if parts.len() != 2 {
            return Err(Error::new("chave de leitura inválida (ex.: 74,54)"));
        }
        Ok(Some([parts[0], parts[1]]))
    }

    fn target(&self) -> Result<Target> {
        if self.brand == Brand::Canon {
            return Err(Error::new("esta função é só para Epson. Na Canon, use as abas Diagnóstico e Manutenção."));
        }
        if !self.usb && self.host.trim().is_empty() {
            return Err(Error::new("digite o IP da impressora"));
        }
        Ok(Target {
            usb: self.usb,
            host: self.host.trim().into(),
            model: by_name(&self.model, &self.models).cloned(),
            key: self.manual_key()?,
            models: self.models.clone(),
        })
    }

    /// Trabalho com a Epson: conecta, roda `f` e desconecta.
    fn session<T: Send + 'static>(
        &mut self,
        label: &str,
        f: impl FnOnce(&mut EpsonPrinter, &Progress, &AtomicBool) -> Result<T> + Send + 'static,
        done: impl FnOnce(&mut App, T) + Send + 'static,
    ) {
        match self.target() {
            Ok(t) => self.spawn(label, move |pr, c| {
                let (mut p, _) = t.open()?;
                f(&mut p, pr, c)
            }, done),
            Err(e) => self.error(e),
        }
    }

    fn where_text(&self) -> String {
        if self.usb { "USB".into() } else { self.host.trim().into() }
    }

    fn backup_dir() -> PathBuf {
        let dir = library::folder().join("backups");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn stamp() -> String {
        ecoreset_core::printer::now_iso().replace(['-', ':'], "").replace('T', "-")
    }

    // ---- ações: impressora ------------------------------------------------------

    fn detect(&mut self) {
        if self.brand == Brand::Canon {
            return self.diagnose();
        }
        self.model.clear();
        let Ok(t) = self.target() else { return self.error("digite o IP da impressora") };
        let usb_conn = t.usb;
        self.spawn(
            "Detectando",
            move |_, _| {
                let (mut p, info) = t.open()?;
                let text = p.model_string()?;
                let serial = p.serial_number().unwrap_or_default();
                let queues = if usb_conn {
                    let model = text.to_uppercase().replace("EPSON", "").trim().to_string();
                    maintenance::queues_for_usb(&info.port, &info.serial, &model)
                } else {
                    Vec::new()
                };
                Ok((text, p.model_name(), info.port, queues, serial))
            },
            move |app, (text, name, port, queues, serial)| {
                let mut place = app.where_text();
                if !port.is_empty() {
                    place += &format!(", porta {port}");
                }
                if let Some(q) = queues.first() {
                    place += &format!(", impressora instalada \"{q}\"");
                    app.queue = q.clone(); // a mesma na aba Manutenção
                    app.via_installed = true;
                } else if usb_conn {
                    place += ", sem impressora instalada nessa porta";
                }
                let serial = if serial.is_empty() { String::new() } else { format!(", série {serial}") };
                let text = if text.is_empty() { "Sem nome".to_string() } else { text };
                match name {
                    Some(n) => {
                        app.info = format!("{text} ({place}{serial}) → parâmetros do {n}");
                        app.model = n;
                    }
                    None => app.info = format!("{text} ({place}): modelo não está na lista; escolha um compatível."),
                }
            },
        );
    }

    fn show_problems(&mut self, title: String, lines: Vec<String>, problems: Vec<Problem>) {
        let mut text = vec![title];
        text.extend(lines);
        self.diag = vec![(text.join("\n"), problems)];
    }

    fn diagnose(&mut self) {
        if self.brand == Brand::Epson {
            return self.session("Lendo o estado", |p, _, _| p.diagnose(), |app, d| {
                let mut lines: Vec<String> = d.avisos.iter().map(|a| format!("Aviso: {a}")).collect();
                if d.problemas.is_empty() {
                    lines.push("Nenhum erro.".into());
                }
                app.show_problems(format!("Estado: {}", d.estado), lines, d.problemas);
            });
        }
        let (usb_conn, host, queue) = (self.usb, self.host.trim().to_string(), self.queue.clone());
        if !usb_conn && host.is_empty() && queue.is_empty() {
            return self.error("digite o IP da Canon ou escolha a impressora instalada na aba Manutenção");
        }
        self.spawn(
            "Lendo o estado da Canon",
            move |_, _| {
                if usb_conn {
                    return ivec::CanonUsb::open()?.diagnose();
                }
                let url = if host.is_empty() { ipp::find_queue_url(&queue)? } else { format!("ipp://{host}:631/ipp/print") };
                canon::diagnose(&url)
            },
            |app, d| {
                let mut lines = vec![format!("Estado: {}", d.estado)];
                lines.extend([d.mensagem.clone()].into_iter().chain(d.motivos.clone()).filter(|l| !l.is_empty()));
                lines.extend(d.tintas.iter().map(|(n, v)| format!("Nível {n}: {v}")));
                app.show_problems(format!("Modelo: {}", d.modelo), lines, d.problemas);
            },
        );
    }

    fn lookup_code(&mut self) {
        let code = self.lookup.trim().to_string();
        if code.is_empty() {
            return;
        }
        let found = if code.to_lowercase().starts_with("0x") {
            parse_int(&code).map(|n| vec![explain_epson_error(n as u8)]).unwrap_or_default()
        } else {
            search(&code)
        };
        let title = if found.is_empty() { format!("Nada encontrado para \"{code}\".") } else { format!("Resultado para \"{code}\":") };
        self.show_problems(title, Vec::new(), found);
    }

    fn read_levels(&mut self) {
        self.session("Lendo contadores", |p, _, _| p.waste_levels(), |app, l| app.levels = l);
    }

    fn reset(&mut self) {
        let Ok(t) = self.target() else { return self.error("digite o IP da impressora") };
        let name = t.model.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| "o modelo detectado".into());
        self.confirm(
            format!("Zerar os contadores da almofada ({name})?\n\nAntes, o EcoReset salva um backup dos valores atuais na pasta de backups."),
            |app| {
                app.session(
                    "Zerando",
                    |p, _, _| {
                        let path = App::backup_dir().join(format!("backup-{}-{}.json", p.model_name().unwrap_or_default(), App::stamp()));
                        p.reset_waste(Some(&path.to_string_lossy()))?;
                        Ok((path, p.waste_levels().unwrap_or_default()))
                    },
                    |app, (path, levels)| {
                        app.levels = levels;
                        app.message(format!("Contadores zerados. Desligue e ligue a impressora.\n\nBackup: {}", path.display()));
                    },
                )
            },
        );
    }

    fn temp_reset(&mut self) {
        self.session(
            "Lendo o número de série",
            |p, _, _| Ok(p.serial_number().unwrap_or_default()),
            |app, serial| {
                app.input(
                    "Reset temporário (firmware que bloqueia a gravação). Confira o número de série da etiqueta:",
                    &serial,
                    false,
                    |app, serial| {
                        app.session(
                            "Reset temporário",
                            move |p, _, _| p.temporary_reset(Some(&serial)),
                            |app, s| app.message(format!("Impressora {s} liberada. Desligue e ligue de novo.")),
                        )
                    },
                )
            },
        );
    }

    fn restore(&mut self) {
        let Some(path) = rfd::FileDialog::new().set_title("Backup para restaurar").add_filter("Backup", &["json"]).set_directory(App::backup_dir()).pick_file() else {
            return;
        };
        let backup = match Backup::load(&path.to_string_lossy()) {
            Ok(b) => b,
            Err(e) => return self.error(e),
        };
        let text = format!(
            "Restaurar {} endereço(s) do backup de {} ({})?",
            backup.eeprom.len(),
            backup.date,
            backup.model.clone().unwrap_or("?".into())
        );
        self.confirm(text, move |app| {
            app.session("Restaurando", move |p, _, _| p.restore(&backup), |app, _| app.message("Backup restaurado e conferido."))
        });
    }

    // ---- ações: manutenção ------------------------------------------------------

    fn maintain(&mut self, nozzle: bool) {
        let label = if nozzle { "Teste de bicos" } else { "Limpeza de cabeça" };
        if self.brand == Brand::Canon && self.usb {
            return self.maintain_ivec(if nozzle { "teste-bicos" } else { "limpeza" }.into());
        }
        let job = match (self.brand, nozzle) {
            (Brand::Canon, true) => canon::nozzle_check_job(),
            (Brand::Canon, false) => canon::head_cleaning_job(false),
            (Brand::Epson, true) => maintenance::nozzle_check_job(),
            (Brand::Epson, false) => maintenance::head_cleaning_job(),
        };
        let (installed, queue, host, canon_brand) = (self.via_installed, self.queue.clone(), self.host.trim().to_string(), self.brand == Brand::Canon);
        if installed && queue.is_empty() {
            return self.error("Escolha a impressora.");
        }
        if !installed && host.is_empty() {
            return self.error("Digite o IP da impressora.");
        }
        let send = move || -> Result<()> {
            if canon_brand {
                // os modelos só-IVEC ignorariam o trabalho
                let url = if installed { ipp::find_queue_url(&queue)? } else { format!("ipp://{host}:631/ipp/print") };
                canon::check_bjl(&url)?;
            }
            if installed { maintenance::send_raw_installed(&queue, &job) } else { maintenance::send_raw_network(&host, &job) }
        };
        let go = move |app: &mut App| app.spawn(&format!("Enviando {}", label.to_lowercase()), move |_, _| send(), move |app, _| app.status = format!("{label} enviado."));
        if nozzle {
            go(self);
        } else {
            self.confirm("Cada limpeza gasta tinta e enche a almofada. Fazer a limpeza agora?", go);
        }
    }

    fn maintain_ivec(&mut self, name: String) {
        let heavy = matches!(name.as_str(), "limpeza-profunda" | "ink-flush");
        let go = move |app: &mut App| {
            let n = name.clone();
            app.spawn(
                &format!("Enviando {n}"),
                move |_, _| ivec::CanonUsb::open()?.maintenance(&n, false),
                move |app, _| app.status = format!("{name} enviado para a Canon."),
            )
        };
        if heavy {
            self.confirm("Esta limpeza gasta muita tinta e enche a caixa de manutenção. Continuar?", go);
        } else {
            go(self);
        }
    }

    // ---- ações: EEPROM ---------------------------------------------------------

    fn ee_read(&mut self) {
        let (start, end) = match (|| -> Result<(u16, u16)> {
            let s = parse_int(&self.addr).ok_or_else(|| Error::new("endereço inválido (use decimal ou 0x..)"))? as u16;
            let e = if self.addr_end.trim().is_empty() { s } else { parse_int(&self.addr_end).ok_or_else(|| Error::new("endereço final inválido"))? as u16 };
            Ok((s, e))
        })() {
            Ok(v) => v,
            Err(e) => return self.error(e),
        };
        self.session("Lendo EEPROM", move |p, _, _| p.dump(start, end), |app, data| {
            app.ee_out = data.iter().map(|(a, v)| format!("{a:5} (0x{a:04X}): {v:3} (0x{v:02X})")).collect::<Vec<_>>().join("\n");
            app.last_dump = data;
        });
    }

    fn ee_write(&mut self) {
        let (Some(a), Some(v)) = (parse_int(&self.addr), parse_int(&self.value)) else {
            return self.error("endereço ou valor inválido");
        };
        if v > 255 || a > 0xFFFF {
            return self.error("o valor precisa estar entre 0 e 255");
        }
        self.confirm(format!("Gravar {v} no endereço {a} (0x{a:04X})?"), move |app| {
            app.session("Gravando", move |p, _, _| p.write_eeprom(a as u16, v as u8, true), |app, _| app.status = "Gravado e conferido.".into())
        });
    }

    fn ee_save(&mut self) {
        if self.last_dump.is_empty() {
            return self.error("Leia um intervalo primeiro.");
        }
        let Some(path) = rfd::FileDialog::new().set_file_name("leitura-eeprom.json").save_file() else { return };
        let b = Backup { printer: self.where_text(), model: Some(self.model.clone()).filter(|m| !m.is_empty()), date: ecoreset_core::printer::now_iso(), eeprom: self.last_dump.clone() };
        if let Err(e) = b.save(&path.to_string_lossy()) {
            self.error(e);
        }
    }

    fn ee_image(&mut self) {
        let Some(path) = rfd::FileDialog::new().set_title("Arquivo de EEPROM (.json do EcoReset ou binário)").pick_file() else { return };
        let path = path.to_string_lossy().to_string();
        let start = if path.to_lowercase().ends_with(".json") {
            0
        } else {
            match parse_int(&self.addr) {
                Some(n) => n as u16,
                None => return self.error("No binário, o primeiro byte vai para o endereço do campo Endereço."),
            }
        };
        let (image, file_model) = match load_image(&path, start) {
            Ok(v) => v,
            Err(e) => return self.error(e),
        };
        self.session(
            "Comparando com a impressora",
            move |p, _, _| Ok((p.plan_image(&image, false)?, p.model_name())),
            move |app, ((changes, skipped), model)| {
                if changes.is_empty() {
                    return app.message("A impressora já está igual ao arquivo.");
                }
                let mut text = format!("{} endereço(s) a gravar, {} protegido(s) ignorado(s) (série/MAC).\n\n", changes.len(), skipped.len());
                if let Some(fm) = file_model.filter(|fm| Some(fm) != model.as_ref()) {
                    text += &format!("Atenção: o arquivo é de {fm} e a impressora é {}.\n\n", model.unwrap_or_default());
                }
                for (a, old, new) in changes.iter().take(30) {
                    text += &format!("{a:5} (0x{a:04X}): {old:3} -> {new:3}\n");
                }
                if changes.len() > 30 {
                    text += "...\n";
                }
                text += "\nGravar? Um backup dos valores atuais é salvo antes.";
                app.confirm(text, move |app| {
                    app.session(
                        "Regravando",
                        move |p, _, _| {
                            let path = App::backup_dir().join(format!("backup-antes-regravar-{}.json", App::stamp()));
                            p.write_image(&changes, Some(&path.to_string_lossy()))?;
                            Ok(path)
                        },
                        |app, path| app.message(format!("Regravado e conferido.\n\nBackup: {}", path.display())),
                    )
                });
            },
        );
    }

    fn snapshot_take(&mut self) {
        let Some(path) = rfd::FileDialog::new().set_title("Salvar leitura completa").set_file_name(format!("leitura-{}.json", App::stamp())).save_file() else {
            return;
        };
        self.input("Rótulo da leitura (ex.: antes do reset):", "", false, move |app, label| {
            app.session(
                "Leitura completa",
                move |p, pr, _| {
                    let snap = snapshot::take(p, 0, snapshot::DEFAULT_END, &label, &mut |i, n| {
                        if i % 32 == 0 || i == n {
                            pr.set(format!("Leitura completa: {i}/{n}"));
                        }
                    })?;
                    snap.save(&path.to_string_lossy())?;
                    Ok((path, snap.eeprom.len(), snap.refused.len()))
                },
                |app, (path, read, refused)| {
                    app.ee_out = format!("Leitura salva em {}: {read} endereços lidos, {refused} recusados.", path.display());
                },
            )
        });
    }

    fn snapshot_compare(&mut self) {
        let Some(a) = rfd::FileDialog::new().set_title("Leitura de ANTES").add_filter("Leitura", &["json"]).pick_file() else { return };
        let Some(b) = rfd::FileDialog::new().set_title("Leitura de DEPOIS").add_filter("Leitura", &["json"]).pick_file() else { return };
        let (before, after) = match (snapshot::Snapshot::load(&a.to_string_lossy()), snapshot::Snapshot::load(&b.to_string_lossy())) {
            (Ok(x), Ok(y)) => (x, y),
            (Err(e), _) | (_, Err(e)) => return self.error(e),
        };
        let (changes, warnings) = snapshot::compare(&before, &after);
        let mut out: Vec<String> = warnings.iter().map(|w| format!("Atenção: {w}")).collect();
        out.push(format!("{} endereço(s) mudaram entre '{}' e '{}':", changes.len(), before.label, after.label));
        out.extend(changes.iter().map(|(a, o, n)| format!("{a:5} (0x{a:04X}): {o:3} -> {n:3}   {}", snapshot::classify(*a, *o, *n))));
        if !changes.is_empty() {
            let range = after.model.as_deref().and_then(|n| by_name(n, &self.models)).map(|m| m.serial_range()).unwrap_or_default();
            let s = snapshot::suggest_model(&before, &after, &changes, &range);
            out.push(format!("\nReset sugerido: {}", s["raw_waste_reset"]));
            self.confirm("Salvar a sugestão de modelo em arquivo?", move |app| {
                if let Some(p) = rfd::FileDialog::new().set_file_name("modelo-sugerido.json").save_file()
                    && let Err(e) = std::fs::write(p, serde_json::to_string_pretty(&s).unwrap()) {
                        app.error(e);
                    }
            });
        }
        self.ee_out = out.join("\n");
    }

    fn find_key(&mut self) {
        if self.busy.is_some() && self.can_cancel {
            self.cancel.store(true, Ordering::Relaxed);
            return;
        }
        let models = self.models.clone();
        self.session(
            "Procurando a chave",
            move |p, pr, cancel| {
                keyfinder::find_read_key(p, &models, 0, &mut |i, n, k| {
                    if i % 64 == 0 {
                        pr.set(format!("Procurando a chave: {i}/{n} (testando {},{})", k[0], k[1]));
                    }
                }, cancel)
            },
            |app, result| {
                let Some(r) = result else {
                    app.ee_out = "Busca interrompida.".into();
                    return;
                };
                app.manual_key = format!("{},{}", r.read_key[0], r.read_key[1]);
                let mut out = vec![format!("Chave de leitura encontrada: {},{} (em {} tentativas)", r.read_key[0], r.read_key[1], r.tries)];
                if r.known {
                    out.push("É a chave de um modelo já conhecido: provavelmente da mesma família.".into());
                }
                if r.accepts_any {
                    out.push("Atenção: a impressora aceitou também outra chave. Ela parece não conferir a chave de leitura.".into());
                } else if !r.confirmed {
                    out.push("Atenção: a chave não se confirmou num segundo endereço.".into());
                }
                out.push("A chave já foi preenchida no campo. Agora dá para fazer a Leitura completa.".into());
                app.ee_out = out.join("\n");
                app.confirm(
                    "Testar também as chaves de GRAVAÇÃO conhecidas? Isso grava no endereço 0 o mesmo valor que já está lá, então não muda nada na impressora.",
                    |app| {
                        let models = app.models.clone();
                        app.session("Testando chaves de gravação", move |p, _, _| keyfinder::find_write_key(p, &models, 0), |app, k| {
                            app.ee_out += &match k {
                                Some(k) => format!("\nChave de gravação aceita: {} (de um modelo conhecido).", String::from_utf8_lossy(&k)),
                                None => "\nNenhuma chave de gravação conhecida foi aceita. Este modelo usa uma chave nova.".into(),
                            };
                        });
                    },
                );
            },
        );
        if self.busy.is_some() {
            self.can_cancel = true;
        }
    }

    // ---- ações: banco de modelos -------------------------------------------------

    fn refresh_library(&mut self) {
        self.learned = library::load().unwrap_or_default();
        self.models = all_models();
    }

    fn sync_library(&mut self, quiet: bool) {
        if !server::configured() {
            if !quiet {
                self.choose_server();
            }
            return;
        }
        self.spawn(
            "Atualizando o banco",
            move |_, _| Ok(server::sync().map_err(|e| e.to_string())),
            move |app, r: std::result::Result<(usize, usize, Vec<String>), String>| {
                app.refresh_library();
                match r {
                    Ok((sent, total, new)) => {
                        app.status = format!(
                            "Banco atualizado: {total} modelo(s) no servidor{}{}.",
                            if sent > 0 { format!(", {sent} enviado(s)") } else { String::new() },
                            if new.is_empty() { String::new() } else { format!(", novos: {}", new.join(", ")) }
                        )
                    }
                    // ao abrir: sem janela de erro se estiver sem internet
                    Err(e) if quiet => app.status = format!("Banco não atualizado: {e}"),
                    Err(e) => app.error(e),
                }
            },
        );
    }

    fn choose_server(&mut self) {
        let (url, _) = server::settings();
        self.input("Endereço do servidor:", &url, false, |app, url| {
            app.input("Chave do EcoReset no servidor:", "", true, move |app, token| {
                if token.trim().is_empty() {
                    return;
                }
                match server::configure(&url, &token) {
                    Ok(()) => {
                        app.refresh_library();
                        app.sync_library(false);
                    }
                    Err(e) => app.error(e),
                }
            })
        });
    }

    fn import_wic_log(&mut self) {
        let Some(path) = rfd::FileDialog::new().set_title("application.log do WICReset").add_filter("Log", &["log", "txt"]).pick_file() else {
            return;
        };
        let (model, warnings) = match wiclog::learn_file(&path.to_string_lossy(), None, wiclog::TRIAL_PERCENT, &self.models) {
            Ok(v) => v,
            Err(e) => return self.error(e),
        };
        let suggested = model.name.clone();
        self.input("Nome do modelo:", &suggested, false, move |app, name| {
            let name = name.trim().to_uppercase();
            if name.is_empty() {
                return;
            }
            let mut m = model.clone();
            m.name = name.clone();
            let mut lines = vec![
                format!("Chave de leitura: {},{}", m.read_key[0], m.read_key[1]),
                format!("Chave de gravação: {}", m.write_key_text()),
                format!("Reset grava {} endereços", m.raw_waste_reset.len()),
            ];
            lines.extend(warnings.iter().map(|w| format!("Atenção: {w}")));
            if by_name(&name, &ecoreset_core::models::builtin()).is_some() {
                lines.push("Este nome já está na lista do app: o aprendido vai valer no lugar dele.".into());
            }
            app.confirm(format!("{}\n\nSalvar no banco?", lines.join("\n")), move |app| {
                app.spawn("Salvando o modelo", move |_, _| server::publish(&m), move |app, problem| {
                    app.refresh_library();
                    app.model = name;
                    if let Some(p) = problem {
                        app.message(format!("Salvo nesta máquina, mas não foi ao servidor: {p}"));
                    }
                });
            });
        });
    }

    fn remove_learned(&mut self) {
        let Some(name) = self.selected.clone() else { return self.error("Escolha um modelo na lista.") };
        self.confirm(format!("Apagar {name} do banco (e do servidor)?"), move |app| {
            app.spawn("Apagando", move |_, _| server::remove(&name), |app, _| {
                app.selected = None;
                app.refresh_library();
            });
        });
    }

    fn choose_folder(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().set_title("Pasta do banco de modelos").pick_folder() {
            match library::set_folder(&dir.to_string_lossy()) {
                Ok(_) => self.refresh_library(),
                Err(e) => self.error(e),
            }
        }
    }

    fn merge_library(&mut self) {
        let Some(path) = rfd::FileDialog::new().set_title("modelos.json de outra máquina").add_filter("Banco", &["json"]).pick_file() else {
            return;
        };
        match library::merge_file(&path.to_string_lossy()) {
            Ok(new) => {
                self.refresh_library();
                self.message(format!("Novos: {}", if new.is_empty() { "nenhum".into() } else { new.join(", ") }));
            }
            Err(e) => self.error(e),
        }
    }

    // ---- tela ---------------------------------------------------------------------

    fn top(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(&mut self.usb, true, "Cabo USB");
            ui.radio_value(&mut self.usb, false, "Rede, IP:");
            ui.add(egui::TextEdit::singleline(&mut self.host).desired_width(130.0).hint_text("192.168.0.50"));
            if ui.button("Detectar").clicked() {
                self.detect();
            }
            ui.separator();
            let label = ui.label("Modelo:");
            let shown = if self.model.is_empty() { "automático".to_string() } else { self.model.clone() };
            egui::ComboBox::from_id_salt("modelo")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.model, String::new(), "automático");
                    for n in names(&self.models) {
                        ui.selectable_value(&mut self.model, n.clone(), n);
                    }
                })
                .response
                .labelled_by(label.id);
            ui.separator();
            ui.label("Marca:");
            let before = self.brand;
            ui.radio_value(&mut self.brand, Brand::Epson, "Epson");
            ui.radio_value(&mut self.brand, Brand::Canon, "Canon");
            if before != self.brand {
                self.brand_changed();
            }
        });
        ui.label(RichText::new(&self.info).color(MUTED));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (tab, label) in [
                (Tab::Diagnose, "Diagnóstico"),
                (Tab::Waste, "Almofada de tinta"),
                (Tab::Maintenance, "Manutenção"),
                (Tab::Eeprom, "EEPROM"),
                (Tab::Library, "Modelos aprendidos"),
            ] {
                ui.selectable_value(&mut self.tab, tab, label);
            }
        });
        ui.add_space(4.0);
    }

    fn brand_changed(&mut self) {
        let wanted = if self.brand == Brand::Canon { "CANON" } else { "EPSON" };
        if self.brand == Brand::Canon {
            self.model.clear();
            self.info = "Canon: diagnóstico (códigos de erro) e manutenção. O contador da almofada da Canon só zera com a \
                         ferramenta de serviço da marca; modelos com caixa de manutenção (G2060...) só precisam trocar a caixa."
                .into();
        } else {
            self.info = "Escolha a conexão e clique em Detectar.".into();
        }
        if let Some(q) = self.printers.iter().find(|n| n.to_uppercase().contains(wanted)) {
            self.queue = q.clone();
        }
    }

    fn tab_diagnose(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Diagnosticar").clicked() {
                self.diagnose();
            }
            ui.separator();
            ui.label("Consultar código ou palavra:");
            let r = ui.add(egui::TextEdit::singleline(&mut self.lookup).desired_width(140.0).hint_text("1726, 0x10, almofada"));
            if ui.button("Consultar").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                self.lookup_code();
            }
        });
        ui.add_space(8.0);
        for (text, problems) in &self.diag {
            ui.label(text);
            for p in problems {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(format!("{} {}: {}", p.brand, p.code, p.title)).strong());
                    ui.label(format!("Causa: {}", p.cause));
                    ui.label(format!("Correção: {}", p.fix));
                    if p.service {
                        ui.label(RichText::new("Pode exigir assistência técnica.").color(WARN));
                    }
                });
            }
        }
    }

    fn tab_waste(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Ler contadores").clicked() {
                self.read_levels();
            }
            if ui.button("Zerar contadores…").clicked() {
                self.reset();
            }
            if ui.button("Reset temporário…").clicked() {
                self.temp_reset();
            }
            if ui.button("Restaurar backup…").clicked() {
                self.restore();
            }
        });
        ui.add_space(10.0);
        for (label, pct) in &self.levels {
            ui.label(format!("Almofada {label}: {pct:.2}%"));
            let color = if *pct >= 100.0 { WARN } else { ACCENT };
            ui.add(egui::ProgressBar::new((*pct / 100.0).clamp(0.0, 1.0) as f32).fill(color));
        }
        ui.add_space(10.0);
        ui.label(RichText::new(
            "O reset salva antes um backup dos valores atuais (pasta de backups do EcoReset). O reset temporário serve \
             para firmwares que bloqueiam a gravação: libera a impressora por um tempo, sem mexer na EEPROM.",
        ).color(MUTED));
    }

    fn tab_maintenance(&mut self, ui: &mut egui::Ui) {
        if self.brand == Brand::Canon && self.usb {
            let label = ui.label("Canon pelo cabo USB (IVEC):");
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("ivec")
                    .selected_text(&self.ivec_op)
                    .show_ui(ui, |ui| {
                        for n in ivec::maintenance_names() {
                            ui.selectable_value(&mut self.ivec_op, n.to_string(), n);
                        }
                    })
                    .response
                    .labelled_by(label.id);
                if ui.button("Executar").clicked() {
                    let op = self.ivec_op.clone();
                    self.maintain_ivec(op);
                }
            });
            ui.add_space(10.0);
        } else {
            ui.radio_value(&mut self.via_installed, false, "Pela rede (usa o IP acima)");
            ui.horizontal(|ui| {
                let label = ui.radio_value(&mut self.via_installed, true, "Impressora instalada no computador (USB ou rede):");
                egui::ComboBox::from_id_salt("fila")
                    .selected_text(&self.queue)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for n in self.printers.clone() {
                            ui.selectable_value(&mut self.queue, n.clone(), n);
                        }
                    })
                    .response
                    .labelled_by(label.id);
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Teste de bicos").clicked() {
                    self.maintain(true);
                }
                if ui.button("Limpeza de cabeça").clicked() {
                    self.maintain(false);
                }
            });
        }
        ui.add_space(10.0);
        ui.label(RichText::new(
            "Coloque papel A4 antes do teste de bicos. Cada limpeza gasta tinta e enche a almofada: faça no máximo duas \
             seguidas e espere algumas horas antes de repetir. Se a impressora travar, segure o botão liga/desliga por \
             alguns segundos antes de tirar da tomada.",
        ).color(MUTED));
    }

    fn tab_eeprom(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Endereço:");
            ui.add(egui::TextEdit::singleline(&mut self.addr).desired_width(70.0));
            ui.label("até:");
            ui.add(egui::TextEdit::singleline(&mut self.addr_end).desired_width(70.0));
            ui.label("Valor:");
            ui.add(egui::TextEdit::singleline(&mut self.value).desired_width(50.0));
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Ler").clicked() {
                self.ee_read();
            }
            if ui.button("Gravar…").clicked() {
                self.ee_write();
            }
            if ui.button("Salvar leitura…").clicked() {
                self.ee_save();
            }
            if ui.button("Regravar de arquivo…").clicked() {
                self.ee_image();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Leitura completa…").clicked() {
                self.snapshot_take();
            }
            if ui.button("Comparar leituras…").clicked() {
                self.snapshot_compare();
            }
            ui.label("Chave de leitura (modelo fora da lista):");
            ui.add(egui::TextEdit::singleline(&mut self.manual_key).desired_width(60.0).hint_text("74,54"));
            let searching = self.busy.is_some() && self.can_cancel;
            if ui.button(if searching { "Parar a busca" } else { "Descobrir chave…" }).clicked() {
                self.find_key();
            }
        });
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut self.ee_out).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(14));
        });
        ui.label(RichText::new(
            "Avançado: gravar endereços errados pode travar a impressora. Para descobrir o reset de um modelo novo: faça \
             uma leitura completa, zere a impressora com outra ferramenta, faça outra leitura e compare.",
        ).color(WARN));
    }

    fn tab_library(&mut self, ui: &mut egui::Ui) {
        let (url, token) = server::settings();
        egui::Grid::new("lib").num_columns(2).show(ui, |ui| {
            ui.label("Servidor:");
            ui.label(RichText::new(if token.is_empty() { "não configurado (clique em Servidor…)".into() } else { url }).color(MUTED));
            ui.end_row();
            ui.label("Cópia local:");
            ui.label(RichText::new(library::db_path().display().to_string()).color(MUTED));
            ui.end_row();
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Atualizar").clicked() {
                self.sync_library(false);
            }
            if ui.button("Importar log do WICReset…").clicked() {
                self.import_wic_log();
            }
            if ui.button("Apagar").clicked() {
                self.remove_learned();
            }
            if ui.button("Servidor…").clicked() {
                self.choose_server();
            }
            if ui.button("Pasta…").clicked() {
                self.choose_folder();
            }
            if ui.button("Juntar arquivo…").clicked() {
                self.merge_library();
            }
        });
        ui.add_space(6.0);
        egui::Grid::new("learned").striped(true).num_columns(4).min_col_width(120.0).show(ui, |ui| {
            for h in ["Modelo", "Chave de leitura", "Aprendido em", "Data"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for m in &self.learned {
                let l = m.learned.clone().unwrap_or_default();
                let selected = self.selected.as_deref() == Some(m.name.as_str());
                if ui.selectable_label(selected, &m.name).clicked() {
                    self.selected = Some(m.name.clone());
                }
                ui.label(format!("{},{}", m.read_key[0], m.read_key[1]));
                ui.label(&l.on);
                ui.label(format!("{}{}", l.at, if l.pending { "  (não enviado)" } else { "" }));
                ui.end_row();
            }
        });
        if self.learned.is_empty() {
            ui.label(RichText::new("Nenhum modelo aprendido ainda.").color(MUTED));
        }
        ui.add_space(10.0);
        ui.label(RichText::new(
            "Faça um reset (pode ser o trial gratuito) no WICReset e importe o application.log dele: o EcoReset aprende as \
             chaves e o reset do modelo e passa a zerar sozinho pelo cabo USB. Com o servidor configurado, o modelo vai \
             para o banco do servidor e as outras máquinas o recebem ao clicar em Atualizar (o EcoReset também atualiza \
             ao abrir).",
        ).color(MUTED));
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialogs.first_mut() else { return };
        let mut answer: Option<bool> = None;
        egui::Modal::new(egui::Id::unique("dialog")).show(ctx, |ui| {
            ui.set_max_width(520.0);
            match dialog {
                Dialog::Message { text, error } => {
                    ui.label(if *error { RichText::new(text.as_str()).color(WARN) } else { RichText::new(text.as_str()) });
                    ui.add_space(8.0);
                    if ui.button("OK").clicked() {
                        answer = Some(true);
                    }
                }
                Dialog::Confirm { text, .. } => {
                    ui.label(text.as_str());
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Sim").clicked() {
                            answer = Some(true);
                        }
                        if ui.button("Não").clicked() {
                            answer = Some(false);
                        }
                    });
                }
                Dialog::Input { prompt, value, secret, .. } => {
                    ui.label(prompt.as_str());
                    let r = ui.add(egui::TextEdit::singleline(value).password(*secret).desired_width(400.0));
                    r.request_focus();
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() || enter {
                            answer = Some(true);
                        }
                        if ui.button("Cancelar").clicked() {
                            answer = Some(false);
                        }
                    });
                }
            }
        });
        let Some(yes) = answer else { return };
        match self.dialogs.remove(0) {
            Dialog::Confirm { yes: f, .. } if yes => f(self),
            Dialog::Input { value, ok, .. } if yes => ok(self, value),
            _ => {}
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        let ctx = ui.ctx().clone();
        egui::Panel::top("topo").show(ui, |ui| self.top(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let progress = self.progress.lock().unwrap().clone();
                let text = if self.busy.is_some() && !progress.is_empty() { progress } else { self.busy.clone().unwrap_or(self.status.clone()) };
                if self.busy.is_some() {
                    ui.spinner();
                }
                ui.label(RichText::new(text).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("EcoReset {}", ecoreset_core::VERSION)).small().color(MUTED));
                });
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.tab {
                Tab::Diagnose => self.tab_diagnose(ui),
                Tab::Waste => self.tab_waste(ui),
                Tab::Maintenance => self.tab_maintenance(ui),
                Tab::Eeprom => self.tab_eeprom(ui),
                Tab::Library => self.tab_library(ui),
            });
        });
        self.dialogs(&ctx);
    }
}

/// Renderiza as abas fora da tela, para conferir o layout:
/// `SHOTS=/pasta cargo test -p ecoreset -- --ignored screenshots`
#[cfg(test)]
mod screens {
    use super::*;
    use egui_kittest::Harness;

    #[test]
    #[ignore]
    fn screenshots() {
        let out = PathBuf::from(std::env::var("SHOTS").unwrap_or("target/shots".into()));
        std::fs::create_dir_all(&out).unwrap();
        let mut hs = Harness::builder().with_size([900.0, 680.0]).wgpu().build_eframe(|cc| App::new(cc));
        let shot = |hs: &mut Harness<App>, name: &str| {
            hs.run_steps(4);
            hs.render().unwrap().save(out.join(format!("{name}.png"))).unwrap();
        };
        hs.state_mut().lookup = "1726".into();
        hs.state_mut().lookup_code();
        shot(&mut hs, "1-diagnostico");
        hs.state_mut().tab = Tab::Waste;
        hs.state_mut().levels = vec![("principal".into(), 80.0), ("sem margem".into(), 80.01), ("terceira".into(), 80.0)];
        shot(&mut hs, "2-almofada");
        hs.state_mut().tab = Tab::Maintenance;
        shot(&mut hs, "3-manutencao");
        hs.state_mut().tab = Tab::Eeprom;
        hs.state_mut().ee_out = "   48 (0x0030):   0 (0x00)\n   49 (0x0031):   0 (0x00)".into();
        shot(&mut hs, "4-eeprom");
        hs.state_mut().tab = Tab::Library;
        shot(&mut hs, "5-modelos");
        hs.state_mut().confirm("Zerar os contadores da almofada (ET-2812)?", |_| {});
        shot(&mut hs, "6-dialogo");
    }
}
