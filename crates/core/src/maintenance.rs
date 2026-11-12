//! Teste de bicos e limpeza de cabeça (comandos ESC/P2 em "remote mode").
//!
//! Os bytes vão pela rede (porta RAW 9100) ou para uma impressora instalada,
//! como trabalho RAW: pelo spooler no Windows e pelo CUPS (lp -o raw) no Mac e
//! no Linux.

use std::io::Write;
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

use regex::Regex;

use crate::{Error, Result, bail};

pub const EXIT_PACKET_MODE: &[u8] = b"\x00\x00\x00\x1b\x01@EJL 1284.4\n@EJL     \n";
pub const RESET: &[u8] = b"\x1b@";
pub const ENTER_REMOTE: &[u8] = b"\x1b(R\x08\x00\x00REMOTE1";
pub const EXIT_REMOTE: &[u8] = b"\x1b\x00\x00\x00\x1b\x00";
/// Fim do trabalho: avança a folha e encerra. Suspeita: sem isso (e com um ESC @
/// logo depois do modo remoto, como era antes) a impressora pode ficar
/// esperando o resto da página, com o papel e o carro indo e voltando.
pub const END_JOB: &[u8] = b"\x0c\x1b\x00\x1b\x00";

/// Um trabalho completo com um comando remoto: 2 letras + tamanho + args.
/// Mesma moldura que o escputil (gutenprint) usa.
pub fn remote_job(cmd: &[u8; 2], args: &[u8]) -> Vec<u8> {
    let mut job = Vec::new();
    for part in [EXIT_PACKET_MODE, RESET, RESET, ENTER_REMOTE, cmd] {
        job.extend_from_slice(part);
    }
    job.extend((args.len() as u16).to_le_bytes());
    job.extend_from_slice(args);
    job.extend_from_slice(EXIT_REMOTE);
    job.extend_from_slice(END_JOB);
    job
}

pub fn nozzle_check_job() -> Vec<u8> {
    remote_job(b"NC", &[0, 0])
}

pub fn head_cleaning_job() -> Vec<u8> {
    remote_job(b"CH", &[0, 0])
}

pub fn send_raw_network(host: &str, data: &[u8]) -> Result<()> {
    let addr = format!("{host}:9100");
    let target = std::net::ToSocketAddrs::to_socket_addrs(&addr)?
        .next()
        .ok_or_else(|| Error::new(format!("endereço inválido: {host}")))?;
    let mut sock = TcpStream::connect_timeout(&target, Duration::from_secs(10))?;
    sock.write_all(data)?;
    Ok(())
}

/// Impressoras instaladas no sistema (vazio se não houver spooler).
pub fn installed_printers() -> Vec<String> {
    if cfg!(windows) { windows_printer_ports().into_iter().map(|(n, _)| n).collect() } else { cups_printers() }
}

pub fn send_raw_installed(printer: &str, data: &[u8]) -> Result<()> {
    #[cfg(windows)]
    {
        send_raw_windows(printer, data)
    }
    #[cfg(not(windows))]
    {
        send_raw_cups(printer, data)
    }
}

/// Filas instaladas que imprimem na impressora USB detectada.
/// Windows: as que usam a porta (USB001...). Mac/Linux: as do CUPS com a mesma
/// série no endereço usb://; sem série, as do mesmo modelo.
pub fn queues_for_usb(port: &str, serial: &str, model: &str) -> Vec<String> {
    if cfg!(windows) {
        let port = port.to_uppercase();
        return windows_printer_ports()
            .into_iter()
            .filter(|(_, ports)| !port.is_empty() && ports.contains(&port))
            .map(|(n, _)| n)
            .collect();
    }
    match_cups_queues(&cups_devices(), serial, model)
}

/// devices: [(fila, uri)]. A série é a do USB; o CUPS às vezes a grava em hexadecimal.
pub fn match_cups_queues(devices: &[(String, String)], serial: &str, model: &str) -> Vec<String> {
    let mut wanted: Vec<String> = Vec::new();
    if !serial.is_empty() {
        wanted.push(serial.to_uppercase());
        wanted.push(serial.bytes().map(|b| format!("{b:02X}")).collect());
        if let Some(text) = from_hex(serial) {
            wanted.push(text.to_uppercase());
        }
    }
    let (mut by_serial, mut by_model) = (Vec::new(), Vec::new());
    for (name, uri) in devices {
        if !uri.starts_with("usb://") {
            continue;
        }
        let query = uri.split_once('?').map(|(_, q)| q).unwrap_or_default();
        let serials: Vec<String> = query
            .split('&')
            .filter_map(|kv| kv.strip_prefix("serial="))
            .map(|v| unquote(v).to_uppercase())
            .collect();
        if serials.iter().any(|s| wanted.contains(s)) {
            by_serial.push(name.clone());
        } else if !model.is_empty() && unquote(uri).to_uppercase().contains(&model.to_uppercase()) {
            by_model.push(name.clone());
        }
    }
    if by_serial.is_empty() { by_model } else { by_serial }
}

fn from_hex(text: &str) -> Option<String> {
    if !text.len().is_multiple_of(2) || text.is_empty() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect();
    let bytes = bytes?;
    bytes.iter().all(|b| b.is_ascii_graphic()).then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// Desfaz %20 e afins.
pub fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---- CUPS (Mac e Linux) ----------------------------------------------------

pub fn cups_printers() -> Vec<String> {
    let Ok(out) = Command::new("lpstat").arg("-e").output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

/// [(fila, endereço do dispositivo)] do CUPS (lpstat -v).
pub fn cups_devices() -> Vec<(String, String)> {
    let Ok(out) = Command::new("lpstat").arg("-v").output() else { return Vec::new() };
    parse_lpstat_v(&String::from_utf8_lossy(&out.stdout))
}

/// "device for FILA: uri" (o Mac traduz o começo, então só a fila e a uri).
pub fn parse_lpstat_v(text: &str) -> Vec<(String, String)> {
    let re = Regex::new(r"(\S+): ([a-z][a-z0-9+.-]*:\S*)$").unwrap();
    text.lines()
        .filter_map(|l| re.captures(l.trim()).map(|c| (c[1].to_string(), c[2].to_string())))
        .collect()
}

pub fn send_raw_cups(printer: &str, data: &[u8]) -> Result<()> {
    let mut child = Command::new("lp")
        .args(["-d", printer, "-o", "raw", "-t", "EcoReset"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::new(format!("não consegui chamar o lp do CUPS: {e}")))?;
    child.stdin.take().unwrap().write_all(data)?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("o CUPS recusou o trabalho: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

// ---- spooler do Windows ----------------------------------------------------

/// [(impressora, [portas])] das instaladas no Windows (vazio em outros sistemas).
pub fn windows_printer_ports() -> Vec<(String, Vec<String>)> {
    #[cfg(windows)]
    {
        spooler::printer_ports()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
pub fn send_raw_windows(printer: &str, data: &[u8]) -> Result<()> {
    spooler::send_raw(printer, data)
}

#[cfg(windows)]
mod spooler {
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Graphics::Printing::{
        ClosePrinter, DOC_INFO_1W, EndDocPrinter, EndPagePrinter, EnumPrintersW, OpenPrinterW,
        PRINTER_ENUM_LOCAL, PRINTER_HANDLE, PRINTER_INFO_2W, StartDocPrinterW, StartPagePrinter,
        WritePrinter,
    };

    use crate::{Error, Result};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn from_wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut len = 0;
        while unsafe { *p.add(len) } != 0 {
            len += 1;
        }
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
    }

    pub fn printer_ports() -> Vec<(String, Vec<String>)> {
        unsafe {
            let (mut needed, mut count) = (0u32, 0u32);
            EnumPrintersW(PRINTER_ENUM_LOCAL, null(), 2, null_mut(), 0, &mut needed, &mut count);
            if needed == 0 {
                return Vec::new();
            }
            let mut buf = vec![0u8; needed as usize];
            if EnumPrintersW(PRINTER_ENUM_LOCAL, null(), 2, buf.as_mut_ptr(), needed, &mut needed, &mut count) == 0 {
                return Vec::new();
            }
            let infos = buf.as_ptr() as *const PRINTER_INFO_2W;
            (0..count as usize)
                .map(|i| {
                    let info = &*infos.add(i);
                    let ports = from_wide(info.pPortName)
                        .split(',')
                        .map(|p| p.trim().to_uppercase())
                        .filter(|p| !p.is_empty())
                        .collect();
                    (from_wide(info.pPrinterName), ports)
                })
                .collect()
        }
    }

    pub fn send_raw(printer: &str, data: &[u8]) -> Result<()> {
        unsafe {
            let mut handle: PRINTER_HANDLE = std::mem::zeroed();
            if OpenPrinterW(wide(printer).as_ptr(), &mut handle, null()) == 0 {
                return Err(Error::new(format!("não consegui abrir a impressora {printer}")));
            }
            let name = wide("EcoReset");
            let kind = wide("RAW");
            let doc = DOC_INFO_1W { pDocName: name.as_ptr() as *mut u16, pOutputFile: null_mut(), pDatatype: kind.as_ptr() as *mut u16 };
            let mut result = Ok(());
            if StartDocPrinterW(handle, 1, &doc) == 0 {
                result = Err(Error::new("o spooler recusou o trabalho"));
            } else {
                StartPagePrinter(handle);
                let mut written = 0u32;
                let ok = WritePrinter(handle, data.as_ptr().cast(), data.len() as u32, &mut written);
                EndPagePrinter(handle);
                EndDocPrinter(handle);
                if ok == 0 || written as usize != data.len() {
                    result = Err(Error::new("o spooler não aceitou os dados"));
                }
            }
            ClosePrinter(handle);
            result
        }
    }
}
