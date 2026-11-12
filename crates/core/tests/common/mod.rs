//! Impressoras simuladas para os testes: Epson pela rede (agente SNMP em UDP) e
//! pelo USB (D4), e a Canon IVEC.
#![allow(dead_code)]

use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};

use ecoreset_core::d4::{self, UsbIo};
use ecoreset_core::printer::{EPCTRL_OID, EpsonPrinter, MODEL_OID, shift_key};
use ecoreset_core::snmp::{self, SnmpClient};
use ecoreset_core::{Error, Result};
use sha1::{Digest, Sha1};

#[derive(Clone)]
pub struct State {
    pub name: String,
    pub read_key: [u8; 2],
    pub write_key: Vec<u8>,
    pub eeprom: HashMap<u16, u8>,
    /// false, "write" (só gravação bloqueada) ou "all"
    pub locked: &'static str,
    pub serial: String,
    /// firmware que não confere a chave de leitura
    pub any_key: bool,
    pub temp_resets: u32,
    /// campo do status com o número de série
    pub serial_field: u8,
}

#[derive(Clone)]
pub struct FakePrinter(pub Arc<Mutex<State>>);

impl FakePrinter {
    pub fn new(name: &str, read_key: [u8; 2], write_key: &[u8], eeprom: &[(u16, u8)]) -> Self {
        FakePrinter(Arc::new(Mutex::new(State {
            name: name.into(),
            read_key,
            write_key: shift_key(write_key),
            eeprom: eeprom.iter().copied().collect(),
            locked: "",
            serial: "X5NY012345".into(),
            any_key: false,
            temp_resets: 0,
            serial_field: 0x1F,
        })))
    }

    /// L3150: contador principal 0x0C10 (~48%) e sem margem 0x0120 (~8%).
    pub fn l3150() -> Self {
        FakePrinter::new("EPSON L3150 Series", [151, 7], b"Maribaya", &[(48, 0x10), (49, 0x0C), (47, 0), (50, 0x20), (51, 0x01)])
    }

    pub fn ee(&self, a: u16) -> u8 {
        *self.0.lock().unwrap().eeprom.get(&a).unwrap_or(&0)
    }

    pub fn set(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.0.lock().unwrap());
    }

    /// Responde a um quadro EPSON-CTRL como a impressora: (tag SNMP, valor).
    pub fn handle(&self, frame: &[u8]) -> (u8, Vec<u8>) {
        let mut st = self.0.lock().unwrap();
        let (cmd, payload) = (&frame[..2], &frame[4..]);
        let ok = |v: Vec<u8>| (snmp::OCTET_STRING, v);
        match cmd {
            b"di" => ok(format!("@EJL ID\r\nMFG:EPSON;MDL:{};\x0c", st.name).into_bytes()),
            b"st" => {
                let serial = st.serial.clone().into_bytes();
                let mut items = vec![0x01, 0x01, 0x04, st.serial_field, serial.len() as u8];
                items.extend(serial);
                let mut v = b"\x00@BDC ST2\r\n".to_vec();
                v.extend((items.len() as u16).to_le_bytes());
                v.extend(items);
                ok(v)
            }
            b"rw" => {
                let good = payload[2..] == Sha1::digest(st.serial.as_bytes())[..];
                st.temp_resets += good as u32;
                ok(if good { b"rw:01:OK;\x0c".to_vec() } else { b"rw:01:NA;\x0c".to_vec() })
            }
            b"||" => {
                if (payload[..2] != st.read_key && !st.any_key) || st.locked == "all" {
                    return ok(b"@BDC PS\r\nee:NA;\x0c".to_vec());
                }
                let address = u16::from_le_bytes([payload[5], payload[6]]);
                match payload[2] {
                    0x41 => {
                        let v = *st.eeprom.get(&address).unwrap_or(&0);
                        ok(format!("@BDC PS\r\nEE:{address:04X}{v:02X};\x0c").into_bytes())
                    }
                    0x42 => {
                        if !st.locked.is_empty() || payload[8..16] != st.write_key[..] {
                            return ok(b"||:42:NA;\x0c".to_vec());
                        }
                        st.eeprom.insert(address, payload[7]);
                        ok(b"||:42:OK;\x0c".to_vec())
                    }
                    _ => ok(b"||:NA;\x0c".to_vec()),
                }
            }
            other => {
                let mut v = other.to_vec();
                v.extend(b":NA;\x0c");
                ok(v)
            }
        }
    }

    /// Sobe um agente SNMP em UDP e devolve a impressora pela rede.
    pub fn serve_snmp(&self) -> EpsonPrinter {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = sock.local_addr().unwrap().port();
        let fake = self.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 65535];
            while let Ok((n, addr)) = sock.recv_from(&mut buf) {
                let (rid, oid) = parse_request(&buf[..n]);
                let (tag, value) = if oid == MODEL_OID {
                    (snmp::OCTET_STRING, fake.0.lock().unwrap().name.clone().into_bytes())
                } else if let Some(rest) = oid.strip_prefix(&format!("{EPCTRL_OID}.")) {
                    let frame: Vec<u8> = rest.split('.').map(|x| x.parse().unwrap()).collect();
                    fake.handle(&frame)
                } else {
                    (0x80, Vec::new())
                };
                let _ = sock.send_to(&response(rid, &oid, tag, &value), addr);
            }
        });
        let mut client = SnmpClient::new("127.0.0.1");
        client.port = port;
        client.timeout = std::time::Duration::from_millis(500);
        client.retries = 0;
        EpsonPrinter::new("127.0.0.1", Box::new(ecoreset_core::printer::SnmpTransport(client)))
    }
}

fn parse_request(packet: &[u8]) -> (i64, String) {
    let (_, message, _) = snmp::decode_tlv(packet, 0).unwrap();
    let parts = snmp::decode_children(message).unwrap();
    let fields = snmp::decode_children(parts[2].1).unwrap();
    let rid = fields[0].1.iter().fold(0i64, |a, b| (a << 8) | *b as i64);
    let varbinds = snmp::decode_children(fields[3].1).unwrap();
    let vb = snmp::decode_children(varbinds[0].1).unwrap();
    (rid, snmp::decode_oid(vb[0].1))
}

fn response(rid: i64, oid: &str, tag: u8, value: &[u8]) -> Vec<u8> {
    let mut vb = snmp::encode_oid(oid).unwrap();
    vb.extend(snmp::tlv(tag, value));
    let mut pdu = snmp::encode_int(rid);
    pdu.extend(snmp::encode_int(0));
    pdu.extend(snmp::encode_int(0));
    pdu.extend(snmp::tlv(snmp::SEQUENCE, &snmp::tlv(snmp::SEQUENCE, &vb)));
    let mut msg = snmp::encode_int(0);
    msg.extend(snmp::tlv(snmp::OCTET_STRING, b"public"));
    msg.extend(snmp::tlv(snmp::GET_RESPONSE, &pdu));
    snmp::tlv(snmp::SEQUENCE, &msg)
}

pub const CTRL_SOCKET: u8 = 0x40;

/// Aparelho USB simulado que fala D4 e responde EPSON-CTRL como a FakePrinter.
pub struct FakeD4Device {
    pub printer: FakePrinter,
    pub silent: bool,
    out: Vec<u8>,
    in_d4: bool,
    channel_open: bool,
    credit_to_host: u32,
    pub log: Arc<Mutex<Vec<u8>>>,
}

impl FakeD4Device {
    pub fn new(printer: FakePrinter, silent: bool) -> Self {
        FakeD4Device { printer, silent, out: Vec::new(), in_d4: false, channel_open: false, credit_to_host: 0, log: Default::default() }
    }

    fn reply(&mut self, command: u8, result: u8, rest: &[u8]) {
        let mut body = vec![command | 0x80, result];
        body.extend_from_slice(rest);
        self.out.extend(d4::packet(0, 0, &body, 1, 0));
    }

    fn transaction(&mut self, payload: &[u8]) {
        let (cmd, args) = (payload[0], &payload[1..]);
        self.log.lock().unwrap().push(cmd);
        match cmd {
            d4::INIT => self.reply(cmd, 0, &[0x10]),
            d4::GET_SOCKET_ID => {
                let ok = args == b"EPSON-CTRL";
                let mut rest = vec![CTRL_SOCKET];
                rest.extend_from_slice(args);
                self.reply(cmd, if ok { 0 } else { 0x0A }, &rest);
            }
            d4::OPEN_CHANNEL => {
                self.channel_open = true;
                let mut rest = args[..6].to_vec();
                rest.extend([0, 0, 0, 0]);
                self.reply(cmd, 0, &rest);
            }
            d4::CREDIT_REQUEST => {
                self.credit_to_host += 1;
                let mut rest = args[..2].to_vec();
                rest.extend([0, 1]);
                self.reply(cmd, 0, &rest);
            }
            d4::CREDIT => self.reply(cmd, 0, &args[..2].to_vec()),
            d4::CLOSE_CHANNEL => {
                self.channel_open = false;
                self.reply(cmd, 0, &args[..2].to_vec());
            }
            d4::EXIT => {
                self.in_d4 = false;
                self.reply(cmd, 0, &[]);
            }
            _ => self.reply(cmd, 0x87, &[]),
        }
    }
}

impl UsbIo for FakeD4Device {
    fn write(&mut self, data: &[u8]) -> Result<()> {
        if self.silent {
            return Ok(());
        }
        if data == d4::ENTER_D4 {
            self.in_d4 = true;
            self.out.extend(b"\x00\x00\x00\x08\x01\x00\xc5\x00");
            return Ok(());
        }
        assert!(self.in_d4, "pacote D4 antes do @EJL 1284.4");
        let length = u16::from_be_bytes([data[2], data[3]]) as usize;
        assert_eq!(length, data.len(), "tamanho do pacote errado");
        let payload = data[6..].to_vec();
        if data[0] == 0 {
            self.transaction(&payload);
        } else {
            assert!(self.channel_open && data[0] == CTRL_SOCKET);
            assert!(self.credit_to_host > 0, "dados sem crédito da impressora");
            self.credit_to_host -= 1;
            let (_, value) = self.printer.handle(&payload);
            self.out.extend(d4::packet(CTRL_SOCKET, CTRL_SOCKET, &value, 0, 0));
        }
        Ok(())
    }

    fn read(&mut self, _timeout_ms: u32) -> Result<Vec<u8>> {
        Ok(std::mem::take(&mut self.out))
    }
}

pub const G2060_STATUS: &[u8] = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<cmd xmlns:ivec=\"http://www.canon.com/ns/cmd/2008/07/common/\">\r\n<ivec:contents>\r\n<ivec:operation>GetStatusResponse</ivec:operation>\r\n<ivec:param_set servicetype=\"print\">\r\n<ivec:response>OK</ivec:response>\r\n<ivec:status>notready</ivec:status>\r\n<ivec:status_detail>AttentionRequired</ivec:status_detail>\r\n<ivec:current_support_code>7800</ivec:current_support_code>\r\n<ivec:marker_info><ivec:ink><ivec:level>-2</ivec:level></ivec:ink></ivec:marker_info>\r\n<ivec:wasteink>\r\n<ivec:item>\r\n<ivec:icon>warning</ivec:icon>\r\n<ivec:model>MC-G02</ivec:model>\r\n<ivec:level>90</ivec:level>\r\n</ivec:item>\r\n</ivec:wasteink>\r\n<ivec:status_detail_list><ivec:item id=\"11\"><ivec:error_code>7800</ivec:error_code></ivec:item></ivec:status_detail_list>\r\n</ivec:param_set>\r\n</ivec:contents>\r\n</cmd>";

/// Responde a toda leitura com o status, como as Canon IVEC.
pub struct FakeIvecDevice {
    pub pending: Vec<u8>,
    pub written: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl FakeIvecDevice {
    pub fn new(status: &[u8]) -> Self {
        FakeIvecDevice { pending: status.to_vec(), written: Default::default() }
    }
}

impl UsbIo for FakeIvecDevice {
    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.written.lock().unwrap().push(data.to_vec());
        Ok(())
    }

    fn read(&mut self, _timeout_ms: u32) -> Result<Vec<u8>> {
        Ok(std::mem::take(&mut self.pending))
    }
}

pub fn err(e: Error) -> String {
    e.message
}
