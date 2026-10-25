"""Canon PIXMA / MegaTank: diagnóstico pelo IPP e manutenção por comandos BJL.

O que existe de forma pública para a Canon:
- estado e "support codes" pelo IPP (rede ou IPP-over-USB);
- limpeza de cabeça e teste de bicos pelos comandos BJL, que vão como um
  trabalho de impressão bruto (os mesmos do filtro commandtocanon do gutenprint).

O contador da almofada (erro 5B00) só é zerado pela ferramenta de serviço da
Canon, que é proprietária. Modelos com caixa de manutenção (G2060, G3160...)
não precisam de reset: a caixa é trocável (códigos 1726/1727).
"""

from . import ipp
from .errors import explain_canon

BJL_START = b"\x1b[K\x02\x00\x00\x1fBJLSTART\n"
BJL_END = b"BJLEND\n"


def bjl_job(*commands):
    return BJL_START + b"".join(c.encode("ascii") + b"\n" for c in commands) + BJL_END


def head_cleaning_job(black_only=False):
    return bjl_job("@Cleaning=1K" if black_only else "@Cleaning=1ALL")


def nozzle_check_job():
    return bjl_job("@TestPrint=NozzleCheck")


def command_sets(attrs):
    """Linguagens que a impressora declara no Device ID (campo CMD)."""
    device_id = (attrs.get("printer-device-id") or [""])[0]
    for part in device_id.split(";"):
        if part.startswith(("CMD:", "COMMAND SET:")):
            return [c.strip() for c in part.split(":", 1)[1].split(",")]
    return []


def check_bjl(url):
    """Erro claro se o modelo não aceita BJL (os mais novos só falam IVEC)."""
    sets = command_sets(ipp.get_printer_attributes(url))
    if sets and "BJL" not in sets:
        raise ipp.IppError(
            "este modelo não aceita comandos de manutenção BJL (declara %s). Use os "
            "botões da impressora ou o utilitário da Canon (IJ Printer Assistant)."
            % ", ".join(sets))


def diagnose(url):
    """Estado da impressora e explicação de cada código/motivo encontrado."""
    attrs = ipp.get_printer_attributes(url)
    return diagnose_attrs(attrs)


def diagnose_attrs(attrs):
    states = {3: "Pronta", 4: "Imprimindo", 5: "Parada"}
    state = (attrs.get("printer-state") or [0])[0]
    result = {
        "modelo": (attrs.get("printer-make-and-model") or [""])[0],
        "estado": states.get(state, "desconhecido"),
        "mensagem": (attrs.get("printer-state-message") or [""])[0],
        "motivos": [text for key, text in ipp.reasons(attrs) if key != "none"],
        "problemas": [explain_canon(c) for c in ipp.support_codes(attrs)],
        "tintas": [(n, LEVELS.get(v, "%d%%" % v) if isinstance(v, int) else v)
                   for n, v in zip(attrs.get("marker-names", []),
                                   attrs.get("marker-levels", []))],
    }
    if state == 5 and not result["problemas"]:
        result["mensagem"] = result["mensagem"] or (
            "A impressora está parada com erro, mas não informou o código pelo IPP. "
            "Veja o código no visor ou conte as piscadas da luz de alarme e "
            "consulte-o no campo 'Consultar código'.")
    return result


# Níveis especiais do IPP (RFC 8011): -2 desconhecido, -3 há algum restante.
LEVELS = {-1: "n/d", -2: "sem sensor", -3: "tem tinta"}
