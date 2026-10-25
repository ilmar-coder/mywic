"""IEEE 1284.4 ("D4"): o protocolo que as Epson usam no cabo USB.

Abre o canal EPSON-CTRL e troca por ele os mesmos comandos que vão por SNMP
(os "||" da EEPROM, "di", "st", "rw"...). Funciona sobre qualquer transporte
com write(bytes) e read(timeout_ms) -> bytes.

Pacote: psid, ssid, tamanho (2 bytes, big-endian, com o cabeçalho), crédito,
controle e dados. O canal 0 é o de transações (Init, OpenChannel, Credit...).
"""

import struct
import time

# "@EJL 1284.4" com um prefixo que também é um pacote D4 válido (comando 0x45).
ENTER_D4 = b"\x00\x00\x00\x1b\x01@EJL 1284.4\n@EJL\n@EJL\n"

INIT, OPEN_CHANNEL, CLOSE_CHANNEL, CREDIT, CREDIT_REQUEST = 0x00, 0x01, 0x02, 0x03, 0x04
EXIT, GET_SOCKET_ID, ERROR = 0x08, 0x09, 0x7F

RESULTS = {
    0x01: "não foi possível iniciar a conversa",
    0x02: "revisão do protocolo não suportada",
    0x04: "sem recursos agora",
    0x05: "conexão negada",
    0x06: "canal já aberto",
    0x08: "canal não está aberto",
    0x09: "serviço não disponível",
    0x0A: "serviço não encontrado",
    0x0B: "falha no Init",
    0x0C: "tamanho de pacote inválido",
}


class D4Error(Exception):
    pass


def packet(psid, ssid, payload, credit=0, control=0):
    return struct.pack(">BBHBB", psid, ssid, len(payload) + 6, credit, control) + payload


class D4:
    def __init__(self, transport, timeout_ms=5000):
        self.t = transport
        self.timeout_ms = timeout_ms
        self.buffer = b""

    # ---- pacotes ----------------------------------------------------------

    def _read_bytes(self, n, timeout_ms):
        deadline = time.monotonic() + timeout_ms / 1000
        while len(self.buffer) < n:
            left = int((deadline - time.monotonic()) * 1000)
            if left <= 0:
                raise D4Error("a impressora não respondeu pelo USB")
            self.buffer += self.t.read(left)
        data, self.buffer = self.buffer[:n], self.buffer[n:]
        return data

    def read_packet(self, timeout_ms=None):
        timeout_ms = timeout_ms or self.timeout_ms
        header = self._read_bytes(6, timeout_ms)
        psid, ssid, length, credit, control = struct.unpack(">BBHBB", header)
        if length < 6:
            raise D4Error("pacote D4 inválido")
        return psid, self._read_bytes(length - 6, timeout_ms)

    def transaction(self, command, args=b"", timeout_ms=None):
        """Comando no canal 0. Devolve os bytes após (código, resultado)."""
        self.t.write(packet(0, 0, bytes([command]) + args, credit=1))
        while True:
            psid, payload = self.read_packet(timeout_ms)
            if psid == 0:
                break
        if payload[:1] == bytes([ERROR]):
            code = payload[3] if len(payload) > 3 else 0
            raise D4Error("erro D4 0x%02X" % code)
        if payload[0] != command | 0x80:
            raise D4Error("resposta D4 inesperada: %s" % payload.hex())
        result = payload[1]
        if result:
            raise D4Error(RESULTS.get(result, "resultado D4 0x%02X" % result), result)
        return payload[2:]

    # ---- sessão -----------------------------------------------------------

    def enter(self, timeout_ms=2000):
        """Põe a impressora em modo 1284.4. Devolve False se ninguém respondeu."""
        self.buffer = b""
        self.t.write(ENTER_D4)
        try:
            self._read_bytes(8, timeout_ms)
        except D4Error:
            return False
        return True

    def init(self):
        self.transaction(INIT, b"\x10")

    def socket_id(self, service):
        return self.transaction(GET_SOCKET_ID, service.encode("ascii"))[0]

    def open_channel(self, sock, size=0x0200):
        args = struct.pack(">BBHHHH", sock, sock, size, size, 0, 0)
        for _ in range(5):
            try:
                self.transaction(OPEN_CHANNEL, args)
                return
            except D4Error as e:
                code = e.args[1] if len(e.args) > 1 else None
                if code == 0x06:
                    self.close_channel(sock)
                elif code != 0x04:
                    raise
            time.sleep(0.2)
        raise D4Error("não consegui abrir o canal EPSON-CTRL")

    def close_channel(self, sock):
        self.transaction(CLOSE_CHANNEL, bytes([sock, sock, 0]))

    def give_credit(self, sock, n=1):
        self.transaction(CREDIT, struct.pack(">BBH", sock, sock, n))

    def request_credit(self, sock):
        reply = self.transaction(CREDIT_REQUEST, bytes([sock, sock]) + b"\x00\x80\xff\xff")
        return struct.unpack(">H", reply[2:4])[0]

    def exit(self):
        self.transaction(EXIT)

    # ---- dados ------------------------------------------------------------

    def exchange(self, sock, data, timeout_ms=None):
        """Pede crédito para enviar, dá crédito para a resposta, envia e lê."""
        if self.request_credit(sock) <= 0:
            raise D4Error("a impressora não deu crédito para enviar")
        self.give_credit(sock, 1)
        self.t.write(packet(sock, sock, data, control=1))
        while True:
            psid, payload = self.read_packet(timeout_ms)
            if psid == sock:
                return payload


class EpsonCtrlChannel:
    """Canal EPSON-CTRL aberto sobre D4: command(quadro) -> resposta."""

    def __init__(self, transport):
        self.d4 = D4(transport)
        self.sock = None

    def open(self):
        if not self.d4.enter():
            return False
        self.d4.init()
        self.sock = self.d4.socket_id("EPSON-CTRL")
        self.d4.open_channel(self.sock)
        return True

    def command(self, frame):
        return self.d4.exchange(self.sock, frame)

    def close(self):
        """Fecha o canal e sai do modo 1284.4, para a impressora voltar a imprimir."""
        try:
            if self.sock is not None:
                self.d4.close_channel(self.sock)
            self.d4.exit()
        except D4Error:
            pass
        self.sock = None
