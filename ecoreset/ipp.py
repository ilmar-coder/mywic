"""Cliente IPP mínimo (Get-Printer-Attributes) para ler estado e alertas.

Funciona com Epson, Canon e qualquer impressora AirPrint/IPP Everywhere, pela
rede ou pelo USB quando o sistema expõe o IPP-over-USB (ippusb no Mac).
"""

import http.client
import re
import ssl
import struct
import subprocess
import urllib.parse

OPERATION_ATTRS, END = 0x01, 0x03
CHARSET, LANGUAGE, URI = 0x47, 0x48, 0x45

# Motivos padrão do IPP (RFC 8011 / PWG 5100) mais comuns, em português.
REASONS = {
    "none": "Sem problemas",
    "media-empty": "Sem papel",
    "media-needed": "Coloque papel",
    "media-jam": "Papel enroscado",
    "media-low": "Papel acabando",
    "cover-open": "Tampa aberta",
    "door-open": "Porta/tampa aberta",
    "input-tray-missing": "Bandeja ausente",
    "marker-supply-low": "Tinta baixa",
    "marker-supply-empty": "Sem tinta",
    "marker-waste-almost-full": "Almofada/caixa de manutenção quase cheia",
    "marker-waste-full": "Almofada/caixa de manutenção cheia",
    "toner-low": "Tinta/toner baixo",
    "toner-empty": "Sem tinta/toner",
    "offline-report": "Impressora desconectada ou desligada",
    "paused": "Pausada",
    "moving-to-paused": "Pausando",
    "shutdown": "Desligando",
    "connecting-to-device": "Conectando",
    "timed-out": "Sem resposta",
    "stopped-partly": "Parcialmente parada",
    "spool-area-full": "Fila cheia",
    "interpreter-resource-unavailable": "Recurso interno indisponível",
    "other": "Outro problema (veja a mensagem)",
}


class IppError(Exception):
    pass


def _attr(tag, name, value):
    name, value = name.encode(), value.encode()
    return struct.pack(">BH", tag, len(name)) + name + struct.pack(">H", len(value)) + value


def build_request(printer_uri, request_id=1):
    return (struct.pack(">BBHI", 1, 1, 0x000B, request_id) + bytes([OPERATION_ATTRS])
            + _attr(CHARSET, "attributes-charset", "utf-8")
            + _attr(LANGUAGE, "attributes-natural-language", "en")
            + _attr(URI, "printer-uri", printer_uri) + bytes([END]))


def _decode_value(tag, raw):
    if tag in (0x21, 0x23) and len(raw) == 4:  # integer, enum
        return struct.unpack(">i", raw)[0]
    if tag == 0x22 and len(raw) == 1:  # boolean
        return bool(raw[0])
    if tag in (0x35, 0x36) and len(raw) >= 4:  # textWithLanguage, nameWithLanguage
        n = struct.unpack(">H", raw[:2])[0]
        m = struct.unpack(">H", raw[2 + n:4 + n])[0]
        return raw[4 + n:4 + n + m].decode("utf-8", "replace")
    if 0x30 <= tag <= 0x4A:
        return raw.decode("utf-8", "replace")
    return raw


def parse_response(data):
    """Devolve (status_code, {atributo: [valores]}) dos grupos de atributos."""
    if len(data) < 8:
        raise IppError("resposta IPP curta demais")
    status = struct.unpack(">H", data[2:4])[0]
    attrs, pos, name = {}, 8, None
    while pos < len(data):
        tag = data[pos]
        pos += 1
        if tag == END:
            break
        if tag < 0x10:  # delimitador de grupo
            continue
        n = struct.unpack(">H", data[pos:pos + 2])[0]
        pos += 2
        if n:
            name = data[pos:pos + n].decode("utf-8", "replace")
            attrs.setdefault(name, [])
        pos += n
        v = struct.unpack(">H", data[pos:pos + 2])[0]
        pos += 2
        if name is not None:
            attrs[name].append(_decode_value(tag, data[pos:pos + v]))
        pos += v
    return status, attrs


def get_printer_attributes(url, timeout=5.0):
    """url: ipp://host[:porta]/caminho ou ipps://..."""
    u = urllib.parse.urlsplit(url)
    secure = u.scheme in ("ipps", "https")
    port = u.port or 631
    if secure:
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE  # impressoras usam certificado próprio
        conn = http.client.HTTPSConnection(u.hostname, port, timeout=timeout, context=ctx)
    else:
        conn = http.client.HTTPConnection(u.hostname, port, timeout=timeout)
    printer_uri = urllib.parse.urlunsplit(("ipps" if secure else "ipp", u.netloc,
                                           u.path or "/ipp/print", "", ""))
    try:
        conn.request("POST", u.path or "/ipp/print", body=build_request(printer_uri),
                     headers={"Content-Type": "application/ipp"})
        resp = conn.getresponse()
        body = resp.read()
    except (OSError, http.client.HTTPException) as e:
        raise IppError("não consegui falar IPP com %s: %s" % (u.hostname, e))
    finally:
        conn.close()
    if resp.status != 200:
        raise IppError("a impressora respondeu HTTP %d" % resp.status)
    status, attrs = parse_response(body)
    if status >= 0x0400:
        raise IppError("erro IPP 0x%04X" % status)
    return attrs


def find_queue_url(queue, timeout=4.0):
    """Endereço IPP de uma impressora instalada no CUPS (rede, Bonjour ou ippusb)."""
    out = subprocess.run(["lpstat", "-v", queue], capture_output=True, text=True,
                         timeout=10).stdout
    m = re.search(r"(\w+)://(\S+)", out)
    if not m:
        raise IppError("fila %s não encontrada" % queue)
    scheme, rest = m.groups()
    if scheme in ("ipp", "ipps") and "._tcp" not in rest:
        return "%s://%s" % (scheme, rest)
    service = urllib.parse.unquote(rest.split("/")[0]).rstrip(".")
    name, _, kind = service.partition("._")
    kind = "_" + kind.replace(".local", "")
    return resolve_bonjour(name, kind, timeout)


def resolve_bonjour(name, kind="_ipp._tcp", timeout=4.0):
    try:
        out = subprocess.run(["dns-sd", "-L", name, kind, "local"], capture_output=True,
                             text=True, timeout=timeout).stdout
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode() if isinstance(e.stdout, bytes) else (e.stdout or "")
    m = re.search(r"can be reached at (\S+?)\.?:(\d+)", out)
    if not m:
        raise IppError("não achei %s na rede local (Bonjour)" % name)
    host, port = m.groups()
    rp = re.search(r"\brp=(\S+)", out)
    scheme = "ipps" if kind.startswith("_ipps") else "ipp"
    return "%s://%s:%s/%s" % (scheme, host, port, rp.group(1) if rp else "ipp/print")


def support_codes(attrs):
    """Códigos de 4 caracteres (estilo Canon) citados nas mensagens/alertas."""
    texts = []
    for key in ("printer-state-message", "printer-alert-description", "printer-alert",
                "printer-state-reasons"):
        texts += [v for v in attrs.get(key, []) if isinstance(v, str)]
    codes = []
    for t in texts:
        for c in re.findall(r"(?<![0-9A-Za-z])([0-9][0-9A-F]{3}|B20[0-9A-F])(?![0-9A-Za-z])",
                            t):
            if c not in codes:
                codes.append(c)
    return codes


def reasons(attrs):
    """[(palavra-chave, explicação)] sem os sufixos -report/-warning/-error."""
    found = []
    for r in attrs.get("printer-state-reasons", []):
        base = re.sub(r"-(report|warning|error)$", "", r)
        found.append((r, REASONS.get(base, REASONS.get(r, r))))
    return found
