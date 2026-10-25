"""Teste de bicos e limpeza de cabeça (comandos ESC/P2 em "remote mode").

Os bytes podem ir pela rede (porta RAW 9100) ou direto para uma impressora
instalada (USB ou rede), como um trabalho RAW: pelo spooler no Windows e pelo
CUPS (lp -o raw) no Mac e no Linux.
"""

import socket
import subprocess
import sys

EXIT_PACKET_MODE = b"\x00\x00\x00\x1b\x01@EJL 1284.4\n@EJL     \n"
RESET = b"\x1b@"
ENTER_REMOTE = b"\x1b(R\x08\x00\x00REMOTE1"
EXIT_REMOTE = b"\x1b\x00\x00\x00"


def remote_job(cmd, args=b"\x00\x00"):
    """Um trabalho completo com um comando remoto: 2 letras + tamanho + args."""
    body = cmd.encode("ascii") + len(args).to_bytes(2, "little") + args
    return EXIT_PACKET_MODE + RESET + ENTER_REMOTE + body + EXIT_REMOTE + RESET


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
