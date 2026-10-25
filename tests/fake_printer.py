"""Impressora Epson simulada: agente SNMP em UDP que responde aos comandos EPSON-CTRL."""

import hashlib
import socket
import struct
import threading

from ecoreset import snmp
from ecoreset.printer import EPCTRL_OID, MODEL_OID, shift_key


def _response(request_id, oid, tag, value, community=b"public"):
    varbind = snmp._tlv(snmp.SEQUENCE, snmp._encode_oid(oid) + snmp._tlv(tag, value))
    pdu = snmp._tlv(snmp.GET_RESPONSE, snmp._encode_int(request_id) + snmp._encode_int(0)
                    + snmp._encode_int(0) + snmp._tlv(snmp.SEQUENCE, varbind))
    return snmp._tlv(snmp.SEQUENCE, snmp._encode_int(0)
                     + snmp._tlv(snmp.OCTET_STRING, community) + pdu)


def _parse_request(packet):
    _, message, _ = snmp._decode_tlv(packet, 0)
    parts = snmp._decode_children(message)
    fields = snmp._decode_children(parts[2][1])
    request_id = int.from_bytes(fields[0][1], "big", signed=True)
    varbind = snmp._decode_children(snmp._decode_children(fields[3][1])[0][1])
    return request_id, snmp.decode_oid(varbind[0][1])


class FakePrinter:
    def __init__(self, name="EPSON L3150 Series", read_key=(151, 7),
                 write_key=b"Maribaya", eeprom=None, locked=False):
        self.name = name
        self.read_key = bytes(read_key)
        self.write_key = shift_key(write_key)
        self.eeprom = dict(eeprom or {})
        self.locked = locked
        self.serial = "X5NY012345"
        self.temp_resets = 0
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.port = self.sock.getsockname()[1]
        self.thread = threading.Thread(target=self._serve, daemon=True)
        self.thread.start()

    def close(self):
        self.sock.close()

    def _serve(self):
        while True:
            try:
                packet, addr = self.sock.recvfrom(65535)
            except OSError:
                return
            request_id, oid = _parse_request(packet)
            tag, value = self.handle(oid)
            self.sock.sendto(_response(request_id, oid, tag, value), addr)

    def handle(self, oid):
        if oid == MODEL_OID:
            return snmp.OCTET_STRING, self.name.encode()
        if not oid.startswith(EPCTRL_OID + "."):
            return 0x80, b""  # noSuchObject
        data = bytes(int(x) for x in oid[len(EPCTRL_OID) + 1:].split("."))
        cmd, payload = data[:2], data[4:]
        if cmd == b"di":
            return snmp.OCTET_STRING, b"@EJL ID\r\nMFG:EPSON;MDL:%s;\x0c" % self.name.encode()
        if cmd == b"st":
            serial = self.serial.encode()
            items = b"\x01\x01\x04" + b"\x1f" + bytes([len(serial)]) + serial
            return snmp.OCTET_STRING, (b"\x00@BDC ST2\r\n" + struct.pack("<H", len(items))
                                       + items)
        if cmd == b"rw":
            ok = payload[2:] == hashlib.sha1(self.serial.encode()).digest()
            self.temp_resets += ok
            return snmp.OCTET_STRING, b"rw:01:OK;\x0c" if ok else b"rw:01:NA;\x0c"
        if cmd != b"||":
            return snmp.OCTET_STRING, cmd + b":NA;\x0c"
        if payload[:2] != self.read_key or (self.locked == "all"):
            return snmp.OCTET_STRING, b"@BDC PS\r\nee:NA;\x0c"
        address = payload[5] | (payload[6] << 8)
        if payload[2] == 0x41:  # leitura
            value = self.eeprom.get(address, 0)
            return snmp.OCTET_STRING, b"@BDC PS\r\nEE:%04X%02X;\x0c" % (address, value)
        if payload[2] == 0x42:  # gravação
            if self.locked or payload[8:16] != self.write_key:
                return snmp.OCTET_STRING, b"||:42:NA;\x0c"
            self.eeprom[address] = payload[7]
            return snmp.OCTET_STRING, b"||:42:OK;\x0c"
        return snmp.OCTET_STRING, b"||:NA;\x0c"
