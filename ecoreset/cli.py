"""Linha de comando: python -m ecoreset <comando> <ip|usb> ..."""

import argparse
import datetime
import json
import re
import sys

from . import canon, ipp, keyfinder, library, maintenance, snapshot, wiclog
from .errors import explain_epson_error, search
from .ivec import MAINTENANCE, CanonUsb, IvecError
from .models import MODELS, model_names, reload_learned
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
        key = getattr(args, "key", None)
        if not name and key:  # modelo fora da lista: só leitura, com a chave dada
            p.read_key = tuple(int(k, 0) for k in key.split(","))
        elif not name:
            raise PrinterError("modelo não suportado; use --model (%s)"
                               % ", ".join(model_names()))
    print("Modelo: %s" % (p.model or "fora da lista (chave %s)" % (p.read_key,)))
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
    cs.add_argument("where", help="usb (IVEC, código exato), IP ou nome da fila")
    cm = sub.add_parser("canon-maint", help="manutenção da Canon pelo USB (IVEC)")
    cm.add_argument("operation", choices=list(MAINTENANCE))
    sub.add_parser("code", help="explica um código de erro").add_argument("code")
    sn = sub.add_parser("snapshot", help="leitura completa da EEPROM salva em arquivo")
    sn.add_argument("host")
    sn.add_argument("-o", "--output", required=True, help="arquivo .json de saída")
    sn.add_argument("--end", type=lambda s: int(s, 0), default=snapshot.DEFAULT_RANGE[1])
    sn.add_argument("--label", default="")
    sn.add_argument("--key", help="chave de leitura para modelo fora da lista, ex.: 74,54")
    fk = sub.add_parser("find-key", help="descobre a chave de leitura (só lê)")
    fk.add_argument("host")
    fk.add_argument("--write", action="store_true",
                    help="testa também as chaves de gravação conhecidas (grava o mesmo "
                         "valor de volta, não muda nada)")
    cp = sub.add_parser("compare", help="compara duas leituras (antes e depois)")
    cp.add_argument("before")
    cp.add_argument("after")
    cp.add_argument("--save", help="salva a sugestão de modelo neste .json")
    wi = sub.add_parser("write-image", help="regrava a EEPROM a partir de um arquivo")
    wi.add_argument("host")
    wi.add_argument("file", help=".json do EcoReset ou dump binário")
    wi.add_argument("--start", type=lambda s: int(s, 0), default=0,
                    help="endereço do primeiro byte do binário")
    wi.add_argument("--include-protected", action="store_true",
                    help="grava também série/MAC (só para backup da mesma impressora)")
    wi.add_argument("--yes", action="store_true")
    wl = sub.add_parser("wic-log", help="aprende um modelo pelo log de um reset do WICReset")
    wl.add_argument("log", help="application.log do WICReset")
    wl.add_argument("--name", help="nome do modelo (se o log não disser)")
    wl.add_argument("--percent", type=float, default=wiclog.TRIAL_PERCENT,
                    help="porcentagem em que o reset do log deixou a almofada (trial: 80)")
    wl.add_argument("--save", action="store_true", help="grava no banco de modelos")
    lb = sub.add_parser("library", help="banco de modelos aprendidos (pasta compartilhada)")
    lb.add_argument("--folder", help="usa esta pasta (ex.: uma pasta do Google Drive)")
    lb.add_argument("--merge", help="junta outro modelos.json neste")
    lb.add_argument("--remove", help="apaga um modelo aprendido")
    args = ap.parse_args(argv)

    if args.cmd in ("wic-log", "library"):
        try:
            return _wic_log(args) if args.cmd == "wic-log" else _library(args)
        except (wiclog.WicLogError, OSError, ValueError) as e:
            print("Erro: %s" % e, file=sys.stderr)
            return 2

    if args.cmd == "compare":
        return _compare(args)

    if args.cmd == "code":
        found = ([explain_epson_error(int(args.code, 16))]
                 if args.code.lower().startswith("0x") else search(args.code))
        for p in found:
            _print_problem(p)
        return 0 if found else 1

    try:
        if args.cmd == "canon-maint":
            with CanonUsb() as c:
                c.maintenance(args.operation)
            print("%s enviado para a Canon." % args.operation)
            return 0
        if args.cmd == "canon-status":
            where = args.where
            if where.lower() == "usb":
                with CanonUsb() as c:
                    d = c.diagnose()
            else:
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

        if args.cmd == "find-key":
            return _find_key(args)

        p = _printer(args)
        try:
            return _run(args, p)
        finally:
            getattr(p.snmp, "close", lambda: None)()
    except (PrinterError, SnmpError, UsbError, ipp.IppError, IvecError, OSError,
            ValueError) as e:
        print("Erro: %s" % e, file=sys.stderr)
        return 2


def _find_key(args):
    p = _connect(args)
    try:
        text, name = p.detect()
        print("Impressora: %s" % (text or "?"))
        if name:
            print("Este modelo já está na lista (%s), chave %s." % (name, p.read_key))
            return 0

        def progress(i, n, key):
            if i % 1024 == 0:
                print("  %d/%d (testando %d,%d)" % (i, n, key[0], key[1]), file=sys.stderr)
        try:
            result = keyfinder.find_read_key(p, progress=progress)
        except KeyboardInterrupt:
            print("Busca interrompida.")
            return 1
        key = result["read_key"]
        print("Chave de leitura: %d,%d (%d tentativas)" % (key[0], key[1],
                                                         result["tentativas"]))
        if result["aceita_qualquer"]:
            print("Atenção: a impressora também aceitou outra chave.")
        if args.write:
            wk = keyfinder.find_write_key(p)
            print("Chave de gravação: %s" % (wk.decode() if wk else
                                             "nenhuma das conhecidas foi aceita"))
        return 0
    finally:
        getattr(p.snmp, "close", lambda: None)()


def _compare(args):
    before, after = snapshot.load(args.before), snapshot.load(args.after)
    result = snapshot.compare(before, after)
    for w in result["avisos"]:
        print("Atenção: %s" % w)
    changes = result["mudancas"]
    print("%d endereço(s) mudaram:" % len(changes))
    for a, old, new in changes:
        print("%5d (0x%04X): %3d -> %3d   %s" % (a, a, old, new,
                                             snapshot.classify(a, old, new)))
    if changes:
        model = MODELS.get(after.get("model") or "", {})
        suggestion = snapshot.suggest_model(before, after, changes,
                                            model.get("serial_number", ()))
        print("\nReset sugerido: %s" % suggestion["raw_waste_reset"])
        if args.save:
            with open(args.save, "w", encoding="utf-8") as f:
                json.dump(suggestion, f, indent=2, ensure_ascii=False)
            print("Sugestão salva em %s" % args.save)
    return 0


def _describe(name, cfg):
    print("Modelo: %s" % name)
    print("  Chave de leitura: %d,%d" % cfg["read_key"])
    print("  Chave de gravação: %s" % cfg["write_key"].decode("latin-1"))
    print("  Reset grava: %s" % cfg["raw_waste_reset"])
    for key, label in (("main_waste", "principal"), ("borderless_waste", "sem margem"),
                       ("third_waste", "terceira")):
        if key in cfg:
            print("  Contador %s: endereços %s, divisor %s"
                  % (label, cfg[key]["oids"], cfg[key]["divider"]))


def _wic_log(args):
    name, cfg, warnings = wiclog.learn_file(args.log, name=args.name, percent=args.percent)
    _describe(name, cfg)
    for w in warnings:
        print("Atenção: %s" % w)
    if args.save:
        print("Salvo no banco: %s" % library.save(name, cfg))
    else:
        print("(use --save para gravar no banco)")
    return 0


def _library(args):
    if args.folder:
        print("Pasta do banco: %s" % library.set_folder(args.folder))
    if args.merge:
        new = library.merge_file(args.merge)
        print("Juntado. Novos: %s" % (", ".join(new) or "nenhum"))
    if args.remove:
        print("Apagado." if library.remove(args.remove) else "Não está no banco.")
    learned = reload_learned()
    print("Banco: %s" % library.db_path())
    for name in learned:
        cfg = MODELS[name]
        print("  %-12s chave %d,%d  (%s, %s)" % (
            name, cfg["read_key"][0], cfg["read_key"][1],
            cfg.get("learned_on", "?"), cfg.get("learned_at", "?")))
    if not learned:
        print("  nenhum modelo aprendido ainda")
    return 0


def _print_problem(p):
    print("\n%s %s: %s" % (p.brand, p.code, p.title))
    print("  Causa: %s" % p.cause)
    print("  Correção: %s" % p.fix)
    if p.service:
        print("  Pode exigir assistência técnica.")


def _run(args, p):
    if args.cmd == "snapshot":
        def progress(i, n):
            if i % 128 == 0 or i == n:
                print("  %d/%d" % (i, n), file=sys.stderr)
        snap = snapshot.take(p, 0, args.end, label=args.label, progress=progress)
        snapshot.save(snap, args.output)
        print("Leitura salva em %s: %d endereços lidos, %d recusados."
              % (args.output, len(snap["eeprom"]), len(snap["sem_leitura"])))
    elif args.cmd == "diagnose":
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
