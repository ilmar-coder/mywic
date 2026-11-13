//! EcoReset: tela (egui) para Windows, Mac e Linux.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, RichText, Stroke, Vec2, vec2};
use ecoreset_core::errors::{Problem, explain_epson_error, search};
use ecoreset_core::models::{Model, by_name, names};
use ecoreset_core::printer::{Backup, EpsonPrinter, load_image, parse_int};
use ecoreset_core::usb::UsbInfo;
use ecoreset_core::{Error, Result, all_models, canon, ipp, ivec, keyfinder, library, maintenance, server, snapshot, usb, wiclog};

mod ui_kit;
use ui_kit::*;

const INFO_EPSON: &str = "Escolha a conexão e clique em Detectar. Pelo cabo USB funciona até em firmwares que \
                          bloqueiam a rede (como o da L3250).";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1060.0, 720.0])
            .with_min_inner_size([820.0, 560.0])
            .with_title(format!("EcoReset {}", ecoreset_core::VERSION))
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../../../packaging/icon/ecoreset-256.png")).unwrap_or_default()),
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

/// Registro de uso e erros (EcoReset/ecoreset.log), para mandar quando algo falhar.
fn log_path() -> PathBuf {
    library::folder().join("ecoreset.log")
}

fn log_line(text: &str) {
    use std::io::Write;
    let path = log_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // guarda só o fim do registro para não crescer sem limite
    if std::fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {}", ecoreset_core::printer::now_iso(), text);
    }
}

/// Abre um arquivo ou pasta no programa padrão do sistema.
fn open_path(path: &std::path::Path) {
    let program = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(program).arg(path).spawn();
}

/// Impressora detectada, mostrada na barra lateral.
struct Detected {
    text: String,
    model: Option<String>,
    rows: Vec<(String, String)>,
}

/// Resultado do diagnóstico ou da consulta de código.
struct DiagView {
    title: String,
    lines: Vec<String>,
    problems: Vec<Problem>,
}

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
    detected: Option<Detected>,
    // diagnóstico
    diag: Option<DiagView>,
    lookup: String,
    // almofada
    levels: Vec<(String, f64)>,
    // manutenção
    via_installed: bool,
    printers: Vec<String>,
    queue: String,
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
        apply_theme(&cc.egui_ctx);
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
            info: INFO_EPSON.into(),
            detected: None,
            diag: None,
            lookup: String::new(),
            levels: Vec::new(),
            via_installed: !printers.is_empty(),
            printers,
            queue,
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
        if self.tab == Tab::Eeprom {
            self.ee_out = format!("Erro: {e}");
        }
        self.status = "Erro.".into();
        log_line(&format!("ERRO {}", e));
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
        let label = label.to_string();
        log_line(&format!("início: {label} ({})", self.where_text()));
        let progress = Progress { text: self.progress.clone(), ctx: self.ctx.clone() };
        std::thread::spawn(move || {
            let result = work(&progress, &cancel);
            let job: Job = Box::new(move |app: &mut App| {
                app.busy = None;
                app.can_cancel = false;
                match result {
                    Ok(v) => {
                        app.status = "Pronto.".into();
                        log_line(&format!("ok: {label}"));
                        done(app, v);
                    }
                    Err(e) => app.error(format!("{label}: {e}")),
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
                let mut rows = vec![("Conexão".to_string(), app.where_text())];
                if !port.is_empty() {
                    rows.push(("Porta".into(), port));
                }
                if let Some(q) = queues.first() {
                    rows.push(("Instalada como".into(), q.clone()));
                    app.queue = q.clone(); // a mesma na aba Manutenção
                    app.via_installed = true;
                } else if usb_conn {
                    rows.push(("Instalada como".into(), "nenhuma nessa porta".into()));
                }
                if !serial.is_empty() {
                    rows.push(("Série".into(), serial));
                }
                rows.push(("Parâmetros".into(), name.clone().unwrap_or_else(|| "modelo fora da lista".into())));
                let text = if text.is_empty() { "Sem nome".to_string() } else { text };
                app.status = format!("Detectada: {text}");
                if let Some(n) = &name {
                    app.model = n.clone();
                }
                app.detected = Some(Detected { text, model: name, rows });
            },
        );
    }

    fn show_problems(&mut self, title: String, lines: Vec<String>, problems: Vec<Problem>) {
        self.diag = Some(DiagView { title, lines, problems });
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
        let Some((start, end)) = self.ee_range() else { return };
        self.session("Lendo EEPROM", move |p, _, _| p.dump(start, end), |app, data| app.show_dump(data));
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

    /// Lê o intervalo dos campos (se ainda não leu) e salva em arquivo.
    fn ee_save(&mut self) {
        if self.last_dump.is_empty() {
            let Some((start, end)) = self.ee_range() else { return };
            return self.session("Lendo EEPROM para salvar", move |p, _, _| p.dump(start, end), |app, data| {
                app.show_dump(data);
                app.ee_save();
            });
        }
        let name = format!("leitura-eeprom-{}.json", App::stamp());
        let Some(path) = rfd::FileDialog::new().set_title("Salvar leitura da EEPROM").set_file_name(name).add_filter("Leitura", &["json"]).save_file() else {
            return;
        };
        let b = Backup {
            printer: self.where_text(),
            model: Some(self.model.clone()).filter(|m| !m.is_empty()),
            date: ecoreset_core::printer::now_iso(),
            eeprom: self.last_dump.clone(),
        };
        match b.save(&path.to_string_lossy()) {
            Ok(()) => {
                self.status = format!("Leitura salva em {}", path.display());
                self.message(format!("{} endereço(s) salvos em:\n{}", b.eeprom.len(), path.display()));
            }
            Err(e) => self.error(e),
        }
    }

    fn show_dump(&mut self, data: Vec<(u16, u8)>) {
        self.ee_out = data.iter().map(|(a, v)| format!("{a:5} (0x{a:04X}): {v:3} (0x{v:02X})")).collect::<Vec<_>>().join("\n");
        self.last_dump = data;
    }

    fn ee_range(&mut self) -> Option<(u16, u16)> {
        let parsed = (|| -> Result<(u16, u16)> {
            let s = parse_int(&self.addr).filter(|n| *n <= 0xFFFF).ok_or_else(|| Error::new("endereço inválido (use decimal ou 0x..)"))? as u16;
            let e = if self.addr_end.trim().is_empty() {
                s
            } else {
                parse_int(&self.addr_end).filter(|n| *n <= 0xFFFF).ok_or_else(|| Error::new("endereço final inválido"))? as u16
            };
            if e < s {
                return Err(Error::new("o endereço final é menor que o inicial"));
            }
            Ok((s, e))
        })();
        match parsed {
            Ok(r) => Some(r),
            Err(e) => {
                self.error(e);
                None
            }
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

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(14.0);
        brand(ui);
        ui.add_space(14.0);
        for (tab, icon, label) in [
            (Tab::Diagnose, Icon::Pulse, "Diagnóstico"),
            (Tab::Waste, Icon::Drop, "Almofada de tinta"),
            (Tab::Maintenance, Icon::Wrench, "Manutenção"),
            (Tab::Eeprom, Icon::Chip, "EEPROM"),
            (Tab::Library, Icon::Database, "Modelos aprendidos"),
        ] {
            if nav_item(ui, icon, label, self.tab == tab).clicked() {
                self.tab = tab;
            }
        }
        ui.add_space(12.0);
        section_title(ui, "Impressora");
        let before = self.brand;
        segmented(ui, &mut self.brand, &[(Brand::Epson, Icon::Printer, "Epson"), (Brand::Canon, Icon::Printer, "Canon")], true);
        if before != self.brand {
            self.brand_changed();
        }
        segmented(ui, &mut self.usb, &[(true, Icon::Usb, "Cabo USB"), (false, Icon::Wifi, "Rede")], true);
        if !self.usb {
            field(ui, "Endereço IP", &mut self.host, "192.168.0.50", ui.available_width());
        }
        if self.brand == Brand::Epson {
            let label = ui.label(RichText::new("Modelo").small().color(MUTED));
            let shown = if self.model.is_empty() { "detectar automaticamente".to_string() } else { self.model.clone() };
            egui::ComboBox::from_id_salt("modelo")
                .selected_text(shown)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.model, String::new(), "detectar automaticamente");
                    for n in names(&self.models) {
                        ui.selectable_value(&mut self.model, n.clone(), n);
                    }
                })
                .response
                .labelled_by(label.id);
        }
        ui.add_space(2.0);
        let busy = self.busy.is_some();
        if button(ui, Kind::Primary, Some(Icon::Search), "Detectar", !busy, true).clicked() {
            self.detect();
        }
        ui.add_space(4.0);
        self.detected_card(ui);
        ui.add_space(12.0);
    }

    fn detected_card(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(BG).corner_radius(12).inner_margin(12).stroke(Stroke::new(1.0, BORDER)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match &self.detected {
                Some(d) => {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.5, if d.model.is_some() { GREEN } else { YELLOW });
                        ui.add(egui::Label::new(RichText::new(&d.text).strong()).truncate());
                    });
                    ui.spacing_mut().interact_size.y = 18.0;
                    egui::Grid::new("detectada").num_columns(2).spacing([10.0, 2.0]).show(ui, |ui| {
                        for (k, v) in &d.rows {
                            ui.label(RichText::new(k).small().color(MUTED));
                            ui.add(egui::Label::new(RichText::new(v).small()).truncate());
                            ui.end_row();
                        }
                    });
                }
                None => {
                    ui.add(egui::Label::new(RichText::new(&self.info).small().color(MUTED)).wrap());
                }
            }
        });
    }

    fn brand_changed(&mut self) {
        let wanted = if self.brand == Brand::Canon { "CANON" } else { "EPSON" };
        self.detected = None;
        if self.brand == Brand::Canon {
            self.model.clear();
            self.info = "Canon: diagnóstico (códigos de erro) e manutenção. O contador da almofada da Canon só zera \
                         com a ferramenta de serviço da marca; modelos com caixa de manutenção (G2060...) só precisam \
                         trocar a caixa."
                .into();
        } else {
            self.info = INFO_EPSON.into();
        }
        if let Some(q) = self.printers.iter().find(|n| n.to_uppercase().contains(wanted)) {
            self.queue = q.clone();
        }
    }

    fn tab_diagnose(&mut self, ui: &mut egui::Ui) {
        page_header(ui, Icon::Pulse, "Diagnóstico", "O erro atual da impressora, com a causa e o que fazer.");
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if primary(ui, Icon::Pulse, "Diagnosticar a impressora", self.busy.is_none()).clicked() {
                    self.diagnose();
                }
                ui.label(RichText::new(if self.usb { "pelo cabo USB" } else { "pela rede" }).color(MUTED));
            });
        });
        ui.add_space(8.0);
        card(ui, |ui| {
            section_title(ui, "Consultar um código");
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.lookup)
                        .hint_text("1726, 5B00, 0x10 ou uma palavra (papel, almofada)")
                        .desired_width((ui.available_width() - 130.0).max(160.0))
                        .margin(vec2(8.0, 7.0)),
                );
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if secondary(ui, Icon::Search, "Consultar", true).clicked() || enter {
                    self.lookup_code();
                }
            });
        });
        ui.add_space(8.0);
        let Some(d) = &self.diag else {
            return empty_state(ui, Icon::Pulse, "Nenhum diagnóstico ainda", "Clique em Diagnosticar ou consulte um código.");
        };
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let (icon, c) = if d.problems.is_empty() { (Icon::Check, GREEN) } else { (Icon::Alert, ORANGE) };
                let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                paint_icon(ui.painter(), r, icon, c);
                ui.label(RichText::new(&d.title).strong().size(15.0));
            });
            for l in &d.lines {
                ui.label(RichText::new(l).color(MUTED));
            }
        });
        for p in &d.problems {
            ui.add_space(6.0);
            problem_card(ui, &p.code, &p.brand, &p.title, &p.cause, &p.fix, p.service);
        }
    }

    fn tab_waste(&mut self, ui: &mut egui::Ui) {
        page_header(ui, Icon::Drop, "Almofada de tinta", "Contadores da almofada que recolhe a tinta das limpezas.");
        if self.brand == Brand::Canon {
            banner(ui, Icon::Info, BLUE, "Na Canon o contador da almofada só zera com a ferramenta de serviço da marca. Modelos com caixa de manutenção (G2060, G3160...) só precisam trocar a caixa: o nível dela aparece no Diagnóstico.");
            return;
        }
        let busy = self.busy.is_some();
        let locked = by_name(&self.model, &self.models).is_some_and(|m| m.may_be_locked);
        if !self.usb && locked {
            banner(ui, Icon::Usb, ORANGE, "Este modelo (família da L3250) bloqueia a leitura e a gravação dos contadores pela rede, por causa do firmware. Ligue o cabo USB e escolha \"Cabo USB\" na barra lateral. Pela rede só o reset temporário funciona.");
            ui.add_space(8.0);
        }
        card(ui, |ui| {
            if self.levels.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Leia os contadores para ver quanto a almofada está cheia.").color(MUTED));
                    ui.add_space(6.0);
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    for (label, pct) in &self.levels {
                        gauge(ui, label, *pct);
                    }
                });
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if primary(ui, Icon::Refresh, "Ler contadores", !busy).clicked() {
                    self.read_levels();
                }
                if danger(ui, Icon::Reset, "Zerar contadores", !busy).clicked() {
                    self.reset();
                }
            });
        });
        ui.add_space(8.0);
        card(ui, |ui| {
            section_title(ui, "Outras opções");
            ui.horizontal_wrapped(|ui| {
                if secondary(ui, Icon::Key, "Reset temporário", !busy).clicked() {
                    self.temp_reset();
                }
                if secondary(ui, Icon::Undo, "Restaurar backup", !busy).clicked() {
                    self.restore();
                }
            });
            ui.add_space(2.0);
            ui.label(RichText::new(
                "O reset temporário serve para firmwares que bloqueiam a gravação: libera a impressora por um tempo, sem mexer na EEPROM.",
            ).small().color(MUTED));
        });
        ui.add_space(8.0);
        banner(ui, Icon::Save, ACCENT, &format!("Antes de zerar, o EcoReset salva um backup dos valores atuais em {}.", App::backup_dir().display()));
    }

    fn tab_maintenance(&mut self, ui: &mut egui::Ui) {
        page_header(ui, Icon::Wrench, "Manutenção", "Teste de bicos e limpeza da cabeça de impressão.");
        let busy = self.busy.is_some();
        if self.brand == Brand::Canon && self.usb {
            card(ui, |ui| {
                section_title(ui, "Canon pelo cabo USB (IVEC)");
                for (op, icon, label, help) in [
                    ("teste-bicos", Icon::Nozzle, "Teste de bicos", "imprime o padrão para conferir os bicos"),
                    ("limpeza", Icon::Sparkle, "Limpeza", "limpeza normal de todas as cores"),
                    ("limpeza-preto", Icon::Sparkle, "Limpeza do preto", "só o cartucho preto"),
                    ("limpeza-cor", Icon::Sparkle, "Limpeza das cores", "só o colorido"),
                    ("limpeza-profunda", Icon::Sparkle, "Limpeza profunda", "gasta bastante tinta"),
                    ("ink-flush", Icon::Drop, "Ink flush", "troca a tinta das mangueiras; gasta muita tinta"),
                    ("rolete", Icon::Refresh, "Limpeza do rolete", "para papel que não puxa"),
                    ("placa-inferior", Icon::Refresh, "Limpeza da placa inferior", "para manchas no verso"),
                ] {
                    ui.horizontal(|ui| {
                        let clicked = ui
                            .allocate_ui(vec2(240.0, 36.0), |ui| button(ui, Kind::Secondary, Some(icon), label, !busy, true))
                            .inner
                            .clicked();
                        if clicked {
                            self.maintain_ivec(op.into());
                        }
                        ui.label(RichText::new(help).small().color(MUTED));
                    });
                }
            });
        } else {
            card(ui, |ui| {
                section_title(ui, "Enviar para");
                segmented(ui, &mut self.via_installed, &[(true, Icon::Printer, "Impressora instalada"), (false, Icon::Wifi, "Rede (IP)")], false);
                ui.add_space(4.0);
                if self.via_installed {
                    let label = ui.label(RichText::new("Impressora do computador (USB ou rede)").small().color(MUTED));
                    egui::ComboBox::from_id_salt("fila")
                        .selected_text(if self.queue.is_empty() { "nenhuma instalada" } else { &self.queue })
                        .width(ui.available_width().min(420.0))
                        .show_ui(ui, |ui| {
                            for n in self.printers.clone() {
                                ui.selectable_value(&mut self.queue, n.clone(), n);
                            }
                        })
                        .response
                        .labelled_by(label.id);
                } else {
                    field(ui, "Endereço IP", &mut self.host, "192.168.0.50", 220.0);
                }
            });
            ui.add_space(8.0);
            ui.columns(2, |cols| {
                for (i, (nozzle, icon, title, text)) in [
                    (true, Icon::Nozzle, "Teste de bicos", "Imprime o padrão de bicos. Coloque papel A4."),
                    (false, Icon::Sparkle, "Limpeza de cabeça", "Limpa os bicos entupidos. Gasta tinta."),
                ]
                .into_iter()
                .enumerate()
                {
                    card(&mut cols[i], |ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::hover());
                        paint_icon(ui.painter(), r.shrink(4.0), icon, ACCENT);
                        ui.label(RichText::new(title).strong().size(15.0));
                        ui.label(RichText::new(text).color(MUTED));
                        ui.add_space(4.0);
                        if button(ui, if nozzle { Kind::Primary } else { Kind::Secondary }, Some(icon), if nozzle { "Imprimir teste" } else { "Limpar" }, !busy, true).clicked() {
                            self.maintain(nozzle);
                        }
                    });
                }
            });
        }
        ui.add_space(8.0);
        banner(ui, Icon::Alert, ORANGE, "Cada limpeza gasta tinta e enche a almofada: faça no máximo duas seguidas e espere algumas horas antes de repetir. Se a impressora travar, segure o botão liga/desliga por alguns segundos antes de tirar da tomada.");
    }

    fn tab_eeprom(&mut self, ui: &mut egui::Ui) {
        page_header(ui, Icon::Chip, "EEPROM", "Leitura e gravação da memória da Epson. Modo avançado.");
        let busy = self.busy.is_some();
        card(ui, |ui| {
            section_title(ui, "Ler e gravar");
            ui.horizontal_wrapped(|ui| {
                field(ui, "Endereço", &mut self.addr, "0 ou 0x30", 90.0);
                field(ui, "Até (opcional)", &mut self.addr_end, "", 90.0);
                field(ui, "Valor", &mut self.value, "0-255", 70.0);
            });
            ui.horizontal_wrapped(|ui| {
                if primary(ui, Icon::Download, "Ler", !busy).clicked() {
                    self.ee_read();
                }
                if secondary(ui, Icon::Pen, "Gravar", !busy).clicked() {
                    self.ee_write();
                }
                if secondary(ui, Icon::Save, "Salvar leitura", !busy).clicked() {
                    self.ee_save();
                }
                if secondary(ui, Icon::Upload, "Regravar de arquivo", !busy).clicked() {
                    self.ee_image();
                }
            });
        });
        ui.add_space(8.0);
        card(ui, |ui| {
            section_title(ui, "Modelos fora da lista");
            ui.horizontal_wrapped(|ui| {
                if secondary(ui, Icon::File, "Leitura completa", !busy).clicked() {
                    self.snapshot_take();
                }
                if secondary(ui, Icon::Compare, "Comparar leituras", true).clicked() {
                    self.snapshot_compare();
                }
            });
            ui.horizontal_wrapped(|ui| {
                field(ui, "Chave de leitura", &mut self.manual_key, "74,54", 90.0);
                ui.vertical(|ui| {
                    ui.add_space(17.0);
                    let searching = busy && self.can_cancel;
                    let r = if searching {
                        danger(ui, Icon::Stop, "Parar a busca", true)
                    } else {
                        secondary(ui, Icon::Key, "Descobrir chave", !busy)
                    };
                    if r.clicked() {
                        self.find_key();
                    }
                });
            });
            ui.label(RichText::new(
                "Para descobrir o reset de um modelo novo: faça uma leitura completa, zere a impressora com outra ferramenta, faça outra leitura e compare.",
            ).small().color(MUTED));
        });
        ui.add_space(8.0);
        if !self.ee_out.is_empty() {
            egui::Frame::new().fill(Color32::from_rgb(10, 13, 18)).corner_radius(12).inner_margin(12).stroke(Stroke::new(1.0, BORDER)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut self.ee_out).font(egui::TextStyle::Monospace).frame(egui::Frame::NONE).desired_width(f32::INFINITY));
                });
            });
            ui.add_space(8.0);
        }
        banner(ui, Icon::Alert, RED, "Gravar endereços errados pode travar a impressora. O EcoReset protege o número de série e o MAC ao regravar de arquivo e salva um backup antes.");
    }

    fn tab_library(&mut self, ui: &mut egui::Ui) {
        page_header(ui, Icon::Database, "Modelos aprendidos", "Modelos ensinados pelo log do WICReset, divididos com as outras máquinas.");
        let (url, token) = server::settings();
        let busy = self.busy.is_some();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::hover());
                paint_icon(ui.painter(), r, Icon::Cloud, if token.is_empty() { MUTED } else { ACCENT });
                ui.label(RichText::new("Banco no servidor").strong());
                if token.is_empty() { pill(ui, "não configurado", ORANGE) } else { pill(ui, "conectado", GREEN) }
            });
            if !token.is_empty() {
                ui.label(RichText::new(&url).color(MUTED));
            }
            ui.label(RichText::new(format!("Cópia local: {}", library::db_path().display())).small().color(MUTED));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if primary(ui, Icon::Refresh, "Atualizar", !busy).clicked() {
                    self.sync_library(false);
                }
                if secondary(ui, Icon::Upload, "Importar log do WICReset", !busy).clicked() {
                    self.import_wic_log();
                }
                if button(ui, Kind::Ghost, Some(Icon::Cloud), "Servidor", true, false).clicked() {
                    self.choose_server();
                }
                if button(ui, Kind::Ghost, Some(Icon::Folder), "Pasta", true, false).clicked() {
                    self.choose_folder();
                }
                if button(ui, Kind::Ghost, Some(Icon::File), "Juntar arquivo", true, false).clicked() {
                    self.merge_library();
                }
            });
        });
        ui.add_space(8.0);
        if self.learned.is_empty() {
            empty_state(
                ui,
                Icon::Database,
                "Nenhum modelo aprendido ainda",
                "Faça um reset (pode ser o trial gratuito) no WICReset e importe o application.log: o EcoReset aprende as chaves e o reset do modelo e passa a zerar sozinho pelo cabo USB.",
            );
            return;
        }
        let mut remove = None;
        for m in &self.learned {
            let l = m.learned.clone().unwrap_or_default();
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::hover());
                    paint_icon(ui.painter(), r, Icon::Printer, ACCENT);
                    ui.label(RichText::new(&m.name).strong().size(15.0));
                    if l.pending {
                        pill(ui, "não enviado", ORANGE);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if button(ui, Kind::Ghost, Some(Icon::Trash), "Apagar", !busy, false).clicked() {
                            remove = Some(m.name.clone());
                        }
                    });
                });
                ui.label(RichText::new(format!(
                    "Chave {},{} · reset grava {} endereços · aprendido em {} {}",
                    m.read_key[0],
                    m.read_key[1],
                    m.raw_waste_reset.len(),
                    l.on,
                    l.at.replace('T', " ")
                )).small().color(MUTED));
            });
            ui.add_space(6.0);
        }
        if let Some(name) = remove {
            self.selected = Some(name);
            self.remove_learned();
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialogs.first_mut() else { return };
        let mut answer: Option<bool> = None;
        egui::Modal::new(egui::Id::unique("dialog"))
            .frame(egui::Frame::new().fill(PANEL).corner_radius(16).inner_margin(20).stroke(Stroke::new(1.0, BORDER)))
            .show(ctx, |ui| {
                ui.set_width(440.0);
                let (icon, color, title) = match dialog {
                    Dialog::Message { error: true, .. } => (Icon::Alert, RED, "Algo deu errado"),
                    Dialog::Message { .. } => (Icon::Check, GREEN, "Pronto"),
                    Dialog::Confirm { .. } => (Icon::Alert, ORANGE, "Confirmar"),
                    Dialog::Input { .. } => (Icon::Pen, ACCENT, "Informe"),
                };
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 15.0, color.gamma_multiply(0.15));
                    paint_icon(ui.painter(), r.shrink(8.0), icon, color);
                    ui.label(RichText::new(title).size(17.0).strong());
                });
                ui.add_space(8.0);
                match dialog {
                    Dialog::Message { text, .. } | Dialog::Confirm { text, .. } => {
                        ui.add(egui::Label::new(text.as_str()).wrap());
                    }
                    Dialog::Input { prompt, value, secret, .. } => {
                        ui.add(egui::Label::new(prompt.as_str()).wrap());
                        let r = ui.add(egui::TextEdit::singleline(value).password(*secret).desired_width(f32::INFINITY).margin(vec2(8.0, 7.0)));
                        r.request_focus();
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            answer = Some(true);
                        }
                    }
                }
                ui.add_space(14.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match dialog {
                    Dialog::Message { .. } => {
                        if button(ui, Kind::Primary, None, "OK", true, false).clicked() {
                            answer = Some(true);
                        }
                    }
                    Dialog::Confirm { .. } => {
                        if button(ui, Kind::Primary, None, "Sim, continuar", true, false).clicked() {
                            answer = Some(true);
                        }
                        if button(ui, Kind::Secondary, None, "Cancelar", true, false).clicked() {
                            answer = Some(false);
                        }
                    }
                    Dialog::Input { .. } => {
                        if button(ui, Kind::Primary, None, "OK", true, false).clicked() {
                            answer = Some(true);
                        }
                        if button(ui, Kind::Secondary, None, "Cancelar", true, false).clicked() {
                            answer = Some(false);
                        }
                    }
                });
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
        egui::Panel::bottom("status").frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(14, 6))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let progress = self.progress.lock().unwrap().clone();
                let text = if self.busy.is_some() && !progress.is_empty() { progress } else { self.busy.clone().unwrap_or(self.status.clone()) };
                if self.busy.is_some() {
                    ui.add(egui::Spinner::new().size(13.0).color(ACCENT));
                } else {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 3.5, GREEN);
                }
                ui.label(RichText::new(text).small().color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("Ver registro").small().color(MUTED)).frame(false)).clicked() {
                        open_path(&log_path());
                    }
                });
            });
        });
        egui::Panel::left("lado")
            .exact_size(250.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(14, 0)).stroke(Stroke::new(1.0, BORDER)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("lado").show(ui, |ui| self.sidebar(ui));
            });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(24, 18))).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt(("pagina", self.tab as u8)).auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(860.0));
                match self.tab {
                    Tab::Diagnose => self.tab_diagnose(ui),
                    Tab::Waste => self.tab_waste(ui),
                    Tab::Maintenance => self.tab_maintenance(ui),
                    Tab::Eeprom => self.tab_eeprom(ui),
                    Tab::Library => self.tab_library(ui),
                }
                ui.add_space(16.0);
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
        let mut hs = Harness::builder().with_size([1060.0, 720.0]).wgpu().build_eframe(|cc| App::new(cc));
        let shot = |hs: &mut Harness<App>, name: &str| {
            hs.run_steps(4);
            hs.render().unwrap().save(out.join(format!("{name}.png"))).unwrap();
        };
        shot(&mut hs, "0-inicio");
        {
            let app = hs.state_mut();
            app.detected = Some(Detected {
                text: "EPSON L3250 Series".into(),
                model: Some("ET-2812".into()),
                rows: vec![
                    ("Conexão".into(), "USB".into()),
                    ("Porta".into(), "USB001".into()),
                    ("Instalada como".into(), "EPSON L3250 Series".into()),
                    ("Série".into(), "X5NY012345".into()),
                    ("Parâmetros".into(), "ET-2812".into()),
                ],
            });
            app.model = "ET-2812".into();
            app.lookup = "1726".into();
            app.lookup_code();
        }
        shot(&mut hs, "1-diagnostico");
        hs.state_mut().tab = Tab::Waste;
        hs.state_mut().levels = vec![("principal".into(), 80.0), ("sem margem".into(), 35.5), ("terceira".into(), 101.2)];
        shot(&mut hs, "2-almofada");
        hs.state_mut().tab = Tab::Maintenance;
        shot(&mut hs, "3-manutencao");
        hs.state_mut().tab = Tab::Eeprom;
        hs.state_mut().ee_out = "   48 (0x0030):   0 (0x00)\n   49 (0x0031):   0 (0x00)".into();
        shot(&mut hs, "4-eeprom");
        hs.state_mut().tab = Tab::Library;
        let log = concat!(env!("CARGO_MANIFEST_DIR"), "/../core/tests/data/wicreset_l3250_trial.log");
        let (mut m, _) = wiclog::learn_file(log, Some("L3250 SERIES"), 80.0, &ecoreset_core::models::builtin()).unwrap();
        m.learned = Some(ecoreset_core::models::Learned { from: "log".into(), at: "2026-11-12T21:00:00".into(), on: "MacBook".into(), pending: true });
        hs.state_mut().learned = vec![m];
        shot(&mut hs, "5-modelos");
        hs.state_mut().confirm("Zerar os contadores da almofada (ET-2812)?\n\nAntes, o EcoReset salva um backup dos valores atuais.", |_| {});
        shot(&mut hs, "6-dialogo");
        hs.state_mut().dialogs.clear();
        hs.state_mut().brand = Brand::Canon;
        hs.state_mut().tab = Tab::Maintenance;
        shot(&mut hs, "7-canon");
    }
}
