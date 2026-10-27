"""Aprende um modelo Epson a partir do log de um reset feito no WICReset.

O WICReset registra em texto (application.log) os comandos EPSON-CTRL que
mandou para a impressora, em hexadecimal ou como OID decimal. Um comando de
gravação da EEPROM é:

    "||" 10 00  rk0 rk1  42 BD 21  end_lo end_hi valor  chave(8 bytes, +1)

Dele saem a chave de leitura, a chave de gravação e o que o reset grava.
Só lemos o texto do log; nada da licença do WICReset é usado.
"""

import re

from .models import MODELS

WRITE_HEAD = b"||\x10\x00"
READ_HEAD = b"||\x07\x00"
LEVEL = 94          # "nível de manutenção" que os resets gravam
TRIAL_PERCENT = 80  # o reset trial deixa a almofada em 80%

_HEX_RUN = re.compile(r"(?:\b[0-9A-Fa-f]{2}\b[ :,-]?){12,}")
_DEC_RUN = re.compile(r"\b\d{1,3}(?:\.\d{1,3}){11,}\b")
_MODEL_RES = [
    re.compile(r"GUID:\s*(.+?)\s+\d+\s+KEY"),
    re.compile(r"(?:EPSON|Epson)\s+((?:L|ET-|XP-|WF-|EP-|PX-)[\w-]+)"),
]


class WicLogError(Exception):
    pass


def _byte_runs(text):
    """Toda sequência de bytes do texto, vinda de hexadecimal ou de OID decimal."""
    for line in text.splitlines():
        for m in _HEX_RUN.finditer(line):
            yield bytes(int(t, 16) for t in re.findall(r"[0-9A-Fa-f]{2}", m.group(0)))
        for m in _DEC_RUN.finditer(line):
            # partes acima de 255 (o 1248 do OID da Epson) não são bytes do comando
            yield bytes(p if p < 256 else 0 for p in map(int, m.group(0).split(".")))


def _unshift(key):
    return bytes(b - 1 if b else 0 for b in key)


def parse(text):
    """Devolve {"model", "read_key", "write_key", "writes": [(end, valor)], "reads"}."""
    read_key, write_key, writes, reads = None, None, [], set()
    for run in _byte_runs(text):
        pos = run.find(WRITE_HEAD)
        while pos >= 0 and pos + 20 <= len(run):
            cmd = run[pos + 4:pos + 20]
            if cmd[2:5] == b"\x42\xBD\x21":
                rk, wk = (cmd[0], cmd[1]), _unshift(cmd[8:16])
                if read_key and (rk, wk) != (read_key, write_key):
                    raise WicLogError("o log tem resets de mais de uma impressora; "
                                      "use um log com uma só")
                read_key, write_key = rk, wk
                writes.append((cmd[5] | cmd[6] << 8, cmd[7]))
            pos = run.find(WRITE_HEAD, pos + 1)
        pos = run.find(READ_HEAD)
        if pos >= 0 and pos + 11 <= len(run) and run[pos + 6:pos + 9] == b"\x41\xBE\xA0":
            read_key = read_key or (run[pos + 4], run[pos + 5])
            reads.add(run[pos + 9] | run[pos + 10] << 8)
    model = None
    for regex in _MODEL_RES:
        m = regex.search(text)
        if m:
            model = m.group(1).strip()
            break
    return {"model": model, "read_key": read_key, "write_key": write_key,
            "writes": writes, "reads": sorted(reads)}


def _counter_pairs(addresses):
    """Contadores de 2 bytes: endereços consecutivos tomados de dois em dois.

    [20, 21, 22, 23, 40] -> [[20, 21], [22, 23]]. No reset trial os dois bytes de
    cada contador vêm diferentes de zero, então os pares saem certos.
    """
    runs = []
    for a in sorted(addresses):
        if runs and a == runs[-1][-1] + 1:
            runs[-1].append(a)
        else:
            runs.append([a])
    return [run[i:i + 2] for run in runs for i in range(0, len(run) - 1, 2)]


def _similar_model(addresses, read_key, write_key):
    """Modelo da lista cujo reset grava os mesmos endereços: (nome, mesmas_chaves).

    Vários modelos dividem os endereços com divisores diferentes, então só um
    com as mesmas chaves é tomado como igual.
    """
    wanted = {a for a in addresses if a < 0x100}
    found = None
    for name, cfg in MODELS.items():
        if cfg.get("learned_from"):
            continue
        if {a for a in cfg.get("raw_waste_reset", {}) if a < 0x100} != wanted:
            continue
        if (cfg["read_key"], cfg["write_key"]) == (read_key, write_key):
            return name, True
        found = found or name
    return found, False


def _divider(oids, counters, percent):
    raw = 0
    for a in reversed(oids):
        raw = (raw << 8) | counters.get(a, 0)
    return round(raw / percent, 2)


def learn(text, name=None, percent=TRIAL_PERCENT):
    """Monta a configuração do modelo (no formato de models.MODELS).

    Num reset trial os contadores ficam em `percent`%: os bytes gravados com
    valor diferente de 0 e de 94 são contadores e, no reset completo, viram 0.
    Endereços a partir de 0x100 são flags de estado (variam por unidade) e não
    entram no reset; se o modelo for parecido com um da lista, vale o dele.
    Devolve (nome, cfg, avisos).
    """
    info = parse(text)
    if not info["writes"]:
        raise WicLogError("o log não tem nenhuma gravação de EEPROM: faça um reset "
                          "(nem que seja o trial) no WICReset e use o log de depois")
    name = (name or info["model"] or "").strip().upper()
    if not name:
        raise WicLogError("não achei o nome do modelo no log; informe o nome")
    written = dict(info["writes"])  # a última gravação de cada endereço vale
    reset, counters = {}, {}
    flags = sorted(a for a in written if a >= 0x100)
    for address, value in written.items():
        if address >= 0x100:
            continue  # flag de estado: o valor depende de cada unidade
        if value in (0, LEVEL):
            reset[address] = value
        else:
            reset[address] = 0
            counters[address] = value
    cfg = {
        "aliases": [],
        "read_key": tuple(info["read_key"]),
        "write_key": bytes(info["write_key"]),
        "raw_waste_reset": reset,
        "learned_from": "log do WICReset",
    }
    warnings = []
    names = ["main_waste", "borderless_waste", "third_waste"]
    similar, same_keys = _similar_model(reset, cfg["read_key"], cfg["write_key"])
    if similar:
        base = MODELS[similar]
        for key in ["serial_number", "waste_reset_commit"] + names:
            if key in base:
                cfg[key] = dict(base[key]) if isinstance(base[key], dict) else base[key]
        if same_keys:
            warnings.append("mesmas chaves e endereços do %s: contadores lidos como "
                            "nele" % similar)
        else:
            warnings.append("endereços iguais aos do %s, chaves diferentes" % similar)
            for key in names:  # divisor deste modelo, pelo valor do trial
                if key in cfg and any(a in counters for a in cfg[key]["oids"]):
                    cfg[key]["divider"] = _divider(cfg[key]["oids"], counters, percent)
    else:
        new_layout = max(reset, default=0) > 40  # modelos novos guardam a série em 1604
        cfg["serial_number"] = range(1604, 1614) if new_layout else range(192, 202)
        found = _counter_pairs(counters)[:3]
        for key, group in zip(names, found):
            cfg[key] = {"oids": group, "divider": _divider(group, counters, percent)}
        if not found:
            warnings.append("não deu para saber onde ficam os contadores; o reset "
                            "funciona, mas a leitura da almofada não")
    if counters and any(k in cfg for k in names) and not same_keys:
        warnings.append("divisores estimados pelo reset trial (%d%%): confira a "
                        "leitura da almofada" % percent)
    if flags and "waste_reset_commit" not in cfg:
        warnings.append("o WICReset também gravou %s (flags de estado), que ficaram de "
                        "fora; se o erro não sair, compare leituras antes/depois"
                        % ", ".join("0x%X" % a for a in flags))
    if not counters:
        warnings.append("o log é de um reset completo: será repetido igual")
    return name, cfg, warnings


def learn_file(path, name=None, percent=TRIAL_PERCENT):
    with open(path, encoding="utf-8", errors="replace") as f:
        return learn(f.read(), name=name, percent=percent)
