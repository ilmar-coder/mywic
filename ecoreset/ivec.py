"""Canon IVEC pelo USB: status com o código de erro e manutenção (limpeza etc.).

IVEC é o protocolo XML das Canon PIXMA/MegaTank desde ~2013 (aparece como
"IVEC" no campo CMD do Device ID). Mensagens montadas a partir da descrição
pública do protocolo do projeto canon-universal-driver (docs/PROTOCOL_IVEC.md).

O status vem em resposta a GetStatus: <ivec:status>, <ivec:status_detail> e
<ivec:current_support_code> (o mesmo código do manual, ex. 1300).
"""

import datetime
import re
import sys
import time
import uuid

from . import winusb
from .errors import explain_canon
from .usb import UsbDevice, UsbError, find_devices, interface_list

CANON_VENDOR = 0x04A9

XML_DECL = '<?xml version="1.0" encoding="utf-8" ?>'
XML_DECL2 = '<?xml version="1.0" encoding="utf-8"?>'
NS_IVEC = 'xmlns:ivec="http://www.canon.com/ns/cmd/2008/07/common/"'
NS_VCN = 'xmlns:vcn="http://www.canon.com/ns/cmd/2008/07/canon/"'
CLOSE = "</ivec:param_set></ivec:contents></cmd>"
JOB_ID = "00000002"

GET_STATUS = (XML_DECL + "<cmd " + NS_IVEC + "><ivec:contents><ivec:operation>GetStatus"
              "</ivec:operation><ivec:param_set servicetype=\"print\"></ivec:param_set>"
              "</ivec:contents></cmd>").encode()

# Operações de manutenção: nome -> (operação, tipo, grupo de tinta)
MAINTENANCE = {
    "limpeza": ("Cleaning", "regular", "all"),
    "limpeza-preto": ("Cleaning", "regular", "group1"),
    "limpeza-cor": ("Cleaning", "regular", "group2"),
    "limpeza-profunda": ("Cleaning", "deep", "all"),
    "ink-flush": ("Cleaning", "choke", "all"),
    "rolete": ("RollerCleaning", "roller", ""),
    "placa-inferior": ("RollerCleaning", "platen", ""),
    "teste-bicos": ("TestPrint", "nozzle_check", ""),
}

STATUS_TEXT = {
    "idle": "Pronta", "processing": "Trabalhando", "stopped": "Parada (erro)",
    "notready": "Não está pronta", "busying": "Ocupada", "canceling": "Cancelando",
    "waitingprint": "Aguardando impressão", "printing": "Imprimindo", "held": "Retida",
}
DETAIL_TEXT = {
    "MediaEmpty": "Sem papel", "MediaJam": "Papel enroscado", "DoorOpen": "Tampa aberta",
    "MarkerSupplyAttention": "Atenção à tinta", "DeviceBusy": "Ocupada",
    "AttentionRequired": "Precisa de atenção",
}


class IvecError(Exception):
    pass


def _head(operation, service, vendor_ns, decl=XML_DECL):
    return (decl + "<cmd " + NS_IVEC + (" " + NS_VCN if vendor_ns else "") + ">"
            "<ivec:contents><ivec:operation>%s</ivec:operation>"
            "<ivec:param_set servicetype=\"%s\">" % (operation, service))


def maintenance_job(name, now=None, datetime_cap=True, host_env=True):
    """Bytes de um trabalho de manutenção IVEC completo."""
    if name not in MAINTENANCE:
        raise IvecError("operação desconhecida: %s" % name)
    operation, kind, group = MAINTENANCE[name]
    now = now or datetime.datetime.now()
    parts = [
        _head("StartJob", "maintenance", host_env) + "<ivec:jobID>%s</ivec:jobID>" % JOB_ID
        + "<ivec:bidi>0</ivec:bidi><ivec:jobname/><ivec:username/><ivec:computername/>"
        "<ivec:job_description><![CDATA[%s]]></ivec:job_description>" % uuid.uuid4()
        + ("<vcn:host_environment>linux</vcn:host_environment>" if host_env else "") + CLOSE,
    ]
    if datetime_cap:
        parts.append(_head("SetJobConfiguration", "maintenance", True)
                     + "<ivec:jobID>%s</ivec:jobID><ivec:datetime>%s</ivec:datetime>"
                     % (JOB_ID, now.strftime("%Y%m%d%H%M%S")) + CLOSE)
    # aqui os parâmetros da operação vêm antes do jobID
    parts.append(_head(operation, "maintenance", False, XML_DECL2)
                 + ("<ivec:inkgroup>%s</ivec:inkgroup>" % group if group else "")
                 + "<ivec:type>%s</ivec:type>" % kind
                 + "<ivec:jobID>%s</ivec:jobID>" % JOB_ID + CLOSE)
    parts.append(_head("EndJob", "maintenance", False)
                 + "<ivec:jobID>%s</ivec:jobID>" % JOB_ID + CLOSE)
    return "".join(parts).encode()


def _last(xml, tag):
    found = re.findall(r"<ivec:%s>([^<]*)</ivec:%s>" % (tag, tag), xml)
    return found[-1].strip() if found else ""


def parse_status(data):
    """GetStatusResponse -> {'status', 'detail', 'support_code'} (o último lido vale)."""
    xml = data.decode("utf-8", "replace")
    status = _last(xml, "status")
    if not status:
        return None
    code = _last(xml, "current_support_code") or _last(xml, "error_code")
    waste = re.search(r"<ivec:wasteink>(.*?)</ivec:wasteink>", xml, re.S)
    boxes = []
    for item in re.findall(r"<ivec:item>(.*?)</ivec:item>", waste.group(1) if waste else "",
                           re.S):
        level = _last(item, "level")
        boxes.append((_last(item, "model") or "caixa", int(level) if level.lstrip("-").isdigit()
                      else None, _last(item, "icon")))
    errors = re.findall(r"<ivec:error_code>([^<]+)</ivec:error_code>", xml)
    return {"status": status, "detail": _last(xml, "status_detail"),
            "support_code": "" if code in ("", "0") else code.upper(),
            "error_codes": [e.upper() for e in errors if e not in ("0", code)],
            "maintenance_box": boxes}


class CanonUsb:
    """Canon IVEC ligada no USB, pela interface de impressora bidirecional."""

    def __init__(self):
        self.lib = self.ctx = self.devices = None
        self.device = None

    def open(self):
        if sys.platform == "win32":  # usbprint.sys, sem libusb
            for path in winusb.device_paths():
                try:
                    device = winusb.UsbPrintDevice(path)
                    info = device.device_id()
                except OSError:
                    continue
                if info.get("MFG", "").upper() == "CANON" and "IVEC" in info.get("CMD", ""):
                    device.product = info.get("MDL", "Canon")
                    self.device = device
                    return self
                device.close()
            raise IvecError("nenhuma Canon IVEC no USB (está ligada?)")
        self.lib, self.ctx, self.devices, found = find_devices(CANON_VENDOR)
        try:
            for dev, desc in found:
                device = UsbDevice(self.ctx, dev, desc)
                ifaces = [f for f in interface_list(device.config_descriptor())
                          if f["cls"] == 7 and f["proto"] == 2]
                for f in ifaces:
                    try:
                        device.claim(f["num"], f["alt"], f["out"], f["in"])
                        self.device = device
                        return self
                    except UsbError:
                        continue
                device.close()
        except BaseException:
            self.close()
            raise
        self.close()
        raise IvecError("nenhuma Canon IVEC disponível no USB (está ligada? outro "
                        "programa está imprimindo?)")

    @property
    def product(self):
        return self.device.product if self.device else ""

    def status(self, timeout=5.0):
        """Primeiro GetStatusResponse completo.

        A impressora responde com o status a qualquer leitura; só pedimos com
        GetStatus se ela ficar quieta. Com erro grave ela pode recusar a escrita,
        mas continua respondendo à leitura.
        """
        buffer, deadline, asked = b"", time.monotonic() + timeout, False
        while time.monotonic() < deadline:
            chunk = self.device.read(700)
            if chunk:
                buffer += chunk
                if b"</cmd>" in buffer:
                    st = parse_status(buffer)
                    if st:
                        return st
            elif not asked:
                asked = True
                try:
                    self.device.write(GET_STATUS)
                except UsbError:
                    pass
        raise IvecError("a impressora não respondeu ao pedido de status")

    def maintenance(self, name, force=False):
        """Envia a operação, recusando se a impressora estiver com erro."""
        st = self.status()
        if st["support_code"] and not force:
            raise IvecError("a impressora está com o erro %s; resolva antes de fazer "
                            "manutenção" % st["support_code"])
        self.device.write(maintenance_job(name))
        return st

    def diagnose(self):
        """Mesmo formato do diagnóstico pelo IPP, mas com o código exato."""
        st = self.status()
        codes = [c for c in [st["support_code"]] + st["error_codes"] if c]
        return {
            "modelo": self.product,
            "estado": STATUS_TEXT.get(st["status"], st["status"]),
            "mensagem": DETAIL_TEXT.get(st["detail"], st["detail"]),
            "motivos": [],
            "problemas": [explain_canon(c) for c in dict.fromkeys(codes)],
            "tintas": [("Caixa de manutenção %s" % m, "%d%%" % lvl if lvl is not None
                        else "n/d") for m, lvl, _ in st["maintenance_box"]],
        }

    def close(self):
        if self.device:
            self.device.close()
            self.device = None
        if self.devices:
            self.lib.libusb_free_device_list(self.devices, 1)
            self.devices = None
        if self.ctx:
            self.lib.libusb_exit(self.ctx)
            self.ctx = None

    def __enter__(self):
        return self.open()

    def __exit__(self, *exc):
        self.close()
