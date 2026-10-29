"""Teste de bicos e limpeza de cabeça (comandos ESC/P2 em "remote mode").

Os bytes podem ir pela rede (porta RAW 9100) ou direto para uma impressora
instalada (USB ou rede), como um trabalho RAW: pelo spooler no Windows e pelo
CUPS (lp -o raw) no Mac e no Linux.
"""

import re
import socket
import subprocess
import sys
import urllib.parse

EXIT_PACKET_MODE = b"\x00\x00\x00\x1b\x01@EJL 1284.4\n@EJL     \n"
RESET = b"\x1b@"
ENTER_REMOTE = b"\x1b(R\x08\x00\x00REMOTE1"
EXIT_REMOTE = b"\x1b\x00\x00\x00\x1b\x00"
# Fim do trabalho: avança a folha e encerra. Suspeita: sem isso (e com um ESC @ logo
# depois do modo remoto, como era antes) a impressora pode ficar esperando o
# resto da página, com o papel e o carro indo e voltando.
END_JOB = b"\x0c\x1b\x00\x1b\x00"


def remote_job(cmd, args=b"\x00\x00"):
    """Um trabalho completo com um comando remoto: 2 letras + tamanho + args.

    Mesma moldura que o escputil (gutenprint) usa: sai do modo pacote,
    inicializa, entra no modo remoto, comando, sai do modo remoto, fim.
    """
    body = cmd.encode("ascii") + len(args).to_bytes(2, "little") + args
    return EXIT_PACKET_MODE + RESET + RESET + ENTER_REMOTE + body + EXIT_REMOTE + END_JOB


def nozzle_check_job():
    return remote_job("NC")


def head_cleaning_job():
    return remote_job("CH")


def send_raw_network(host, data, port=9100, timeout=10.0):
    with socket.create_connection((host, port), timeout=timeout) as sock:
        sock.sendall(data)


def installed_printers():
    """Impressoras instaladas no sistema ([] se não houver spooler)."""
    return windows_printers() if sys.platform == "win32" else cups_printers()


def queues_for_usb(port="", serial="", model=""):
    """Filas instaladas que imprimem na impressora USB detectada.

    Windows: as que usam a porta (USB001...). Mac/Linux: as do CUPS com o mesmo
    número de série no endereço usb://; sem série, as do mesmo modelo.
    """
    if sys.platform == "win32":
        return [name for name, ports in windows_printer_ports().items()
                if port and port.upper() in ports]
    return match_cups_queues(cups_devices(), serial, model)


def match_cups_queues(devices, serial="", model=""):
    """devices: {fila: uri}. A série é a do USB; o CUPS às vezes a grava em hexadecimal."""
    wanted = {serial.upper(), serial.encode().hex().upper()} - {""}
    if serial:
        try:
            wanted.add(bytes.fromhex(serial).decode("ascii").upper())
        except ValueError:
            pass
    by_serial, by_model = [], []
    for name, uri in devices.items():
        if not uri.startswith("usb://"):
            continue
        query = urllib.parse.parse_qs(urllib.parse.urlsplit(uri).query)
        if wanted & {v.upper() for v in query.get("serial", [])}:
            by_serial.append(name)
        elif model and model.upper() in urllib.parse.unquote(uri).upper():
            by_model.append(name)
    return by_serial or by_model


def send_raw_installed(printer_name, data):
    if sys.platform == "win32":
        send_raw_windows(printer_name, data)
    else:
        send_raw_cups(printer_name, data)


# ---- CUPS (Mac e Linux) ----------------------------------------------------

def cups_printers():
    try:
        out = subprocess.run(["lpstat", "-e"], capture_output=True, text=True,
                             timeout=10).stdout
    except (OSError, subprocess.TimeoutExpired):
        return []
    return [line.strip() for line in out.splitlines() if line.strip()]


def cups_devices():
    """{fila: endereço do dispositivo} do CUPS (lpstat -v)."""
    try:
        out = subprocess.run(["lpstat", "-v"], capture_output=True, text=True,
                             timeout=10).stdout
    except (OSError, subprocess.TimeoutExpired):
        return {}
    devices = {}
    for line in out.splitlines():
        # "device for FILA: uri" (o Mac traduz o começo, então só a fila e a uri)
        m = re.search(r"(\S+): ([a-z][a-z0-9+.-]*:\S*)$", line.strip())
        if m:
            devices[m.group(1)] = m.group(2)
    return devices


def send_raw_cups(printer_name, data):
    result = subprocess.run(["lp", "-d", printer_name, "-o", "raw", "-t", "EcoReset"],
                            input=data, capture_output=True, timeout=30)
    if result.returncode:
        raise OSError("o CUPS recusou o trabalho: %s"
                      % result.stderr.decode(errors="replace").strip())


# ---- spooler do Windows (sem pywin32, só ctypes) -------------------------

def windows_printers():
    """Nomes das impressoras instaladas no Windows ([] em outros sistemas)."""
    if sys.platform != "win32":
        return []
    import ctypes
    from ctypes import wintypes

    class PRINTER_INFO_4(ctypes.Structure):
        _fields_ = [("pPrinterName", wintypes.LPWSTR),
                    ("pServerName", wintypes.LPWSTR),
                    ("Attributes", wintypes.DWORD)]

    winspool = ctypes.WinDLL("winspool.drv")
    flags = 0x2 | 0x4  # PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS
    needed, count = wintypes.DWORD(), wintypes.DWORD()
    winspool.EnumPrintersW(flags, None, 4, None, 0, ctypes.byref(needed),
                           ctypes.byref(count))
    if not needed.value:
        return []
    buf = ctypes.create_string_buffer(needed.value)
    if not winspool.EnumPrintersW(flags, None, 4, buf, needed, ctypes.byref(needed),
                                  ctypes.byref(count)):
        return []
    infos = ctypes.cast(buf, ctypes.POINTER(PRINTER_INFO_4))
    return [infos[i].pPrinterName for i in range(count.value)]


def windows_printer_ports():
    """{impressora: [portas]} das instaladas no Windows ({} em outros sistemas)."""
    if sys.platform != "win32":
        return {}
    import ctypes
    from ctypes import wintypes

    class PRINTER_INFO_2(ctypes.Structure):
        _fields_ = [(n, wintypes.LPWSTR) for n in (
            "pServerName", "pPrinterName", "pShareName", "pPortName", "pDriverName",
            "pComment", "pLocation")] + [("pDevMode", ctypes.c_void_p)] + [
            (n, wintypes.LPWSTR) for n in ("pSepFile", "pPrintProcessor", "pDatatype",
                                           "pParameters")] + [
            ("pSecurityDescriptor", ctypes.c_void_p)] + [(n, wintypes.DWORD) for n in (
                "Attributes", "Priority", "DefaultPriority", "StartTime", "UntilTime",
                "Status", "cJobs", "AveragePPM")]

    winspool = ctypes.WinDLL("winspool.drv")
    flags = 0x2  # PRINTER_ENUM_LOCAL: as USB são sempre locais
    needed, count = wintypes.DWORD(), wintypes.DWORD()
    winspool.EnumPrintersW(flags, None, 2, None, 0, ctypes.byref(needed),
                           ctypes.byref(count))
    if not needed.value:
        return {}
    buf = ctypes.create_string_buffer(needed.value)
    if not winspool.EnumPrintersW(flags, None, 2, buf, needed, ctypes.byref(needed),
                                  ctypes.byref(count)):
        return {}
    infos = ctypes.cast(buf, ctypes.POINTER(PRINTER_INFO_2))
    return {infos[i].pPrinterName: [p.strip().upper() for p in
                                     (infos[i].pPortName or "").split(",") if p.strip()]
            for i in range(count.value)}


def send_raw_windows(printer_name, data, doc_name="EcoReset"):
    if sys.platform != "win32":
        raise OSError("o envio pelo spooler só funciona no Windows")
    import ctypes
    from ctypes import wintypes

    class DOC_INFO_1(ctypes.Structure):
        _fields_ = [("pDocName", wintypes.LPWSTR),
                    ("pOutputFile", wintypes.LPWSTR),
                    ("pDatatype", wintypes.LPWSTR)]

    winspool = ctypes.WinDLL("winspool.drv", use_last_error=True)
    handle = wintypes.HANDLE()
    if not winspool.OpenPrinterW(printer_name, ctypes.byref(handle), None):
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        doc = DOC_INFO_1(doc_name, None, "RAW")
        if not winspool.StartDocPrinterW(handle, 1, ctypes.byref(doc)):
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            winspool.StartPagePrinter(handle)
            written = wintypes.DWORD()
            ok = winspool.WritePrinter(handle, data, len(data), ctypes.byref(written))
            winspool.EndPagePrinter(handle)
            if not ok or written.value != len(data):
                raise ctypes.WinError(ctypes.get_last_error())
        finally:
            winspool.EndDocPrinter(handle)
    finally:
        winspool.ClosePrinter(handle)
