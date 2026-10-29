"""Acesso USB às Epson via libusb (ctypes, sem dependências Python).

UsbCtrlClient tem o mesmo get(oid) do SnmpClient, então EpsonPrinter funciona
igual pelo cabo: os quadros EPSON-CTRL que iriam dentro do OID vão pelo canal
D4. No Mac/Linux usa o libusb (brew install libusb); no Windows, o
usbprint.sys que o próprio Windows instala (veja winusb.py).
"""

import ctypes
import ctypes.util
import os
import sys

from . import winusb
from .d4 import D4Error, EpsonCtrlChannel

EPSON_VENDOR = 0x04B8
EPCTRL_PREFIX = "1.3.6.1.4.1.1248.1.2.2.44.1.1.2.1."
MODEL_OID = "1.3.6.1.2.1.25.3.2.1.3.1"


class UsbError(Exception):
    pass


class _DeviceDescriptor(ctypes.Structure):
    _fields_ = [("bLength", ctypes.c_uint8), ("bDescriptorType", ctypes.c_uint8),
                ("bcdUSB", ctypes.c_uint16), ("bDeviceClass", ctypes.c_uint8),
                ("bDeviceSubClass", ctypes.c_uint8), ("bDeviceProtocol", ctypes.c_uint8),
                ("bMaxPacketSize0", ctypes.c_uint8), ("idVendor", ctypes.c_uint16),
                ("idProduct", ctypes.c_uint16), ("bcdDevice", ctypes.c_uint16),
                ("iManufacturer", ctypes.c_uint8), ("iProduct", ctypes.c_uint8),
                ("iSerialNumber", ctypes.c_uint8), ("bNumConfigurations", ctypes.c_uint8)]


def _load_libusb():
    names = ["libusb-1.0.dylib", "libusb-1.0.0.dylib", "libusb-1.0.so.0", "libusb-1.0.dll"]
    dirs = [getattr(sys, "_MEIPASS", ""), os.path.dirname(os.path.abspath(__file__)),
            "/opt/homebrew/lib", "/usr/local/lib"]
    candidates = [os.path.join(d, n) for d in dirs if d for n in names]
    found = ctypes.util.find_library("usb-1.0")
    if found:
        candidates.append(found)
    for path in candidates:
        try:
            lib = ctypes.CDLL(path)
            break
        except OSError:
            continue
    else:
        raise UsbError("libusb não encontrado (Mac: brew install libusb; "
                       "Fedora: sudo dnf install libusb1)")
    p, i, u8, u16, ui = (ctypes.c_void_p, ctypes.c_int, ctypes.c_uint8,
                         ctypes.c_uint16, ctypes.c_uint)
    sigs = {
        "libusb_init": ([ctypes.POINTER(p)], i),
        "libusb_exit": ([p], None),
        "libusb_get_device_list": ([p, ctypes.POINTER(ctypes.POINTER(p))], ctypes.c_ssize_t),
        "libusb_free_device_list": ([ctypes.POINTER(p), i], None),
        "libusb_get_device_descriptor": ([p, ctypes.POINTER(_DeviceDescriptor)], i),
        "libusb_open": ([p, ctypes.POINTER(p)], i),
        "libusb_close": ([p], None),
        "libusb_get_string_descriptor_ascii": ([p, u8, ctypes.c_char_p, i], i),
        "libusb_control_transfer": ([p, u8, u8, u16, u16, ctypes.c_char_p, u16, ui], i),
        "libusb_set_auto_detach_kernel_driver": ([p, i], i),
        "libusb_claim_interface": ([p, i], i),
        "libusb_release_interface": ([p, i], i),
        "libusb_set_interface_alt_setting": ([p, i, i], i),
        "libusb_bulk_transfer": ([p, ctypes.c_ubyte, ctypes.c_char_p, i,
                                  ctypes.POINTER(i), ui], i),
        "libusb_error_name": ([i], ctypes.c_char_p),
    }
    for name, (args, res) in sigs.items():
        fn = getattr(lib, name)
        fn.argtypes, fn.restype = args, res
    return lib


_lib = None


def libusb():
    global _lib
    if _lib is None:
        _lib = _load_libusb()
    return _lib


def _check(code, what):
    if code < 0:
        name = libusb().libusb_error_name(code).decode()
        if name == "LIBUSB_ERROR_ACCESS":
            raise UsbError("%s: acesso negado ao USB (outro programa está usando a "
                           "impressora?)" % what)
        raise UsbError("%s: %s" % (what, name))
    return code


def interface_list(config):
    """Interfaces com par de endpoints bulk, como dicionários (num, alt, cls, proto...)."""
    found, current, pos = [], None, 0
    while pos + 2 <= len(config):
        length, kind = config[pos], config[pos + 1]
        if length < 2:
            break
        d = config[pos:pos + length]
        if kind == 4 and length >= 9:
            current = {"num": d[2], "alt": d[3], "cls": d[5], "proto": d[7],
                       "out": None, "in": None}
            found.append(current)
        elif kind == 5 and length >= 7 and current is not None and d[3] & 3 == 2:
            current["in" if d[2] & 0x80 else "out"] = d[2]
        pos += length
    return [f for f in found if f["out"] and f["in"]]


def parse_interfaces(config):
    """[(número, alt, classe, ep_out, ep_in)], impressora (classe 7) primeiro."""
    result = [(f["num"], f["alt"], f["cls"], f["out"], f["in"])
              for f in interface_list(config)]
    return sorted(result, key=lambda r: (r[2] != 7, r[2] != 0xFF, r[0], r[1]))


class UsbDevice:
    """Uma Epson no USB, com uma interface aberta (write/read para o D4)."""

    def __init__(self, ctx, dev, desc):
        self.ctx, self.dev, self.desc = ctx, dev, desc
        self.handle = ctypes.c_void_p()
        self.interface = None
        self.ep_out = self.ep_in = None
        _check(libusb().libusb_open(dev, ctypes.byref(self.handle)), "abrir o USB")
        libusb().libusb_set_auto_detach_kernel_driver(self.handle, 1)

    def string(self, index):
        if not index:
            return ""
        buf = ctypes.create_string_buffer(256)
        n = libusb().libusb_get_string_descriptor_ascii(self.handle, index, buf, 256)
        return buf.raw[:n].decode("ascii", "replace") if n > 0 else ""

    @property
    def product(self):
        return self.string(self.desc.iProduct)

    @property
    def serial(self):
        return self.string(self.desc.iSerialNumber)

    def config_descriptor(self):
        buf = ctypes.create_string_buffer(4096)
        n = _check(libusb().libusb_control_transfer(
            self.handle, 0x80, 6, 0x0200, 0, buf, 4096, 2000), "ler descritor USB")
        return buf.raw[:n]

    def claim(self, num, alt, ep_out, ep_in):
        self.release()
        _check(libusb().libusb_claim_interface(self.handle, num), "abrir a interface %d" % num)
        self.interface = num
        if alt:
            _check(libusb().libusb_set_interface_alt_setting(self.handle, num, alt),
                   "escolher alt %d" % alt)
        self.ep_out, self.ep_in = ep_out, ep_in

    def release(self):
        if self.interface is not None:
            libusb().libusb_release_interface(self.handle, self.interface)
            self.interface = None

    def write(self, data):
        done = ctypes.c_int()
        _check(libusb().libusb_bulk_transfer(self.handle, self.ep_out, data, len(data),
                                             ctypes.byref(done), 5000), "enviar pelo USB")

    def read(self, timeout_ms):
        buf = ctypes.create_string_buffer(16384)
        done = ctypes.c_int()
        code = libusb().libusb_bulk_transfer(self.handle, self.ep_in, buf, 16384,
                                             ctypes.byref(done), max(1, timeout_ms))
        if code == -7:  # LIBUSB_ERROR_TIMEOUT
            return buf.raw[:done.value]
        _check(code, "ler pelo USB")
        return buf.raw[:done.value]

    def close(self):
        self.release()
        libusb().libusb_close(self.handle)


def _epson_devices():
    return find_devices(EPSON_VENDOR)


def find_devices(vendor):
    """(lib, ctx, lista, [(dispositivo, descritor)]) dos aparelhos do fabricante."""
    lib = libusb()
    ctx = ctypes.c_void_p()
    _check(lib.libusb_init(ctypes.byref(ctx)), "iniciar o libusb")
    devices = ctypes.POINTER(ctypes.c_void_p)()
    count = _check(lib.libusb_get_device_list(ctx, ctypes.byref(devices)), "listar USB")
    found = []
    for i in range(count):
        desc = _DeviceDescriptor()
        if lib.libusb_get_device_descriptor(devices[i], ctypes.byref(desc)) == 0 \
                and desc.idVendor == vendor:
            found.append((devices[i], desc))
    return lib, ctx, devices, found


def list_usb_printers():
    """[(product, serial)] das Epson ligadas no USB."""
    if sys.platform == "win32":
        result = []
        for path in winusb.device_paths():
            try:
                device = winusb.UsbPrintDevice(path)
                info = device.device_id()
                device.close()
            except OSError:
                continue
            if info.get("MFG", "").upper() == "EPSON":
                result.append((info.get("MDL", "Epson"), info.get("SN", "")))
        return result
    lib, ctx, devices, found = _epson_devices()
    result = []
    try:
        for dev, desc in found:
            try:
                d = UsbDevice(ctx, dev, desc)
            except UsbError:
                result.append(("Epson 0x%04X (sem acesso)" % desc.idProduct, ""))
                continue
            result.append((d.product, d.serial))
            d.close()
    finally:
        lib.libusb_free_device_list(devices, 1)
        lib.libusb_exit(ctx)
    return result


class UsbCtrlClient:
    """Cliente com get(oid) como o SnmpClient, mas pelo cabo USB (D4)."""

    def __init__(self, serial=None):
        self.serial_filter = serial
        self.lib = self.ctx = self.devices = None
        self.device = self.channel = None
        self.port = ""    # Windows: porta do spooler (USB001...)
        self.serial = ""  # número de série do USB, para achar a fila no Mac

    def open(self):
        if sys.platform == "win32":
            return self._open_windows()
        self.lib, self.ctx, self.devices, found = _epson_devices()
        try:
            for dev, desc in found:
                device = UsbDevice(self.ctx, dev, desc)
                if self.serial_filter and device.serial != self.serial_filter:
                    device.close()
                    continue
                if self._open_ctrl(device):
                    self.device = device
                    self.serial = device.serial
                    return self
                device.close()
        except BaseException:
            self.close()
            raise
        self.close()
        raise UsbError("nenhuma Epson respondeu pelo USB (está ligada e com o cabo "
                       "conectado?)")

    def _open_windows(self):
        """No Windows: usbprint.sys, o driver que já vem com a impressora."""
        for path in winusb.device_paths():
            try:
                device = winusb.UsbPrintDevice(path)
            except OSError:
                continue
            try:
                info = device.device_id()
            except OSError:
                info = {}
            if info and info.get("MFG", "").upper() != "EPSON":
                device.close()
                continue
            device.product = info.get("MDL", "Epson")
            channel = EpsonCtrlChannel(device)
            try:
                if channel.open():
                    self.device, self.channel = device, channel
                    self.port = winusb.port_name(path)
                    self.serial = info.get("SN", "")
                    return self
            except (OSError, D4Error):
                channel.close()
            device.close()
        raise UsbError("nenhuma Epson respondeu pelo USB. Confira se está ligada e "
                       "feche o monitor de status da Epson, que pode ocupar a porta.")

    def _open_ctrl(self, device):
        """Tenta cada interface até uma responder ao D4."""
        for num, alt, cls, ep_out, ep_in in parse_interfaces(device.config_descriptor()):
            channel = None
            try:
                device.claim(num, alt, ep_out, ep_in)
                channel = EpsonCtrlChannel(device)
                if channel.open():
                    self.channel = channel
                    return True
            except (UsbError, D4Error):
                if channel:
                    channel.close()  # não deixar a impressora presa no modo D4
            device.release()
        return False

    def get(self, oid):
        if oid == MODEL_OID:
            return self.device.product.encode()
        if not oid.startswith(EPCTRL_PREFIX):
            return None
        frame = bytes(int(x) for x in oid[len(EPCTRL_PREFIX):].split("."))
        try:
            return self.channel.command(frame)
        except D4Error as e:
            raise UsbError(str(e))

    def close(self):
        if self.channel:
            self.channel.close()
            self.channel = None
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
