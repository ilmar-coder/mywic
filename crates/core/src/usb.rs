//! Epson pelo cabo USB: o canal EPSON-CTRL do D4 levando os mesmos quadros da rede.
//!
//! No Mac e no Linux usa o libusb (rusb); no Windows, o usbprint.sys que o
//! próprio Windows instala para a impressora (veja winusb.rs), sem trocar driver.

use crate::d4::{EpsonCtrlChannel, UsbIo};
use crate::printer::{EpsonPrinter, Transport};
use crate::{Error, Result};

pub const EPSON_VENDOR: u16 = 0x04B8;
pub const CANON_VENDOR: u16 = 0x04A9;

/// Onde a impressora foi achada.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsbInfo {
    pub product: String,
    /// Número de série do USB (o CUPS usa para achar a fila).
    pub serial: String,
    /// Windows: porta do spooler (USB001...).
    pub port: String,
}

/// Uma interface com par de endpoints bulk.
#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
    pub num: u8,
    pub alt: u8,
    pub class: u8,
    pub protocol: u8,
    pub ep_out: u8,
    pub ep_in: u8,
}

/// Impressora (classe 7) primeiro, depois a do fabricante (0xFF).
pub fn sort_interfaces(list: &mut [Interface]) {
    list.sort_by_key(|f| (f.class != 7, f.class != 0xFF, f.num, f.alt));
}

/// Interfaces com endpoints bulk de um descritor de configuração cru.
pub fn parse_config(config: &[u8]) -> Vec<Interface> {
    let mut found: Vec<(Interface, Option<u8>, Option<u8>)> = Vec::new();
    let mut pos = 0;
    while pos + 2 <= config.len() {
        let (len, kind) = (config[pos] as usize, config[pos + 1]);
        if len < 2 || pos + len > config.len() {
            break;
        }
        let d = &config[pos..pos + len];
        if kind == 4 && len >= 9 {
            let f = Interface { num: d[2], alt: d[3], class: d[5], protocol: d[7], ep_out: 0, ep_in: 0 };
            found.push((f, None, None));
        } else if kind == 5 && len >= 7 && d[3] & 3 == 2
            && let Some(last) = found.last_mut() {
                if d[2] & 0x80 != 0 { last.2 = Some(d[2]) } else { last.1 = Some(d[2]) }
            }
        pos += len;
    }
    found
        .into_iter()
        .filter_map(|(mut f, o, i)| {
            f.ep_out = o?;
            f.ep_in = i?;
            Some(f)
        })
        .collect()
}

/// Interfaces de um aparelho, reservadas uma de cada vez ao avançar.
pub type Claims = Box<dyn Iterator<Item = Box<dyn UsbIo>>>;

/// Cliente que fala EPSON-CTRL pelo USB; o EpsonPrinter o usa como a rede.
pub struct UsbCtrlClient {
    pub channel: EpsonCtrlChannel,
    pub info: UsbInfo,
}

impl Transport for UsbCtrlClient {
    fn command(&mut self, frame: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(Some(self.channel.command(frame)?))
    }

    fn model_string(&mut self) -> Result<String> {
        Ok(self.info.product.clone())
    }
}

impl UsbCtrlClient {
    /// A primeira Epson do USB que responde ao D4 (com `serial`, só essa).
    pub fn open(serial: Option<&str>) -> Result<UsbCtrlClient> {
        for (info, interfaces) in platform::epson_candidates(serial)? {
            for io in interfaces {
                match EpsonCtrlChannel::open(io) {
                    Ok(channel) => return Ok(UsbCtrlClient { channel, info }),
                    Err(_) => continue,
                }
            }
        }
        Err(Error::new(platform::NOT_FOUND))
    }

    /// Para injetar um aparelho já aberto (testes, outras camadas).
    pub fn from_io(io: Box<dyn UsbIo>, info: UsbInfo) -> Result<UsbCtrlClient> {
        let channel = EpsonCtrlChannel::open(io).map_err(|(_, e)| e)?;
        Ok(UsbCtrlClient { channel, info })
    }
}

/// Epson pronta para uso pelo cabo, já como EpsonPrinter, e onde foi achada.
pub fn open_printer() -> Result<(EpsonPrinter, UsbInfo)> {
    let client = UsbCtrlClient::open(None)?;
    let info = client.info.clone();
    Ok((EpsonPrinter::new("USB", Box::new(client)), info))
}

/// [(produto, série)] das Epson ligadas no USB.
pub fn list_usb_printers() -> Vec<(String, String)> {
    platform::list(EPSON_VENDOR)
}

/// Interfaces bidirecionais de impressora (classe 7, protocolo 2) do fabricante,
/// já abertas: usado pela Canon (IVEC).
pub fn open_printer_interfaces(vendor: u16) -> Result<Vec<(UsbInfo, Box<dyn UsbIo>)>> {
    platform::printer_interfaces(vendor)
}

#[cfg(not(windows))]
mod platform {
    use std::sync::Arc;
    use std::time::Duration;

    use rusb::{Context, Device, DeviceHandle, Direction, TransferType, UsbContext};

    use super::{Interface, UsbInfo, sort_interfaces};
    use crate::d4::UsbIo;
    use crate::{Error, Result};

    pub const NOT_FOUND: &str =
        "nenhuma Epson respondeu pelo USB (está ligada e com o cabo conectado?)";

    fn usb_err(what: &str, e: rusb::Error) -> Error {
        match e {
            rusb::Error::Access => Error::new(format!(
                "{what}: acesso negado ao USB (outro programa está usando a impressora? \
                 no Linux, instale o pacote do EcoReset para a regra do USB)"
            )),
            rusb::Error::NotFound if what.contains("libusb") => {
                Error::new("libusb não encontrado (Fedora: sudo dnf install libusb1)")
            }
            other => Error::new(format!("{what}: {other}")),
        }
    }

    pub struct LibusbIo {
        handle: Arc<DeviceHandle<Context>>,
        iface: u8,
        ep_out: u8,
        ep_in: u8,
    }

    impl UsbIo for LibusbIo {
        fn write(&mut self, data: &[u8]) -> Result<()> {
            let mut done = 0;
            while done < data.len() {
                let n = self
                    .handle
                    .write_bulk(self.ep_out, &data[done..], Duration::from_secs(5))
                    .map_err(|e| usb_err("enviar pelo USB", e))?;
                if n == 0 {
                    return Err(Error::new("a impressora não aceitou os dados pelo USB"));
                }
                done += n;
            }
            Ok(())
        }

        fn read(&mut self, timeout_ms: u32) -> Result<Vec<u8>> {
            let mut buf = vec![0u8; 16384];
            match self.handle.read_bulk(self.ep_in, &mut buf, Duration::from_millis(timeout_ms.max(1) as u64)) {
                Ok(n) => {
                    buf.truncate(n);
                    Ok(buf)
                }
                Err(rusb::Error::Timeout) => Ok(Vec::new()),
                Err(e) => Err(usb_err("ler pelo USB", e)),
            }
        }
    }

    impl Drop for LibusbIo {
        fn drop(&mut self) {
            let _ = self.handle.release_interface(self.iface);
        }
    }

    fn interfaces(device: &Device<Context>) -> Vec<Interface> {
        let Ok(config) = device.active_config_descriptor() else { return Vec::new() };
        let mut list = Vec::new();
        for iface in config.interfaces() {
            for d in iface.descriptors() {
                let (mut out, mut inp) = (None, None);
                for ep in d.endpoint_descriptors() {
                    if ep.transfer_type() != TransferType::Bulk {
                        continue;
                    }
                    match ep.direction() {
                        Direction::Out => out = out.or(Some(ep.address())),
                        Direction::In => inp = inp.or(Some(ep.address())),
                    }
                }
                if let (Some(ep_out), Some(ep_in)) = (out, inp) {
                    list.push(Interface {
                        num: d.interface_number(),
                        alt: d.setting_number(),
                        class: d.class_code(),
                        protocol: d.protocol_code(),
                        ep_out,
                        ep_in,
                    });
                }
            }
        }
        sort_interfaces(&mut list);
        list
    }

    fn claim(handle: &Arc<DeviceHandle<Context>>, f: &Interface) -> Result<LibusbIo> {
        handle.claim_interface(f.num).map_err(|e| usb_err(&format!("abrir a interface {}", f.num), e))?;
        if f.alt != 0
            && let Err(e) = handle.set_alternate_setting(f.num, f.alt) {
                let _ = handle.release_interface(f.num);
                return Err(usb_err(&format!("escolher alt {}", f.alt), e));
            }
        Ok(LibusbIo { handle: handle.clone(), iface: f.num, ep_out: f.ep_out, ep_in: f.ep_in })
    }

    /// Aparelhos do fabricante, abertos, com produto e série.
    fn open_devices(vendor: u16) -> Result<Vec<(Device<Context>, Arc<DeviceHandle<Context>>, UsbInfo)>> {
        let ctx = Context::new().map_err(|e| usb_err("iniciar o libusb", e))?;
        let mut out = Vec::new();
        for device in ctx.devices().map_err(|e| usb_err("listar USB", e))?.iter() {
            let Ok(desc) = device.device_descriptor() else { continue };
            if desc.vendor_id() != vendor {
                continue;
            }
            let Ok(handle) = device.open() else { continue };
            let _ = handle.set_auto_detach_kernel_driver(true);
            let info = UsbInfo {
                product: handle.read_product_string_ascii(&desc).unwrap_or_default(),
                serial: handle.read_serial_number_string_ascii(&desc).unwrap_or_default(),
                port: String::new(),
            };
            out.push((device, Arc::new(handle), info));
        }
        Ok(out)
    }

    /// Cada Epson com as interfaces candidatas já reservadas, uma por vez.
    pub fn epson_candidates(serial: Option<&str>) -> Result<Vec<(UsbInfo, super::Claims)>> {
        let mut out = Vec::new();
        for (device, handle, info) in open_devices(super::EPSON_VENDOR)? {
            if serial.is_some_and(|s| s != info.serial) {
                continue;
            }
            let claims: super::Claims = Box::new(LazyClaims { handle, list: interfaces(&device) });
            out.push((info, claims));
        }
        Ok(out)
    }

    /// Reserva as interfaces na hora (só uma presa por vez).
    struct LazyClaims {
        handle: Arc<DeviceHandle<Context>>,
        list: Vec<Interface>,
    }

    impl Iterator for LazyClaims {
        type Item = Box<dyn UsbIo>;
        fn next(&mut self) -> Option<Self::Item> {
            while !self.list.is_empty() {
                let f = self.list.remove(0);
                if let Ok(io) = claim(&self.handle, &f) {
                    return Some(Box::new(io));
                }
            }
            None
        }
    }

    pub fn printer_interfaces(vendor: u16) -> Result<Vec<(UsbInfo, Box<dyn UsbIo>)>> {
        let mut out = Vec::new();
        for (device, handle, info) in open_devices(vendor)? {
            for f in interfaces(&device).iter().filter(|f| f.class == 7 && f.protocol == 2) {
                if let Ok(io) = claim(&handle, f) {
                    out.push((info.clone(), Box::new(io) as Box<dyn UsbIo>));
                    break;
                }
            }
        }
        Ok(out)
    }

    pub fn list(vendor: u16) -> Vec<(String, String)> {
        open_devices(vendor)
            .map(|v| v.into_iter().map(|(_, _, i)| (i.product, i.serial)).collect())
            .unwrap_or_default()
    }
}

#[cfg(windows)]
mod platform {
    use super::UsbInfo;
    use crate::d4::UsbIo;
    use crate::winusb::{UsbPrintDevice, device_paths, port_name};
    use crate::Result;

    pub const NOT_FOUND: &str = "nenhuma Epson respondeu pelo USB. Confira se está ligada e \
                                 feche o monitor de status da Epson, que pode ocupar a porta.";

    fn opened(vendor_name: &str) -> Vec<(UsbInfo, std::collections::BTreeMap<String, String>, UsbPrintDevice)> {
        let mut out = Vec::new();
        for path in device_paths() {
            let Ok(mut device) = UsbPrintDevice::open(&path) else { continue };
            let id = device.device_id().unwrap_or_default();
            let mfg = id.get("MFG").cloned().unwrap_or_default().to_uppercase();
            if !id.is_empty() && mfg != vendor_name {
                continue;
            }
            let info = UsbInfo {
                product: id.get("MDL").cloned().unwrap_or_else(|| vendor_name.into()),
                serial: id.get("SN").cloned().unwrap_or_default(),
                port: port_name(&path),
            };
            out.push((info, id, device));
        }
        out
    }

    pub fn epson_candidates(serial: Option<&str>) -> Result<Vec<(UsbInfo, super::Claims)>> {
        Ok(opened("EPSON")
            .into_iter()
            .filter(|(info, _, _)| serial.is_none_or(|s| s == info.serial))
            .map(|(info, _, d)| {
                let claims: super::Claims = Box::new(std::iter::once(Box::new(d) as Box<dyn UsbIo>));
                (info, claims)
            })
            .collect())
    }

    pub fn printer_interfaces(vendor: u16) -> Result<Vec<(UsbInfo, Box<dyn UsbIo>)>> {
        let name = if vendor == super::CANON_VENDOR { "CANON" } else { "EPSON" };
        Ok(opened(name)
            .into_iter()
            .filter(|(_, id, _)| vendor != super::CANON_VENDOR || id.get("CMD").is_some_and(|c| c.contains("IVEC")))
            .map(|(info, _, d)| (info, Box::new(d) as Box<dyn UsbIo>))
            .collect())
    }

    pub fn list(vendor: u16) -> Vec<(String, String)> {
        let name = if vendor == super::CANON_VENDOR { "CANON" } else { "EPSON" };
        opened(name).into_iter().map(|(i, _, _)| (i.product, i.serial)).collect()
    }
}
