//! Cliente SNMPv1 mínimo (só GET), com o BER/ASN.1 feito à mão.

use std::net::UdpSocket;
use std::time::Duration;

use crate::{Error, Result};

pub const INTEGER: u8 = 0x02;
pub const OCTET_STRING: u8 = 0x04;
pub const NULL: u8 = 0x05;
pub const OID: u8 = 0x06;
pub const SEQUENCE: u8 = 0x30;
pub const GET_REQUEST: u8 = 0xA0;
pub const GET_RESPONSE: u8 = 0xA2;

fn encode_length(n: usize) -> Vec<u8> {
    if n < 0x80 {
        return vec![n as u8];
    }
    let body: Vec<u8> = n.to_be_bytes().iter().copied().skip_while(|b| *b == 0).collect();
    let mut out = vec![0x80 | body.len() as u8];
    out.extend(body);
    out
}

pub fn tlv(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend(encode_length(value.len()));
    out.extend_from_slice(value);
    out
}

pub fn encode_int(n: i64) -> Vec<u8> {
    let bytes = n.to_be_bytes();
    // menor forma em complemento de dois
    let mut start = 0;
    while start < 7 {
        let (b, next) = (bytes[start], bytes[start + 1]);
        if (b == 0 && next & 0x80 == 0) || (b == 0xFF && next & 0x80 != 0) {
            start += 1;
        } else {
            break;
        }
    }
    tlv(INTEGER, &bytes[start..])
}

pub fn encode_oid(oid: &str) -> Result<Vec<u8>> {
    let arcs: Vec<u64> = oid
        .trim_matches('.')
        .split('.')
        .map(|a| a.parse::<u64>().map_err(|_| Error::new(format!("OID inválido: {oid}"))))
        .collect::<Result<_>>()?;
    if arcs.len() < 2 {
        return Err(Error::new(format!("OID inválido: {oid}")));
    }
    let mut out = vec![(40 * arcs[0] + arcs[1]) as u8];
    for &arc in &arcs[2..] {
        let mut chunk = vec![(arc & 0x7F) as u8];
        let mut rest = arc >> 7;
        while rest > 0 {
            chunk.push(0x80 | (rest & 0x7F) as u8);
            rest >>= 7;
        }
        chunk.reverse();
        out.extend(chunk);
    }
    Ok(tlv(OID, &out))
}

/// (tag, valor, próxima posição)
pub fn decode_tlv(data: &[u8], pos: usize) -> Result<(u8, &[u8], usize)> {
    let short = || Error::new("resposta SNMP truncada");
    let tag = *data.get(pos).ok_or_else(short)?;
    let mut length = *data.get(pos + 1).ok_or_else(short)? as usize;
    let mut pos = pos + 2;
    if length & 0x80 != 0 {
        let count = length & 0x7F;
        let bytes = data.get(pos..pos + count).ok_or_else(short)?;
        length = bytes.iter().fold(0usize, |acc, b| (acc << 8) | *b as usize);
        pos += count;
    }
    let end = pos + length;
    let value = data.get(pos..end).ok_or_else(short)?;
    Ok((tag, value, end))
}

pub fn decode_children(data: &[u8]) -> Result<Vec<(u8, &[u8])>> {
    let mut items = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let (tag, value, next) = decode_tlv(data, pos)?;
        items.push((tag, value));
        pos = next;
    }
    Ok(items)
}

pub fn decode_oid(value: &[u8]) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut arcs = vec![(value[0] / 40) as u64, (value[0] % 40) as u64];
    let mut n: u64 = 0;
    for &b in &value[1..] {
        n = (n << 7) | (b & 0x7F) as u64;
        if b & 0x80 == 0 {
            arcs.push(n);
            n = 0;
        }
    }
    arcs.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(".")
}

fn decode_int(raw: &[u8]) -> i64 {
    let mut n: i64 = if raw.first().is_some_and(|b| b & 0x80 != 0) { -1 } else { 0 };
    for &b in raw {
        n = (n << 8) | b as i64;
    }
    n
}

pub fn build_get(oid: &str, community: &str, request_id: i64) -> Result<Vec<u8>> {
    let mut varbind = encode_oid(oid)?;
    varbind.extend(tlv(NULL, &[]));
    let varbind = tlv(SEQUENCE, &varbind);
    let mut pdu = encode_int(request_id);
    pdu.extend(encode_int(0));
    pdu.extend(encode_int(0));
    pdu.extend(tlv(SEQUENCE, &varbind));
    let mut msg = encode_int(0);
    msg.extend(tlv(OCTET_STRING, community.as_bytes()));
    msg.extend(tlv(GET_REQUEST, &pdu));
    Ok(tlv(SEQUENCE, &msg))
}

/// (request_id, error_status, tag do valor, valor)
pub fn parse_response(packet: &[u8]) -> Result<(i64, i64, u8, Vec<u8>)> {
    let (tag, message, _) = decode_tlv(packet, 0)?;
    if tag != SEQUENCE {
        return Err(Error::new("pacote SNMP inválido"));
    }
    let parts = decode_children(message)?;
    if parts.len() < 3 || parts[2].0 != GET_RESPONSE {
        return Err(Error::new("não é uma resposta GET"));
    }
    let fields = decode_children(parts[2].1)?;
    if fields.len() < 4 {
        return Err(Error::new("resposta SNMP incompleta"));
    }
    let request_id = decode_int(fields[0].1);
    let error_status = decode_int(fields[1].1);
    let varbinds = decode_children(fields[3].1)?;
    let Some(first) = varbinds.first() else {
        return Ok((request_id, error_status, NULL, Vec::new()));
    };
    let pair = decode_children(first.1)?;
    let (vtag, value) = pair.get(1).copied().unwrap_or((NULL, &[]));
    Ok((request_id, error_status, vtag, value.to_vec()))
}

pub struct SnmpClient {
    pub host: String,
    pub port: u16,
    pub community: String,
    pub timeout: Duration,
    pub retries: u32,
}

impl SnmpClient {
    pub fn new(host: &str) -> Self {
        SnmpClient {
            host: host.to_string(),
            port: 161,
            community: "public".into(),
            timeout: Duration::from_secs(2),
            retries: 2,
        }
    }

    /// GET de um OID: o valor (OctetString) ou None se a impressora não tem o objeto.
    pub fn get(&self, oid: &str) -> Result<Option<Vec<u8>>> {
        let request_id = fastrand::i64(1..0x7FFF_FFFF);
        let packet = build_get(oid, &self.community, request_id)?;
        let sock = UdpSocket::bind(("0.0.0.0", 0))?;
        sock.set_read_timeout(Some(self.timeout))?;
        let target = (self.host.as_str(), self.port);
        let mut buf = vec![0u8; 65535];
        for _ in 0..=self.retries {
            sock.send_to(&packet, target)?;
            loop {
                let n = match sock.recv_from(&mut buf) {
                    Ok((n, _)) => n,
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut) => break,
                    Err(e) => return Err(e.into()),
                };
                let Ok((rid, error, tag, value)) = parse_response(&buf[..n]) else { continue };
                if rid != request_id {
                    continue;
                }
                if error != 0 {
                    return Err(Error::new(format!("impressora respondeu com erro SNMP {error}")));
                }
                if matches!(tag, NULL | 0x80 | 0x81 | 0x82) {
                    return Ok(None);
                }
                return Ok(Some(value));
            }
        }
        Err(Error::new(format!(
            "sem resposta de {}:{} (a impressora está ligada e na rede?)",
            self.host, self.port
        )))
    }
}
