"""Banco de modelos aprendidos, num arquivo que várias máquinas podem dividir.

O banco é o arquivo modelos.json dentro da pasta do EcoReset. Para usar o
mesmo banco em vários computadores, aponte a pasta para uma pasta
sincronizada (Google Drive, OneDrive, Dropbox, iCloud, ou uma pasta de rede):
cada máquina lê o arquivo ao abrir e, ao salvar, junta com o que outra
máquina tiver gravado antes de escrever.

Onde fica a pasta, por ordem: variável ECORESET_PASTA; o que foi escolhido
com set_folder() (guardado em ~/.ecoreset.json); senão ~/EcoReset.
"""

import datetime
import json
import os
import platform

CONFIG = os.path.join(os.path.expanduser("~"), ".ecoreset.json")
DB_NAME = "modelos.json"
DB_TYPE = "ecoreset-modelos"


def folder():
    env = os.environ.get("ECORESET_PASTA")
    if env:
        return env
    try:
        with open(CONFIG, encoding="utf-8") as f:
            saved = json.load(f).get("pasta")
        if saved:
            return saved
    except (OSError, ValueError):
        pass
    return os.path.join(os.path.expanduser("~"), "EcoReset")


def set_folder(path):
    path = os.path.abspath(os.path.expanduser(path))
    os.makedirs(path, exist_ok=True)
    with open(CONFIG, "w", encoding="utf-8") as f:
        json.dump({"pasta": path}, f, ensure_ascii=False, indent=1)
    return path


def db_path():
    return os.path.join(folder(), DB_NAME)


# ---- conversão para JSON (chaves e endereços viram texto) -------------------

def _to_json(cfg):
    out = {}
    for key, value in cfg.items():
        if key == "write_key":
            value = value.decode("latin-1")
        elif key == "serial_number":
            value = [value.start, value.stop]
        elif key in ("raw_waste_reset", "waste_reset_commit"):
            value = {str(a): v for a, v in value.items()}
        elif isinstance(value, tuple):
            value = list(value)
        out[key] = value
    return out


def _from_json(data):
    cfg = dict(data)
    cfg["read_key"] = tuple(cfg["read_key"])
    cfg["write_key"] = cfg["write_key"].encode("latin-1")
    cfg.setdefault("aliases", [])
    if "serial_number" in cfg:
        cfg["serial_number"] = range(*cfg["serial_number"])
    for key in ("raw_waste_reset", "waste_reset_commit"):
        if key in cfg:
            cfg[key] = {int(a): int(v) for a, v in cfg[key].items()}
    return cfg


# ---- leitura e gravação -----------------------------------------------------

def load(path=None):
    """{nome: cfg} do banco; vazio se ainda não existe."""
    path = path or db_path()
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except FileNotFoundError:
        return {}
    except ValueError as e:
        raise ValueError("banco de modelos corrompido (%s): %s" % (path, e))
    return {name: _from_json(entry) for name, entry in data.get("modelos", {}).items()}


def _write(models, path):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    data = {"type": DB_TYPE, "modelos": {n: _to_json(c) for n, c in sorted(models.items())}}
    tmp = "%s.%s.tmp" % (path, os.getpid())
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=1)
    os.replace(tmp, path)  # troca de uma vez: quem lê nunca vê o arquivo pela metade


def save(name, cfg, path=None):
    """Grava (ou atualiza) um modelo no banco, juntando com o que já está lá."""
    path = path or db_path()
    models = load(path)
    cfg = dict(cfg)
    cfg["learned_at"] = datetime.datetime.now().isoformat(timespec="seconds")
    cfg["learned_on"] = platform.node()
    models[name] = cfg
    _write(models, path)
    return path


def remove(name, path=None):
    path = path or db_path()
    models = load(path)
    if models.pop(name, None) is None:
        return False
    _write(models, path)
    return True


def merge_file(other, path=None):
    """Junta outro banco (ex.: copiado de outra máquina) neste. Devolve os nomes novos."""
    models = load(path)
    incoming = load(other)
    new = [n for n in incoming if n not in models]
    for name, cfg in incoming.items():  # na dúvida, vale o aprendido mais recente
        if cfg.get("learned_at", "") >= models.get(name, {}).get("learned_at", ""):
            models[name] = cfg
    _write(models, path or db_path())
    return sorted(new)
