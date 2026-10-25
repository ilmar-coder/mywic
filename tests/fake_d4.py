"""Dispositivo USB simulado que fala D4 e responde EPSON-CTRL como a FakePrinter."""

import struct

from ecoreset import d4
from ecoreset.printer import EPCTRL_OID
from tests.fake_printer import FakePrinter

CTRL_SOCKET = 0x40


class FakeD4Device:
    """Tem write()/read() como o UsbDevice. `printer` decide as respostas."""

    def __init__(self, printer: FakePrinter, silent=False):
        self.printer = printer
        self.silent = silent  # simula uma interface que não responde
        self.out = b""
        self.in_d4 = False
        self.channel_open = False
        self.credit_to_host = 0
        self.log = []

    def read(self, timeout_ms):
        data, self.out = self.out, b""
        return data

    def write(self, data):
        if self.silent:
            return
        if data == d4.ENTER_D4:
            self.in_d4 = True
            self.out += b"\x00\x00\x00\x08\x01\x00\xc5\x00"
            return
        assert self.in_d4, "pacote D4 antes do @EJL 1284.4"
        psid, ssid, length, credit, control = struct.unpack(">BBHBB", data[:6])
        assert length == len(data), "tamanho do pacote errado"
        payload = data[6:]
        if psid == 0:
            self._transaction(payload)
        else:
            assert self.channel_open and psid == CTRL_SOCKET
            assert self.credit_to_host > 0, "dados sem crédito da impressora"
            self.credit_to_host -= 1
            frame = payload
            oid = EPCTRL_OID + "." + ".".join(str(b) for b in frame)
            _, value = self.printer.handle(oid)
            self.out += d4.packet(CTRL_SOCKET, CTRL_SOCKET, value)

    def _reply(self, command, result=0, rest=b""):
        self.out += d4.packet(0, 0, bytes([command | 0x80, result]) + rest, credit=1)

    def _transaction(self, payload):
        cmd, args = payload[0], payload[1:]
        self.log.append(cmd)
        if cmd == d4.INIT:
            self._reply(cmd, 0, b"\x10")
        elif cmd == d4.GET_SOCKET_ID:
            ok = args == b"EPSON-CTRL"
            self._reply(cmd, 0 if ok else 0x0A, bytes([CTRL_SOCKET]) + args)
        elif cmd == d4.OPEN_CHANNEL:
            self.channel_open = True
            self._reply(cmd, 0, args[:6] + b"\x00\x00\x00\x00")
        elif cmd == d4.CREDIT_REQUEST:
            self.credit_to_host += 1
            self._reply(cmd, 0, args[:2] + b"\x00\x01")
        elif cmd == d4.CREDIT:
            self._reply(cmd, 0, args[:2])
        elif cmd == d4.CLOSE_CHANNEL:
            self.channel_open = False
            self._reply(cmd, 0, args[:2])
        elif cmd == d4.EXIT:
            self.in_d4 = False
            self._reply(cmd)
        else:
            self._reply(cmd, 0x87)
