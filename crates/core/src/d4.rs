//! IEEE 1284.4 ("D4"): o protocolo que as Epson usam no cabo USB.
//!
//! Abre o canal EPSON-CTRL e troca por ele os mesmos quadros que vão por SNMP.
//! Pacote: psid, ssid, tamanho (2 bytes, big-endian, com o cabeçalho), crédito,
//! controle e dados. O canal 0 é o de transações (Init, OpenChannel, Credit...).

use std::time::{Duration, Instant};

use crate::{Error, Result};

/// "@EJL 1284.4" com um prefixo que também é um pacote D4 válido (comando 0x45).
pub const ENTER_D4: &[u8] = b"\x00\x00\x00\x1b\x01@EJL 1284.4\n@EJL\n@EJL\n";

pub const INIT: u8 = 0x00;
pub const OPEN_CHANNEL: u8 = 0x01;
pub const CLOSE_CHANNEL: u8 = 0x02;
pub const CREDIT: u8 = 0x03;
pub const CREDIT_REQUEST: u8 = 0x04;
pub const EXIT: u8 = 0x08;
pub const GET_SOCKET_ID: u8 = 0x09;
pub const ERROR: u8 = 0x7F;

fn result_text(code: u8) -> String {
    match code {
        0x01 => "não foi possível iniciar a conversa".into(),
        0x02 => "revisão do protocolo não suportada".into(),
        0x04 => "sem recursos agora".into(),
        0x05 => "conexão negada".into(),
        0x06 => "canal já aberto".into(),
        0x08 => "canal não está aberto".into(),
        0x09 => "serviço não disponível".into(),
        0x0A => "serviço não encontrado".into(),
        0x0B => "falha no Init".into(),
        0x0C => "tamanho de pacote inválido".into(),
        _ => format!("resultado D4 0x{code:02X}"),
    }
}

/// Um aparelho USB com os endpoints bulk já escolhidos.
pub trait UsbIo: Send {
    fn write(&mut self, data: &[u8]) -> Result<()>;
    /// Lê o que chegar até o prazo; vazio se nada chegou.
    fn read(&mut self, timeout_ms: u32) -> Result<Vec<u8>>;
}

pub fn packet(psid: u8, ssid: u8, payload: &[u8], credit: u8, control: u8) -> Vec<u8> {
    let mut p = vec![psid, ssid];
    p.extend(((payload.len() + 6) as u16).to_be_bytes());
    p.push(credit);
    p.push(control);
    p.extend_from_slice(payload);
    p
}

/// Sessão D4 sobre um aparelho USB (guarda o que sobrou da última leitura).
pub struct D4 {
    pub t: Box<dyn UsbIo>,
    pub timeout_ms: u32,
    buffer: Vec<u8>,
}

impl D4 {
    pub fn new(t: Box<dyn UsbIo>) -> Self {
        D4 { t, timeout_ms: 5000, buffer: Vec::new() }
    }

    fn read_bytes(&mut self, n: usize, timeout_ms: u32) -> Result<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
        while self.buffer.len() < n {
            let left = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
            if left == 0 {
                return Err(Error::new("a impressora não respondeu pelo USB"));
            }
            let chunk = self.t.read(left)?;
            if chunk.is_empty() {
                // o driver do Windows volta na hora sem dados: espera um pouco em vez de girar
                std::thread::sleep(Duration::from_millis(3));
            }
            self.buffer.extend(chunk);
        }
        let rest = self.buffer.split_off(n);
        Ok(std::mem::replace(&mut self.buffer, rest))
    }

    pub fn read_packet(&mut self, timeout_ms: Option<u32>) -> Result<(u8, Vec<u8>)> {
        let timeout = timeout_ms.unwrap_or(self.timeout_ms);
        let header = self.read_bytes(6, timeout)?;
        let length = u16::from_be_bytes([header[2], header[3]]) as usize;
        if length < 6 {
            return Err(Error::new("pacote D4 inválido"));
        }
        let payload = self.read_bytes(length - 6, timeout)?;
        Ok((header[0], payload))
    }

    /// Comando no canal 0. Devolve os bytes após (código, resultado).
    pub fn transaction(&mut self, command: u8, args: &[u8]) -> Result<Vec<u8>> {
        let mut body = vec![command];
        body.extend_from_slice(args);
        self.t.write(&packet(0, 0, &body, 1, 0))?;
        let payload = loop {
            let (psid, payload) = self.read_packet(None)?;
            if psid == 0 {
                break payload;
            }
        };
        if payload.first() == Some(&ERROR) {
            let code = payload.get(3).copied().unwrap_or(0);
            return Err(Error::d4(code, format!("erro D4 0x{code:02X}")));
        }
        if payload.len() < 2 || payload[0] != command | 0x80 {
            return Err(Error::new(format!("resposta D4 inesperada: {}", hex(&payload))));
        }
        if payload[1] != 0 {
            return Err(Error::d4(payload[1], result_text(payload[1])));
        }
        Ok(payload[2..].to_vec())
    }

    /// Põe a impressora em modo 1284.4. false se ninguém respondeu.
    pub fn enter(&mut self, timeout_ms: u32) -> Result<bool> {
        self.buffer.clear();
        self.t.write(ENTER_D4)?;
        Ok(self.read_bytes(8, timeout_ms).is_ok())
    }

    pub fn init(&mut self) -> Result<()> {
        self.transaction(INIT, &[0x10]).map(|_| ())
    }

    pub fn socket_id(&mut self, service: &str) -> Result<u8> {
        let r = self.transaction(GET_SOCKET_ID, service.as_bytes())?;
        r.first().copied().ok_or_else(|| Error::new("resposta D4 sem o número do canal"))
    }

    pub fn open_channel(&mut self, sock: u8, size: u16) -> Result<()> {
        let mut args = vec![sock, sock];
        args.extend(size.to_be_bytes());
        args.extend(size.to_be_bytes());
        args.extend([0, 0, 0, 0]);
        for _ in 0..5 {
            match self.transaction(OPEN_CHANNEL, &args) {
                Ok(_) => return Ok(()),
                Err(e) if e.d4_code == Some(0x06) => {
                    let _ = self.close_channel(sock);
                }
                Err(e) if e.d4_code == Some(0x04) => {}
                Err(e) => return Err(e),
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(Error::new("não consegui abrir o canal EPSON-CTRL"))
    }

    pub fn close_channel(&mut self, sock: u8) -> Result<()> {
        self.transaction(CLOSE_CHANNEL, &[sock, sock, 0]).map(|_| ())
    }

    pub fn give_credit(&mut self, sock: u8, n: u16) -> Result<()> {
        let mut args = vec![sock, sock];
        args.extend(n.to_be_bytes());
        self.transaction(CREDIT, &args).map(|_| ())
    }

    pub fn request_credit(&mut self, sock: u8) -> Result<u16> {
        let r = self.transaction(CREDIT_REQUEST, &[sock, sock, 0x00, 0x80, 0xFF, 0xFF])?;
        Ok(r.get(2..4).map(|b| u16::from_be_bytes([b[0], b[1]])).unwrap_or(0))
    }

    pub fn exit(&mut self) -> Result<()> {
        self.transaction(EXIT, &[]).map(|_| ())
    }

    /// Pede crédito para enviar, dá crédito para a resposta, envia e lê.
    pub fn exchange(&mut self, sock: u8, data: &[u8]) -> Result<Vec<u8>> {
        if self.request_credit(sock)? == 0 {
            return Err(Error::new("a impressora não deu crédito para enviar"));
        }
        self.give_credit(sock, 1)?;
        self.t.write(&packet(sock, sock, data, 0, 1))?;
        loop {
            let (psid, payload) = self.read_packet(None)?;
            if psid == sock {
                return Ok(payload);
            }
        }
    }
}

pub fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// Canal EPSON-CTRL aberto sobre D4: command(quadro) -> resposta.
pub struct EpsonCtrlChannel {
    pub d4: D4,
    sock: Option<u8>,
}

impl EpsonCtrlChannel {
    /// Entra no D4 e abre o canal. Err(aparelho) se a interface não respondeu,
    /// para quem chamou poder tentar outra.
    pub fn open(io: Box<dyn UsbIo>) -> std::result::Result<EpsonCtrlChannel, (Box<dyn UsbIo>, Error)> {
        let mut d4 = D4::new(io);
        let attempt = (|| -> Result<Option<u8>> {
            if !d4.enter(2000)? {
                return Ok(None);
            }
            d4.init()?;
            let sock = d4.socket_id("EPSON-CTRL")?;
            d4.open_channel(sock, 0x0200)?;
            Ok(Some(sock))
        })();
        match attempt {
            Ok(Some(sock)) => Ok(EpsonCtrlChannel { d4, sock: Some(sock) }),
            Ok(None) => Err((d4.t, Error::new("a interface não respondeu ao D4"))),
            Err(e) => {
                // não deixar a impressora presa no modo D4
                let _ = d4.exit();
                Err((d4.t, e))
            }
        }
    }

    pub fn command(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        let sock = self.sock.ok_or_else(|| Error::new("canal EPSON-CTRL fechado"))?;
        self.d4.exchange(sock, frame)
    }

    /// Fecha o canal e sai do modo 1284.4, para a impressora voltar a imprimir.
    pub fn close(&mut self) {
        if let Some(sock) = self.sock.take() {
            let _ = self.d4.close_channel(sock);
            let _ = self.d4.exit();
        }
    }
}

impl Drop for EpsonCtrlChannel {
    fn drop(&mut self) {
        self.close();
    }
}
