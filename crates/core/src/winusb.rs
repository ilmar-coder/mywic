//! USB no Windows pelo driver da própria impressora (usbprint.sys), sem libusb.
//!
//! O Windows instala o usbprint.sys na interface de impressora de qualquer
//! impressora USB e publica GUID_DEVINTERFACE_USBPRINT: ReadFile e WriteFile vão
//! direto para os endpoints bulk. Assim o D4 funciona sem trocar o driver.

use std::collections::BTreeMap;

pub const GUID_DEVINTERFACE_USBPRINT: &str = "{28D78FAD-5A12-11D1-AE5B-0000F803A8C2}";
pub const IOCTL_USBPRINT_GET_1284_ID: u32 = 0x220034;

/// IOCTL_USBPRINT_GET_1284_ID: 2 bytes de tamanho (big-endian) + texto.
pub fn parse_device_id(raw: &[u8]) -> BTreeMap<String, String> {
    let mut info = BTreeMap::new();
    if raw.len() < 2 {
        return info;
    }
    let length = u16::from_be_bytes([raw[0], raw[1]]) as usize;
    let end = length.min(raw.len()).max(2);
    let text: String = raw[2..end].iter().map(|b| *b as char).collect();
    for part in text.split(';') {
        if let Some((k, v)) = part.split_once(':') {
            info.insert(k.trim().to_uppercase(), v.trim().to_string());
        }
    }
    // nomes longos do IEEE 1284 equivalem aos curtos
    for (long, short) in [("MANUFACTURER", "MFG"), ("MODEL", "MDL"), ("COMMAND SET", "CMD")] {
        if let Some(v) = info.get(long).cloned() {
            info.entry(short.to_string()).or_insert(v);
        }
    }
    info
}

/// Chave do registro da interface: \\?\usb#... -> ##?#usb#...
pub fn registry_key(path: &str) -> String {
    format!(
        "SYSTEM\\CurrentControlSet\\Control\\DeviceClasses\\{}\\##?#{}\\#\\Device Parameters",
        GUID_DEVINTERFACE_USBPRINT,
        path.get(4..).unwrap_or_default()
    )
}

#[cfg(windows)]
pub use imp::{UsbPrintDevice, device_paths, port_name};

#[cfg(windows)]
mod imp {
    use std::collections::BTreeMap;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SetupDiDestroyDeviceInfoList,
        SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
    };
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
        INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
        WriteFile,
    };
    use windows_sys::Win32::System::IO::{CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows_sys::core::GUID;

    use super::{IOCTL_USBPRINT_GET_1284_ID, parse_device_id, registry_key};
    use crate::d4::UsbIo;
    use crate::{Error, Result};

    const USBPRINT: GUID = GUID::from_u128(0x28d78fad_5a12_11d1_ae5b_0000f803a8c2);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Caminhos de todas as impressoras USB vistas pelo usbprint.sys.
    pub fn device_paths() -> Vec<String> {
        let mut paths = Vec::new();
        unsafe {
            let set = SetupDiGetClassDevsW(&USBPRINT, null(), null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
            if set == 0 || set == -1 {
                return paths;
            }
            let mut index = 0;
            loop {
                let mut data: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
                data.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
                if SetupDiEnumDeviceInterfaces(set, null(), &USBPRINT, index, &mut data) == 0 {
                    break;
                }
                index += 1;
                let mut required = 0u32;
                SetupDiGetDeviceInterfaceDetailW(set, &data, null_mut(), 0, &mut required, null_mut());
                if required == 0 {
                    continue;
                }
                let mut buf = vec![0u8; required as usize + 4];
                // cbSize da estrutura de detalhe: 8 em 64 bits, 6 em 32 bits
                let cb: u32 = if std::mem::size_of::<usize>() == 8 { 8 } else { 6 };
                buf[..4].copy_from_slice(&cb.to_le_bytes());
                if SetupDiGetDeviceInterfaceDetailW(set, &data, buf.as_mut_ptr().cast(), required, null_mut(), null_mut()) != 0 {
                    let chars: Vec<u16> = buf[4..required as usize]
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .take_while(|c| *c != 0)
                        .collect();
                    paths.push(String::from_utf16_lossy(&chars));
                }
            }
            SetupDiDestroyDeviceInfoList(set);
        }
        paths
    }

    /// Porta do spooler desta impressora USB ("USB001"), ou "" se não achar.
    /// O usbprint.sys grava "Base Name" e "Port Number" nos parâmetros da interface.
    pub fn port_name(path: &str) -> String {
        if !path.starts_with("\\\\?\\") {
            return String::new();
        }
        unsafe {
            let mut key: HKEY = null_mut();
            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, wide(&registry_key(path)).as_ptr(), 0, KEY_READ, &mut key) != 0 {
                return String::new();
            }
            let mut base = [0u16; 64];
            let mut size = (base.len() * 2) as u32;
            let ok_base = RegQueryValueExW(key, wide("Base Name").as_ptr(), null(), null_mut(),
                base.as_mut_ptr().cast(), &mut size) == 0;
            let mut number = 0u32;
            let mut nsize = 4u32;
            let ok_num = RegQueryValueExW(key, wide("Port Number").as_ptr(), null(), null_mut(),
                (&mut number as *mut u32).cast(), &mut nsize) == 0;
            RegCloseKey(key);
            if !(ok_base && ok_num) {
                return String::new();
            }
            let base: String = String::from_utf16_lossy(&base[..base.iter().position(|c| *c == 0).unwrap_or(0)]);
            format!("{base}{number:03}")
        }
    }

    /// Uma impressora USB aberta pelo usbprint.sys (write/read para o D4).
    pub struct UsbPrintDevice {
        handle: HANDLE,
    }

    unsafe impl Send for UsbPrintDevice {}

    impl UsbPrintDevice {
        pub fn open(path: &str) -> Result<UsbPrintDevice> {
            let handle = unsafe {
                CreateFileW(wide(path).as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                    null(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, null_mut())
            };
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                return Err(Error::new("não consegui abrir a impressora USB (está imprimindo ou o \
                                       monitor de status da Epson está usando?)"));
            }
            Ok(UsbPrintDevice { handle })
        }

        /// Roda uma operação assíncrona com prazo; devolve os bytes transferidos.
        fn io(&self, timeout_ms: u32, start: impl FnOnce(*mut OVERLAPPED) -> i32) -> Result<u32> {
            unsafe {
                let mut ov: OVERLAPPED = std::mem::zeroed();
                ov.hEvent = CreateEventW(null(), 1, 0, null());
                let result = (|| {
                    if start(&mut ov) == 0 {
                        let err = GetLastError();
                        if err != ERROR_IO_PENDING {
                            return Err(Error::new(format!("erro {err} no USB")));
                        }
                    }
                    let mut done = 0u32;
                    if WaitForSingleObject(ov.hEvent, timeout_ms.max(1)) == WAIT_TIMEOUT {
                        CancelIoEx(self.handle, &ov);
                        GetOverlappedResult(self.handle, &ov, &mut done, 1);
                        return Ok(done);
                    }
                    GetOverlappedResult(self.handle, &ov, &mut done, 0);
                    Ok(done)
                })();
                CloseHandle(ov.hEvent);
                result
            }
        }

        pub fn device_id(&mut self) -> Result<BTreeMap<String, String>> {
            let mut buf = vec![0u8; 1024];
            let h = self.handle;
            let ptr = buf.as_mut_ptr();
            let n = self.io(2000, |ov| unsafe {
                DeviceIoControl(h, IOCTL_USBPRINT_GET_1284_ID, null(), 0, ptr.cast(), 1024, null_mut(), ov)
            })?;
            Ok(parse_device_id(&buf[..n as usize]))
        }
    }

    impl UsbIo for UsbPrintDevice {
        fn write(&mut self, data: &[u8]) -> Result<()> {
            let h = self.handle;
            let n = self.io(5000, |ov| unsafe { WriteFile(h, data.as_ptr(), data.len() as u32, null_mut(), ov) })?;
            if n as usize != data.len() {
                return Err(Error::new("a impressora não aceitou os dados pelo USB"));
            }
            Ok(())
        }

        fn read(&mut self, timeout_ms: u32) -> Result<Vec<u8>> {
            let mut buf = vec![0u8; 16384];
            let h = self.handle;
            let ptr = buf.as_mut_ptr();
            let n = self.io(timeout_ms, |ov| unsafe { ReadFile(h, ptr, 16384, null_mut(), ov) })?;
            buf.truncate(n as usize);
            Ok(buf)
        }
    }

    impl Drop for UsbPrintDevice {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}
