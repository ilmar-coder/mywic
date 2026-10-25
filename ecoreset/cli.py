"""Linha de comando: python -m ecoreset <comando> <ip|usb> ..."""

import argparse
import datetime
import re
import sys

from . import canon, ipp, maintenance
from .errors import explain_epson_error, search
from .models import model_names
from .printer import EpsonPrinter, PrinterError, load_image
from .snmp import SnmpError
from .usb import UsbCtrlClient, UsbError


def _connect(args):
    """'usb' no lugar do IP usa o cabo (D4); senão, SNMP pela rede."""
    if args.host.lower() == "usb":
        return EpsonPrinter("USB", client=UsbCtrlClient().open())
    return EpsonPrinter(args.host, timeout=args.timeout)


def _printer(args):
    p = _connect(args)
    if args.model:
        p.set_model(args.model)
    else:
        text, name = p.detect()
        print("Impressora: %s" % (text or "?"))
        if not name:
            raise PrinterError("modelo não suportado; use --model (%s)"
                               % ", ".join(model_names()))
    print("Modelo: %s" % p.model)
    return p


def main(argv=None):
    ap = argparse.ArgumentParser(prog="ecoreset", description=__doc__)
    ap.add_argument("--model", choices=model_names())
    ap.add_argument("--timeout", type=float, default=2.0)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("detect", "levels", "reset", "nozzle", "clean"):
        sub.add_parser(name).add_argument("host")
    tr = sub.add_parser("temp-reset", help="reset temporário (firmware bloqueado)")
    tr.add_argument("host")
    tr.add_argument("--serial", help="número de série (lido da impressora se omitido)")
    rd = sub.add_parser("read")
    rd.add_argument("host")
    rd.add_argument("start", type=int)
    rd.add_argument("end", type=int, nargs="?")
    wr = sub.add_parser("write")
    wr.add_argument("host")
    wr.add_argument("address", type=int)
    wr.add_argument("value", type=int)
    sub.choices["reset"].add_argument("--yes", action="store_true",
                                      help="não pedir confirmação")
    sub.add_parser("diagnose", help="erros da Epson, com causa e correção").add_argument(
        "host")
    cs = sub.add_parser("canon-status", help="erros da Canon (IP ou fila do sistema)")
    cs.add_argument("where", help="IP ou nome da fila (lpstat -e)")
    sub.add_parser("code", help="explica um código de erro").add_argument("code")
    wi = sub.add_parser("write-image", help="regrava a EEPROM a partir de um arquivo")
    wi.add_argument("host")
    wi.add_argument("file", help=".json do EcoReset ou dump binário")
    wi.add_argument("--start", type=lambda s: int(s, 0), default=0,
                    help="endereço do primeiro byte do binário")
    wi.add_argument("--include-protected", action="store_true",
                    help="grava também série/MAC (só para backup da mesma impressora)")
    wi.add_argument("--yes", action="store_true")
    args = ap.parse_args(argv)

    if args.cmd == "code":
        found = ([explain_epson_error(int(args.code, 16))]
                 if args.code.lower().startswith("0x") else search(args.code))
        for p in found:
            _print_problem(p)
        return 0 if found else 1

    try:
        if args.cmd == "canon-status":
            where = args.where
            url = (ipp.find_queue_url(where) if not re.match(r"^[\d.]+$", where)
                   else "ipp://%s:631/ipp/print" % where)
            d = canon.diagnose(url)
            print("Modelo: %s\nEstado: %s" % (d["modelo"], d["estado"]))
            for line in [d["mensagem"]] + d["motivos"]:
                if line:
                    print(line)
            for n, v in d["tintas"]:
                print("Nível %s: %s" % (n, v))
            for p in d["problemas"]:
                _print_problem(p)
            return 0
        if args.cmd == "nozzle":
            maintenance.send_raw_network(args.host, maintenance.nozzle_check_job())
            print("Teste de bicos enviado.")
            return 0
        if args.cmd == "clean":
            maintenance.send_raw_network(args.host, maintenance.head_cleaning_job())
            print("Limpeza de cabeça enviada.")
            return 0
        if args.cmd == "temp-reset":
            p = _connect(args)
            try:
                serial = p.temporary_reset(args.serial)
            finally:
                getattr(p.snmp, "close", lambda: None)()
            print("Impressora %s liberada. Desligue e ligue de novo." % serial)
            return 0

        p = _printer(args)
        try:
            return _run(args, p)
        finally:
            getattr(p.snmp, "close", lambda: None)()
    except (PrinterError, SnmpError, UsbError, ipp.IppError, OSError, ValueError) as e:
        print("Erro: %s" % e, file=sys.stderr)
        return 2


def _print_problem(p):
    print("\n%s %s: %s" % (p.brand, p.code, p.title))
    print("  Causa: %s" % p.cause)
    print("  Correção: %s" % p.fix)
    if p.service:
        print("  Pode exigir assistência técnica.")


def _run(args, p):
    if args.cmd == "diagnose":
        d = p.diagnose()
        print("Estado: %s" % d["estado"])
        for a in d["avisos"]:
            print("Aviso: %s" % a)
        for problem in d["problemas"]:
            _print_problem(problem)
        if not d["problemas"]:
            print("Nenhum erro.")
    elif args.cmd == "write-image":
        image, file_model = load_image(args.file, args.start)
        if file_model and file_model != p.model:
            print("Atenção: o arquivo é de %s e a impressora é %s." % (file_model, p.model))
        changes, skipped = p.plan_image(image, include_protected=args.include_protected)
        print("%d endereço(s) a gravar, %d protegido(s) ignorado(s)."
              % (len(changes), len(skipped)))
        for a, old, new in changes:
            print("%5d (0x%04X): %3d -> %3d" % (a, a, old, new))
        if not changes:
            return 0
        if not args.yes and input("Gravar? [s/N] ").lower() != "s":
            return 1
        path = "backup-antes-regravar-%s.json" % datetime.datetime.now().strftime(
            "%Y%m%d-%H%M%S")
        p.write_image(changes, backup_path=path)
        print("Regravado e conferido. Backup em %s." % path)
    elif args.cmd == "levels":
        for label, pct in p.waste_levels().items():
            print("Almofada %s: %.2f%%" % (label, pct))
    elif args.cmd == "reset":
        if not args.yes and input("Zerar os contadores? [s/N] ").lower() != "s":
            return 1
        path = "backup-%s-%s.json" % (
            p.model, datetime.datetime.now().strftime("%Y%m%d-%H%M%S"))
        p.reset_waste(backup_path=path)
        print("Contadores zerados. Backup em %s. Desligue e ligue a impressora."
              % path)
    elif args.cmd == "read":
        for a, v in p.dump(args.start, args.end or args.start).items():
            print("%5d (0x%04X): %3d (0x%02X)" % (a, a, v, v))
    elif args.cmd == "write":
        p.write_eeprom(args.address, args.value)
        print("Gravado.")
    return 0
