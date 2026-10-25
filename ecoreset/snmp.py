"""Cliente SNMPv1 mínimo (só GET), feito com a biblioteca padrão.

Implementa só o necessário de BER/ASN.1 para conversar com impressoras Epson.
"""

import random
import socket

INTEGER = 0x02
OCTET_STRING = 0x04
NULL = 0x05
OID = 0x06
SEQUENCE = 0x30
GET_REQUEST = 0xA0
GET_RESPONSE = 0xA2


class SnmpError(Exception):
    pass


def _encode_length(n):
    if n < 0x80:
        return bytes([n])
    body = n.to_bytes((n.bit_length() + 7) // 8, "big")
    return bytes([0x80 | len(body)]) + body


def _tlv(tag, value):
    return bytes([tag]) + _encode_length(len(value)) + value


def _encode_int(n):
    length = max(1, (n.bit_length() + 8) // 8)
    return _tlv(INTEGER, n.to_bytes(length, "big", signed=True))


def _encode_oid(oid):
    arcs = [int(a) for a in oid.strip(".").split(".")]
    if len(arcs) < 2:
        raise ValueError("OID inválido: %r" % oid)
    out = bytearray([40 * arcs[0] + arcs[1]])
    for arc in arcs[2:]:
        chunk = [arc & 0x7F]
        arc >>= 7
        while arc:
            chunk.append(0x80 | (arc & 0x7F))
            arc >>= 7
        out.extend(reversed(chunk))
    return _tlv(OID, bytes(out))


def _decode_tlv(data, pos):
    """Devolve (tag, valor, próxima_posição)."""
    tag = data[pos]
    length = data[pos + 1]
    pos += 2
    if length & 0x80:
        count = length & 0x7F
        length = int.from_bytes(data[pos:pos + count], "big")
        pos += count
    end = pos + length
    if end > len(data):
        raise SnmpError("resposta SNMP truncada")
    return tag, data[pos:end], end


def _decode_children(data):
    items, pos = [], 0
    while pos < len(data):
        tag, value, pos = _decode_tlv(data, pos)
        items.append((tag, value))
    return items


def decode_oid(value):
    arcs = [value[0] // 40, value[0] % 40]
    n = 0
    for b in value[1:]:
        n = (n << 7) | (b & 0x7F)
        if not b & 0x80:
            arcs.append(n)
            n = 0
    return ".".join(str(a) for a in arcs)


def build_get(oid, community="public", request_id=1):
    varbind = _tlv(SEQUENCE, _encode_oid(oid) + _tlv(NULL, b""))
    pdu = _tlv(GET_REQUEST, _encode_int(request_id) + _encode_int(0)
               + _encode_int(0) + _tlv(SEQUENCE, varbind))
    return _tlv(SEQUENCE, _encode_int(0) + _tlv(OCTET_STRING, community.encode())
                + pdu)


def parse_response(packet):
    """Devolve (request_id, error_status, tag_do_valor, valor)."""
    tag, message, _ = _decode_tlv(packet, 0)
    if tag != SEQUENCE:
        raise SnmpError("pacote SNMP inválido")
    parts = _decode_children(message)
    if len(parts) < 3 or parts[2][0] != GET_RESPONSE:
        raise SnmpError("não é uma resposta GET")
    fields = _decode_children(parts[2][1])
    request_id = int.from_bytes(fields[0][1], "big", signed=True)
    error_status = int.from_bytes(fields[1][1], "big", signed=True)
    varbinds = _decode_children(fields[3][1])
    if not varbinds:
        return request_id, error_status, NULL, b""
    value_tag, value = _decode_children(varbinds[0][1])[1]
    return request_id, error_status, value_tag, value


class SnmpClient:
    def __init__(self, host, port=161, community="public", timeout=2.0, retries=2):
        self.host = host
        self.port = port
        self.community = community
        self.timeout = timeout
        self.retries = retries

    def get(self, oid):
        """Faz um GET e devolve o valor em bytes (OctetString) ou None."""
        request_id = random.randint(1, 0x7FFFFFFF)
        packet = build_get(oid, self.community, request_id)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(self.timeout)
            for _ in range(self.retries + 1):
                sock.sendto(packet, (self.host, self.port))
                try:
                    while True:
                        data, _ = sock.recvfrom(65535)
                        rid, error, tag, value = parse_response(data)
                        if rid == request_id:
                            break
                except socket.timeout:
                    continue
                if error:
                    raise SnmpError("impressora respondeu com erro SNMP %d" % error)
                if tag in (NULL, 0x80, 0x81, 0x82):  # NULL / noSuchObject / ...
                    return None
                return value
        raise SnmpError("sem resposta de %s:%d (a impressora está ligada e na rede?)"
                        % (self.host, self.port))
