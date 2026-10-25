"""Comandos EPSON-CTRL via SNMP: identificação, EEPROM e contadores da almofada."""

import datetime
import hashlib
import json
import re
import struct

from .errors import EPSON_STATUS, EPSON_WARNINGS, explain_epson_error
from .models import MODELS, find_model
from .snmp import SnmpClient

EPCTRL_OID = "1.3.6.1.4.1.1248.1.2.2.44.1.1.2.1"
MODEL_OID = "1.3.6.1.2.1.25.3.2.1.3.1"

_EE_RE = re.compile(rb"EE:([0-9A-Fa-f]{6})")
_DI_RE = re.compile(r"(MFG|CMD|MDL|CLS|DES):([^;]*);")


class PrinterError(Exception):
    pass


def ctrl_oid(cmd, payload=b""):
    """OID de um comando EPSON-CTRL: 2 letras + tamanho (LE, 2 bytes) + dados."""
    data = cmd.encode("ascii") + struct.pack("<H", len(payload)) + payload
    return EPCTRL_OID + "." + ".".join(str(b) for b in data)


def shift_key(key):
    """A chave de gravação vai com cada byte somado de 1 (0 continua 0)."""
    return bytes(b + 1 if b else 0 for b in key)


def read_oid(read_key, address):
    payload = bytes([read_key[0], read_key[1], 0x41, 0xBE, 0xA0,
                     address & 0xFF, address >> 8])
    return ctrl_oid("||", payload)


def write_oid(read_key, write_key, address, value):
    if len(write_key) != 8:
        raise ValueError("a chave de gravação tem 8 bytes")
    payload = bytes([read_key[0], read_key[1], 0x42, 0xBD, 0x21,
                     address & 0xFF, address >> 8, value]) + shift_key(write_key)
    return ctrl_oid("||", payload)


def parse_status(reply):
    """Decodifica a resposta '@BDC ST2' em {tipo: bytes}. 0x1F é o número de série."""
    start = reply.find(b"BDC ST2\r\n")
    if start < 0:
        return {}
    pos = start + len(b"BDC ST2\r\n") + 2  # pula o tamanho total
    items = {}
    while pos + 2 <= len(reply):
        kind, length = reply[pos], reply[pos + 1]
        items[kind] = reply[pos + 2:pos + 2 + length]
        pos += 2 + length
    return items


MAC_ADDRESSES = range(0x780, 0x786)  # MAC do Wi-Fi nos modelos que o guardam aqui
MAX_IMAGE = 4096


def load_image(path, start=0):
    """Lê um arquivo de EEPROM: JSON (backup/leitura do EcoReset) ou binário.

    Devolve ({endereço: valor}, modelo_ou_None). No binário, o primeiro byte
    vai para o endereço `start`.
    """
    with open(path, "rb") as f:
        raw = f.read()
    if path.lower().endswith(".json"):
        data = json.loads(raw.decode("utf-8"))
        cells = data.get("eeprom", data)
        image = {int(a, 0) if isinstance(a, str) else int(a): int(v)
                 for a, v in cells.items()}
        model = data.get("model")
    else:
        image = {start + i: b for i, b in enumerate(raw)}
        model = None
    for a, v in image.items():
        if not (0 <= a <= 0xFFFF and 0 <= v <= 255):
            raise PrinterError("arquivo inválido: endereço %r valor %r" % (a, v))
    return image, model


def waste_percent(values, divider):
    """values: bytes na ordem dos endereços (menos significativo primeiro)."""
    raw = 0
    for b in reversed(values):
        raw = (raw << 8) | b
    return round(raw / divider, 2)


class EpsonPrinter:
    def __init__(self, host, model=None, read_key=None, write_key=None,
                 port=161, timeout=2.0, retries=2, client=None):
        self.snmp = client or SnmpClient(host, port=port, timeout=timeout,
                                         retries=retries)
        self.host = host
        self.model = None
        self.read_key = read_key
        self.write_key = write_key
        if model:
            self.set_model(model)

    # ---- identificação ----------------------------------------------------

    def set_model(self, name):
        if name not in MODELS:
            raise PrinterError("modelo desconhecido: %s" % name)
        self.model = name
        cfg = MODELS[name]
        self.read_key = cfg["read_key"]
        self.write_key = cfg["write_key"]

    @property
    def config(self):
        return MODELS.get(self.model, {})

    def model_string(self):
        value = self.snmp.get(MODEL_OID)
        return value.decode("latin-1").strip() if value else ""

    def device_id(self):
        """Resposta do comando 'di' (MFG, MDL, CMD...) como dicionário."""
        value = self.snmp.get(ctrl_oid("di", b"\x01"))
        if not value:
            return {}
        return dict(_DI_RE.findall(value.decode("latin-1")))

    def detect(self):
        """Lê o nome do modelo e escolhe os parâmetros. Devolve (texto, modelo|None)."""
        text = self.model_string()
        if not text:
            text = self.device_id().get("MDL", "")
        name = find_model(text)
        if name:
            self.set_model(name)
        return text, name

    # ---- EEPROM -----------------------------------------------------------

    def _need_keys(self, write=False):
        if not self.read_key:
            raise PrinterError("escolha o modelo (ou informe a chave de leitura)")
        if write and not self.write_key:
            raise PrinterError("falta a chave de gravação para este modelo")

    def read_eeprom(self, address):
        self._need_keys()
        value = self.snmp.get(read_oid(self.read_key, address))
        if not value or b":NA;" in value:
            raise PrinterError(
                "a impressora recusou a leitura do endereço %d "
                "(chave errada ou firmware bloqueado: tente pelo cabo USB, que "
                "costuma funcionar mesmo assim, ou use o reset temporário)" % address)
        match = _EE_RE.search(value)
        if not match:
            raise PrinterError("resposta inesperada: %r" % value)
        hexdata = match.group(1).decode()
        if int(hexdata[:4], 16) != address:
            raise PrinterError("a impressora respondeu outro endereço (%s)" % hexdata[:4])
        return int(hexdata[4:], 16)

    def read_many(self, addresses):
        return [self.read_eeprom(a) for a in addresses]

    def write_eeprom(self, address, value, verify=True):
        self._need_keys(write=True)
        if not 0 <= value <= 255:
            raise ValueError("o valor precisa estar entre 0 e 255")
        reply = self.snmp.get(write_oid(self.read_key, self.write_key, address, value))
        if not reply or b":OK;" not in reply:
            raise PrinterError(
                "a impressora recusou a gravação no endereço %d "
                "(chave errada ou firmware bloqueado para gravação)" % address)
        if verify:
            got = self.read_eeprom(address)
            if got != value:
                raise PrinterError("gravação no endereço %d não conferiu: lido %d, "
                                   "esperado %d" % (address, got, value))

    def dump(self, start, end):
        """Lê um intervalo [start, end] e devolve {endereço: valor}."""
        return {a: self.read_eeprom(a) for a in range(start, end + 1)}

    # ---- almofada de tinta -------------------------------------------------

    def waste_levels(self):
        """{'main': %, 'borderless': %} conforme o modelo."""
        cfg = self.config
        if not cfg:
            raise PrinterError("escolha o modelo primeiro")
        levels = {}
        for key, label in (("main_waste", "principal"), ("borderless_waste", "sem margem"),
                           ("third_waste", "terceira")):
            if key in cfg:
                values = self.read_many(cfg[key]["oids"])
                levels[label] = waste_percent(values, cfg[key]["divider"])
        return levels

    def reset_plan(self):
        """Endereços e valores que o reset vai gravar."""
        cfg = self.config
        if not cfg:
            raise PrinterError("escolha o modelo primeiro")
        if "raw_waste_reset" in cfg:
            return dict(cfg["raw_waste_reset"])
        plan = {}
        for key in ("main_waste", "borderless_waste", "third_waste"):
            for a in cfg.get(key, {}).get("oids", []):
                plan[a] = 0
        return plan

    def backup(self, addresses):
        """Lê os endereços que serão alterados, para poder desfazer."""
        return {
            "printer": self.host,
            "model": self.model,
            "date": datetime.datetime.now().isoformat(timespec="seconds"),
            "eeprom": {str(a): self.read_eeprom(a) for a in sorted(addresses)},
        }

    def reset_waste(self, backup_path=None):
        """Faz backup (se pedido) e zera os contadores. Devolve o backup."""
        plan = self.reset_plan()
        commit = self.config.get("waste_reset_commit", {})
        saved = self.backup(list(plan) + list(commit))
        if backup_path:
            with open(backup_path, "w", encoding="utf-8") as f:
                json.dump(saved, f, indent=2)
        for address, value in plan.items():
            self.write_eeprom(address, value)
        for address, mask in commit.items():
            self.write_eeprom(address, saved["eeprom"][str(address)] & mask)
        return saved

    def restore(self, backup):
        for address, value in backup["eeprom"].items():
            self.write_eeprom(int(address), int(value))

    # ---- regravar a EEPROM a partir de arquivo -------------------------------

    def protected_addresses(self):
        """Endereços que identificam esta unidade: série e MAC do Wi-Fi."""
        protected = set(self.config.get("serial_number", ()))
        protected.update(MAC_ADDRESSES)
        return protected

    def plan_image(self, image, include_protected=False):
        """Compara o arquivo com a impressora.

        Devolve (mudanças [(endereço, atual, novo)], protegidos_ignorados).
        """
        if len(image) > MAX_IMAGE:
            raise PrinterError("arquivo grande demais (%d endereços, máximo %d)"
                               % (len(image), MAX_IMAGE))
        protected = set() if include_protected else self.protected_addresses()
        changes, skipped = [], []
        for address in sorted(image):
            if address in protected:
                skipped.append(address)
                continue
            current = self.read_eeprom(address)
            if current != image[address]:
                changes.append((address, current, image[address]))
        return changes, skipped

    def write_image(self, changes, backup_path=None):
        """Grava as mudanças planejadas, salvando antes os valores atuais."""
        saved = {
            "printer": self.host,
            "model": self.model,
            "date": datetime.datetime.now().isoformat(timespec="seconds"),
            "eeprom": {str(a): old for a, old, _ in changes},
        }
        if backup_path:
            with open(backup_path, "w", encoding="utf-8") as f:
                json.dump(saved, f, indent=2)
        for address, _, new in changes:
            self.write_eeprom(address, new)
        return saved

    # ---- firmware bloqueado ------------------------------------------------

    def status(self):
        return parse_status(self.snmp.get(ctrl_oid("st", b"\x01")) or b"")

    def diagnose(self):
        """Estado, erro atual (com causa e correção) e avisos, pelo status ST2."""
        st = self.status()
        if not st:
            raise PrinterError("a impressora não devolveu o status")
        state = st.get(0x01, b"\xff")[0]
        result = {
            "estado": EPSON_STATUS.get(state, "desconhecido (0x%02X)" % state),
            "problemas": [],
            "avisos": [EPSON_WARNINGS.get(w, "aviso 0x%02X" % w)
                       for w in st.get(0x04, b"")],
        }
        if 0x02 in st:
            result["problemas"].append(explain_epson_error(st[0x02][0]))
        box = st.get(0x37, b"")
        if len(box) >= 2 and box[1] in (1, 2):
            result["avisos"].append("Caixa de manutenção %s"
                                    % ("quase cheia" if box[1] == 1 else "cheia"))
        return result

    def serial_number(self):
        """Número de série pelo status (funciona mesmo com a EEPROM bloqueada)."""
        status = self.status()
        for kind in (0x1F, 0x40):  # o campo varia por modelo (0x40 na L3250)
            serial = status.get(kind, b"").decode("ascii", "replace").strip("\x00 ")
            if len(serial) == 10 and serial.isalnum():
                return serial
        raise PrinterError("não consegui ler o número de série; digite o da etiqueta")

    def temporary_reset(self, serial=None):
        """Comando 'rw': libera a impressora sem chave nem acesso à EEPROM.

        Serve para firmwares que bloqueiam a gravação (como o da L3250).
        """
        serial = serial or self.serial_number()
        payload = struct.pack("<H", 1) + hashlib.sha1(serial.encode("ascii")).digest()
        reply = self.snmp.get(ctrl_oid("rw", payload))
        if not reply or b"rw:" not in reply or b":OK;" not in reply:
            raise PrinterError("a impressora recusou o reset temporário "
                               "(confira o número de série)")
        return serial
