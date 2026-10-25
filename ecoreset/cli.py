"""Linha de comando: python -m ecoreset <comando> <ip|usb> ..."""

import argparse
import datetime
import sys

from . import maintenance
from .models import model_names
from .printer import EpsonPrinter, PrinterError
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
    args = ap.parse_args(argv)

    try:
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
    except (PrinterError, SnmpError, UsbError, OSError, ValueError) as e:
        print("Erro: %s" % e, file=sys.stderr)
        return 2


def _run(args, p):
    if args.cmd == "levels":
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
