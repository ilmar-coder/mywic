"""Descobrir as chaves de uma Epson que não está na lista de modelos.

Chave de leitura: são 2 bytes (65.536 combinações). Testamos lendo um endereço
da EEPROM, que é inofensivo: a chave errada só recebe "NA". Primeiro vão as
chaves dos modelos conhecidos, depois as combinações com o segundo byte baixo
(onde caem quase todas as chaves conhecidas) e, por fim, o resto.

Chave de gravação: são 8 bytes, então não dá para testar todas. Testamos só as
conhecidas, gravando no endereço o MESMO valor que já está lá (não muda nada).
"""

from .models import MODELS
from .printer import PrinterError

TOTAL_KEYS = 256 * 256


def known_read_keys():
    keys = []
    for cfg in MODELS.values():
        if tuple(cfg["read_key"]) not in keys:
            keys.append(tuple(cfg["read_key"]))
    return keys


def known_write_keys():
    keys = []
    for cfg in MODELS.values():
        if cfg["write_key"] not in keys:
            keys.append(cfg["write_key"])
    return keys


def candidate_keys():
    """Todas as 65.536 chaves, as mais prováveis primeiro, sem repetir."""
    known = known_read_keys()
    yield from known
    seen = set(known)
    for second in list(range(16)) + list(range(16, 256)):
        for first in range(256):
            if (first, second) not in seen:
                yield (first, second)


def _accepts(printer, key, address):
    printer.read_key = key
    try:
        printer.read_eeprom(address)
        return True
    except PrinterError:
        return False


def find_read_key(printer, address=0, progress=None, cancel=None):
    """Procura a chave de leitura. Devolve dict com a chave ou None se cancelado.

    progress(tentativas, total, chave) é chamado a cada tentativa. Erros de
    comunicação (impressora sumiu) interrompem a busca.
    """
    original, found = printer.read_key, None
    try:
        for tries, key in enumerate(candidate_keys(), 1):
            if cancel is not None and cancel.is_set():
                return None
            if progress:
                progress(tries, TOTAL_KEYS, key)
            if not _accepts(printer, key, address):
                continue
            # confere com outro endereço e com uma chave certamente diferente
            confirmed = _accepts(printer, key, address + 1)
            wrong = ((key[0] + 1) % 256, key[1])
            any_key = _accepts(printer, wrong, address)
            printer.read_key = found = key
            return {"read_key": key, "tentativas": tries, "confirmada": confirmed,
                    "aceita_qualquer": any_key, "conhecida": key in known_read_keys()}
        raise PrinterError("nenhuma chave funcionou. Se for pela rede, o firmware pode "
                           "estar bloqueado: tente pelo cabo USB.")
    finally:
        if found is None:  # cancelado ou sem sucesso: volta como estava
            printer.read_key = original


def find_write_key(printer, address=0):
    """Testa as chaves de gravação conhecidas gravando o valor atual de volta.

    Devolve a chave aceita ou None. Exige a chave de leitura já definida.
    """
    if not printer.read_key:
        raise PrinterError("descubra a chave de leitura primeiro")
    value = printer.read_eeprom(address)
    for key in known_write_keys():
        printer.write_key = key
        try:
            printer.write_eeprom(address, value)  # mesmo valor: não muda nada
            return key
        except PrinterError:
            continue
    printer.write_key = None
    return None
