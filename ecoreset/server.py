"""Banco de modelos no servidor (o site no Raspberry): Atualizar e Enviar.

O servidor guarda um modelo por nome:
    GET    {url}/api/ecoreset/models          -> {"modelos": {nome: modelo}}
    PUT    {url}/api/ecoreset/models/{nome}   <- modelo
    DELETE {url}/api/ecoreset/models/{nome}
com a chave no cabeçalho X-EcoReset-Token.

O banco local (library.py) é a cópia do servidor, para funcionar sem
internet. O que foi aprendido sem conexão fica "pendente" e sobe no próximo
Atualizar.
"""

import json
import ssl
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request

from . import __version__, library

DEFAULT_URL = "https://download.ilmar.com.br"
TIMEOUT = 15


class ServerError(Exception):
    pass


_SSL = None
_MAC_ROOTS = "/System/Library/Keychains/SystemRootCertificates.keychain"


def ssl_context():
    """Contexto TLS que confere o certificado do servidor.

    O Python empacotado no .app do Mac não acha os certificados do sistema;
    nesse caso usamos os certificados raiz do próprio macOS (Keychain).
    """
    global _SSL
    if _SSL is None:
        ctx = ssl.create_default_context()
        if sys.platform == "darwin" and not ctx.cert_store_stats()["x509_ca"]:
            try:
                pem = subprocess.run(
                    ["/usr/bin/security", "find-certificate", "-a", "-p", _MAC_ROOTS],
                    capture_output=True, text=True, timeout=20, check=True).stdout
                ctx.load_verify_locations(cadata=pem)
            except (OSError, subprocess.SubprocessError, ssl.SSLError):
                pass  # sem os certificados, a conexão falha com o erro do TLS
        _SSL = ctx
    return _SSL


def settings():
    """(url, chave) do servidor; chave vazia = servidor não configurado."""
    cfg = library.read_config()
    return (cfg.get("servidor") or DEFAULT_URL).rstrip("/"), cfg.get("token") or ""


def configured():
    return bool(settings()[1])


def configure(url, token):
    url = (url or DEFAULT_URL).strip().rstrip("/")
    if not url.startswith(("https://", "http://localhost", "http://127.0.0.1")):
        raise ServerError("use um endereço https:// (a chave não pode ir sem criptografia)")
    library.write_config(servidor=url, token=token.strip())


def _request(method, path, body=None):
    url, token = settings()
    if not token:
        raise ServerError("servidor não configurado: informe o endereço e a chave")
    data = json.dumps(body).encode("utf-8") if body is not None else None
    req = urllib.request.Request(url + path, data=data, method=method, headers={
        "X-EcoReset-Token": token,
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "EcoReset/%s" % __version__,
    })
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT, context=ssl_context()) as resp:
            return json.loads(resp.read().decode("utf-8") or "{}")
    except urllib.error.HTTPError as e:
        try:
            message = json.loads(e.read().decode("utf-8")).get("error", "")
        except ValueError:
            message = ""
        if e.code == 404 and method == "DELETE":
            return {}
        raise ServerError("servidor respondeu %d %s" % (e.code, message or e.reason))
    except (urllib.error.URLError, OSError) as e:
        raise ServerError("sem conexão com o servidor (%s)" % getattr(e, "reason", e))
    except ValueError:
        raise ServerError("resposta inválida do servidor")


def _path(name):
    return "/api/ecoreset/models/" + urllib.parse.quote(name, safe="")


def fetch():
    return _request("GET", "/api/ecoreset/models").get("modelos") or {}


def push(name, cfg):
    _request("PUT", _path(name), library.to_json(cfg))


def publish(name, cfg):
    """Salva no banco local e envia ao servidor. Devolve None ou o motivo de não ter ido."""
    library.save(name, cfg, pending=configured())
    if not configured():
        return None
    try:
        push(name, cfg)
    except ServerError as e:
        return "%s; vai no próximo Atualizar" % e
    library.mark_sent(name)
    return None


def remove(name):
    if configured():
        _request("DELETE", _path(name))
    return library.remove(name)


def sync():
    """Envia os pendentes e baixa o banco do servidor. Devolve (enviados, total, novos)."""
    local = library.load()
    pending = {n: c for n, c in local.items() if c.get("pendente")}
    for name, cfg in pending.items():
        push(name, cfg)
    remote = fetch()
    new = sorted(n for n in remote if n not in local)
    library.replace_all(remote)
    return len(pending), len(remote), new
