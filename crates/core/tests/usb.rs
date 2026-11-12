mod common;

use common::{FakeD4Device, FakePrinter};
use ecoreset_core::d4::{EpsonCtrlChannel, UsbIo};
use ecoreset_core::models::builtin;
use ecoreset_core::printer::EpsonPrinter;
use ecoreset_core::usb::{Interface, UsbCtrlClient, UsbInfo, parse_config, sort_interfaces};
use ecoreset_core::winusb::{parse_device_id, registry_key};

fn l3250() -> FakePrinter {
    FakePrinter::new("EPSON L3250 Series", [74, 54], b"Maribaya", &[(48, 151), (49, 13), (47, 0), (0x100, 0x13)])
}

#[test]
fn full_reset_over_d4() {
    let fake = l3250();
    let dev = FakeD4Device::new(fake.clone(), false);
    let log = dev.log.clone();
    let info = UsbInfo { product: "L3250 Series".into(), ..Default::default() };
    let client = UsbCtrlClient::from_io(Box::new(dev), info).unwrap();
    let mut p = EpsonPrinter::new("USB", Box::new(client));
    assert_eq!(p.detect(&builtin()).unwrap(), ("L3250 Series".into(), Some("ET-2812".into())));
    assert_eq!(p.read_eeprom(49).unwrap(), 13);
    assert_eq!(p.serial_number().unwrap(), "X5NY012345");
    p.reset_waste(None).unwrap();
    assert_eq!((fake.ee(48), fake.ee(255), fake.ee(0x100)), (0, 94, 0x12));
    drop(p); // fecha o canal e sai do D4
    let log = log.lock().unwrap();
    assert_eq!(&log[log.len() - 2..], &[ecoreset_core::d4::CLOSE_CHANNEL, ecoreset_core::d4::EXIT]);
}

#[test]
fn silent_interface_gives_device_back() {
    let dev: Box<dyn UsbIo> = Box::new(FakeD4Device::new(l3250(), true));
    assert!(EpsonCtrlChannel::open(dev).is_err());
}

#[test]
fn interfaces_prefer_printer_class() {
    // dispositivo + configuração com 2 interfaces: 0 (fabricante) e 1 (impressora)
    let config = [
        9, 2, 46, 0, 2, 1, 0, 0xC0, 1,
        9, 4, 0, 0, 2, 0xFF, 0, 0, 0,
        7, 5, 0x03, 2, 0, 2, 0, 7, 5, 0x84, 2, 0, 2, 0,
        9, 4, 1, 0, 2, 7, 1, 2, 0,
        7, 5, 0x01, 2, 0, 2, 0, 7, 5, 0x82, 2, 0, 2, 0,
    ];
    let mut list = parse_config(&config);
    sort_interfaces(&mut list);
    assert_eq!(list[0], Interface { num: 1, alt: 0, class: 7, protocol: 2, ep_out: 0x01, ep_in: 0x82 });
    assert_eq!(list[1].class, 0xFF);
}

#[test]
fn windows_device_id_and_registry_key() {
    let mut raw = vec![0u8, 0];
    raw.extend(b"MANUFACTURER:EPSON;MODEL:L3250 Series;COMMAND SET:ESCPL2,D4;SN:X5NY;");
    raw[1] = raw.len() as u8;
    let id = parse_device_id(&raw);
    assert_eq!((id["MFG"].as_str(), id["MDL"].as_str(), id["SN"].as_str()), ("EPSON", "L3250 Series", "X5NY"));
    let path = r"\\?\usb#vid_04b8&pid_1186#58414142#{28d78fad-5a12-11d1-ae5b-0000f803a8c2}";
    assert_eq!(
        registry_key(path),
        r"SYSTEM\CurrentControlSet\Control\DeviceClasses\{28D78FAD-5A12-11D1-AE5B-0000F803A8C2}\##?#usb#vid_04b8&pid_1186#58414142#{28d78fad-5a12-11d1-ae5b-0000f803a8c2}\#\Device Parameters"
    );
}
