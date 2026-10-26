"""Leitura completa da EEPROM e comparação entre duas leituras.

Uso típico, para descobrir o reset de um modelo que ainda não está na lista:
salve uma leitura antes, zere a impressora com outra ferramenta, salve outra
leitura e compare. Os endereços que mudaram são os do reset.
"""

import datetime
import json

from .printer import MAC_ADDRESSES, PrinterError

SNAPSHOT_TYPE = "ecoreset-leitura"
DEFAULT_RANGE = (0, 0x7FF)
WASTE_LEVEL = 94  # valor de "nível de manutenção" que os resets costumam gravar


def take(printer, start=DEFAULT_RANGE[0], end=DEFAULT_RANGE[1], label="", progress=None):
    """Lê [start, end] e devolve o dicionário da leitura (pronto para salvar em JSON).

    Endereços que a impressora recusa ficam em "sem_leitura". Se os primeiros
    16 forem todos recusados, a chave de leitura está errada e a leitura para.
    """
    if end < start:
        raise ValueError("intervalo inválido")
    cells, refused = {}, []
    for i, address in enumerate(range(start, end + 1)):
        try:
            cells[str(address)] = printer.read_eeprom(address)
        except PrinterError:
            refused.append(address)
            if i == 15 and not cells:
                raise PrinterError("a impressora recusou os 16 primeiros endereços: a "
                                   "chave de leitura está errada ou o acesso está "
                                   "bloqueado (tente pelo cabo USB)")
        if progress:
            progress(i + 1, end - start + 1)
    try:
        serial = printer.serial_number()
    except Exception:
        serial = ""
    return {
        "type": SNAPSHOT_TYPE,
        "label": label,
        "printer": printer.host,
        "model": printer.model,
        "model_name": _model_name(printer),
        "serial": serial,
        "read_key": list(printer.read_key or ()),
        "date": datetime.datetime.now().isoformat(timespec="seconds"),
        "range": [start, end],
        "eeprom": cells,
        "sem_leitura": refused,
    }


def _model_name(printer):
    try:
        return printer.model_string()
    except Exception:
        return ""


def save(snap, path):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(snap, f, indent=1)


def load(path):
    with open(path, encoding="utf-8") as f:
        snap = json.load(f)
    if "eeprom" not in snap:
        raise ValueError("%s não é uma leitura do EcoReset" % path)
    return snap


def compare(before, after):
    """Diferenças entre duas leituras.

    Devolve {"mudancas": [(endereço, antes, depois)], "avisos": [...]}.
    """
    warnings = []
    if before.get("serial") and after.get("serial") and before["serial"] != after["serial"]:
        warnings.append("As leituras são de impressoras diferentes (série %s x %s)."
                        % (before["serial"], after["serial"]))
    if before.get("model_name") and after.get("model_name") and \
            before["model_name"] != after["model_name"]:
        warnings.append("Modelos diferentes: %s x %s."
                        % (before["model_name"], after["model_name"]))
    a, b = before["eeprom"], after["eeprom"]
    common = sorted(set(a) & set(b), key=int)
    if len(common) < max(len(a), len(b)):
        warnings.append("%d endereço(s) só existem numa das leituras e foram ignorados."
                        % (max(len(a), len(b)) - len(common)))
    changes = [(int(k), a[k], b[k]) for k in common if a[k] != b[k]]
    return {"mudancas": changes, "avisos": warnings}


def classify(address, old, new):
    """Palpite sobre o papel de um endereço que mudou num reset."""
    if address in MAC_ADDRESSES:
        return "MAC (não deveria mudar)"
    if new == 0 and old != 0:
        return "provável contador zerado"
    if new == WASTE_LEVEL:
        return "provável nível de manutenção (94)"
    if new < old:
        return "diminuiu"
    return "aumentou (pode ser uso normal, não reset)"


def suggest_model(before, after, changes, serial_range=()):
    """Entrada de modelo sugerida a partir das mudanças de um reset.

    Inclui só o que tem cara de reset (zerou ou foi para 94), fora série/MAC.
    """
    skip = set(serial_range) | set(MAC_ADDRESSES)
    reset = {a: new for a, old, new in changes
             if a not in skip and (new == 0 or new == WASTE_LEVEL)}
    return {
        "modelo": after.get("model_name") or before.get("model_name") or "?",
        "read_key": after.get("read_key") or before.get("read_key"),
        "raw_waste_reset": {str(a): v for a, v in sorted(reset.items())},
        "ignorados": [a for a, _, _ in changes if a not in reset],
        "origem": "comparação de leituras do EcoReset (%s -> %s)"
                  % (before.get("date", "?"), after.get("date", "?")),
    }
