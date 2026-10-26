"""USB no Windows pelo driver da própria impressora (usbprint.sys), sem libusb.

O Windows instala o usbprint.sys na interface de impressora de qualquer
impressora USB, e ele publica a interface GUID_DEVINTERFACE_USBPRINT: ReadFile
e WriteFile vão direto para os endpoints bulk. Assim o D4 funciona sem trocar
o driver da Epson (sem Zadig/WinUSB).
"""

import ctypes
import sys
import uuid

GUID_DEVINTERFACE_USBPRINT = "{28D78FAD-5A12-11D1-AE5B-0000F803A8C2}"
IOCTL_USBPRINT_GET_1284_ID = 0x220034

GENERIC_READ, GENERIC_WRITE = 0x80000000, 0x40000000
FILE_SHARE_READ, FILE_SHARE_WRITE = 0x1, 0x2
OPEN_EXISTING, FILE_FLAG_OVERLAPPED = 3, 0x40000000
DIGCF_PRESENT, DIGCF_DEVICEINTERFACE = 0x2, 0x10
ERROR_IO_PENDING, WAIT_TIMEOUT = 997, 0x102


class WinUsbError(OSError):
    pass


def parse_device_id(raw):
    """IOCTL_USBPRINT_GET_1284_ID: 2 bytes de tamanho (big-endian) + texto."""
    if len(raw) < 2:
        return {}
    length = int.from_bytes(raw[:2], "big")
    text = raw[2:max(2, min(length, len(raw)))].decode("latin-1", "replace")
    info = {}
    for part in text.split(";"):
        if ":" in part:
            k, v = part.split(":", 1)
            info[k.strip().upper()] = v.strip()
    # nomes longos do IEEE 1284 equivalem aos curtos
    for long_name, short in (("MANUFACTURER", "MFG"), ("MODEL", "MDL"),
                             ("COMMAND SET", "CMD")):
        if long_name in info and short not in info:
            info[short] = info[long_name]
    return info


if sys.platform == "win32":
    from ctypes import wintypes

    class GUID(ctypes.Structure):
        _fields_ = [("Data1", wintypes.DWORD), ("Data2", wintypes.WORD),
                    ("Data3", wintypes.WORD), ("Data4", ctypes.c_ubyte * 8)]

    class SP_DEVICE_INTERFACE_DATA(ctypes.Structure):
        _fields_ = [("cbSize", wintypes.DWORD), ("InterfaceClassGuid", GUID),
                    ("Flags", wintypes.DWORD), ("Reserved", ctypes.c_size_t)]

    class OVERLAPPED(ctypes.Structure):
        _fields_ = [("Internal", ctypes.c_size_t), ("InternalHigh", ctypes.c_size_t),
                    ("Offset", wintypes.DWORD), ("OffsetHigh", wintypes.DWORD),
                    ("hEvent", wintypes.HANDLE)]

    _setupapi = ctypes.WinDLL("setupapi", use_last_error=True)
    _k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _vp, _dw, _bool = ctypes.c_void_p, wintypes.DWORD, wintypes.BOOL
    for _name, _args, _res in [
        ("SetupDiGetClassDevsW", [_vp, wintypes.LPCWSTR, _vp, _dw], _vp),
        ("SetupDiEnumDeviceInterfaces", [_vp, _vp, _vp, _dw, _vp], _bool),
        ("SetupDiGetDeviceInterfaceDetailW", [_vp, _vp, _vp, _dw,
                                              ctypes.POINTER(_dw), _vp], _bool),
        ("SetupDiDestroyDeviceInfoList", [_vp], _bool),
    ]:
        _fn = getattr(_setupapi, _name)
        _fn.argtypes, _fn.restype = _args, _res
    for _name, _args, _res in [
        ("CreateFileW", [wintypes.LPCWSTR, _dw, _dw, _vp, _dw, _dw, _vp], _vp),
        ("CloseHandle", [_vp], _bool),
        ("CreateEventW", [_vp, _bool, _bool, wintypes.LPCWSTR], _vp),
        ("WriteFile", [_vp, _vp, _dw, _vp, _vp], _bool),
        ("ReadFile", [_vp, _vp, _dw, _vp, _vp], _bool),
        ("DeviceIoControl", [_vp, _dw, _vp, _dw, _vp, _dw, _vp, _vp], _bool),
        ("GetOverlappedResult", [_vp, _vp, ctypes.POINTER(_dw), _bool], _bool),
        ("WaitForSingleObject", [_vp, _dw], _dw),
        ("CancelIoEx", [_vp, _vp], _bool),
    ]:
        _fn = getattr(_k32, _name)
        _fn.argtypes, _fn.restype = _args, _res

    _INVALID = ctypes.c_void_p(-1).value

    def _ptr(buf):
        return ctypes.cast(buf, ctypes.c_void_p)

    def device_paths():
        """Caminhos de todas as impressoras USB vistas pelo usbprint.sys."""
        guid = GUID.from_buffer_copy(uuid.UUID(GUID_DEVINTERFACE_USBPRINT).bytes_le)
        hdev = _setupapi.SetupDiGetClassDevsW(ctypes.byref(guid), None, None,
                                              DIGCF_PRESENT | DIGCF_DEVICEINTERFACE)
        if not hdev or hdev == _INVALID:
            return []
        paths = []
        try:
            index = 0
            while True:
                data = SP_DEVICE_INTERFACE_DATA()
                data.cbSize = ctypes.sizeof(data)
                if not _setupapi.SetupDiEnumDeviceInterfaces(
                        hdev, None, ctypes.byref(guid), index, ctypes.byref(data)):
                    break
                index += 1
                required = wintypes.DWORD()
                _setupapi.SetupDiGetDeviceInterfaceDetailW(
                    hdev, ctypes.byref(data), None, 0, ctypes.byref(required), None)
                if not required.value:
                    continue
                buf = ctypes.create_string_buffer(required.value)
                # cbSize da estrutura de detalhe: 8 em 64 bits, 6 em 32 bits
                cb = 8 if ctypes.sizeof(ctypes.c_void_p) == 8 else 6
                ctypes.memmove(buf, ctypes.byref(wintypes.DWORD(cb)), 4)
                if _setupapi.SetupDiGetDeviceInterfaceDetailW(
                        hdev, ctypes.byref(data), buf, required, None, None):
                    paths.append(ctypes.wstring_at(ctypes.addressof(buf) + 4))
        finally:
            _setupapi.SetupDiDestroyDeviceInfoList(hdev)
        return paths

    class UsbPrintDevice:
        """Uma impressora USB aberta pelo usbprint.sys (write/read para o D4)."""

        interface = 0  # compatível com o UsbDevice do libusb

        def __init__(self, path):
            self.path = path
            self.product = ""
            self.handle = _k32.CreateFileW(
                path, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None)
            if not self.handle or self.handle == _INVALID:
                raise WinUsbError("não consegui abrir a impressora USB (está imprimindo "
                                  "ou o monitor de status da Epson está usando?)")

        def _io(self, start, timeout_ms):
            """Roda uma operação assíncrona com prazo; devolve os bytes transferidos."""
            ov = OVERLAPPED()
            ov.hEvent = _k32.CreateEventW(None, True, False, None)
            try:
                if not start(ctypes.byref(ov)):
                    err = ctypes.get_last_error()
                    if err != ERROR_IO_PENDING:
                        raise WinUsbError("erro %d no USB" % err)
                done = wintypes.DWORD()
                if _k32.WaitForSingleObject(ov.hEvent, max(1, timeout_ms)) == WAIT_TIMEOUT:
                    _k32.CancelIoEx(self.handle, ctypes.byref(ov))
                    _k32.GetOverlappedResult(self.handle, ctypes.byref(ov),
                                             ctypes.byref(done), True)
                    return done.value
                _k32.GetOverlappedResult(self.handle, ctypes.byref(ov),
                                         ctypes.byref(done), False)
                return done.value
            finally:
                _k32.CloseHandle(ov.hEvent)

        def write(self, data):
            buf = ctypes.create_string_buffer(data, len(data))
            n = self._io(lambda ov: _k32.WriteFile(self.handle, _ptr(buf), len(data), None, ov),
                         5000)
            if n != len(data):
                raise WinUsbError("a impressora não aceitou os dados pelo USB")

        def read(self, timeout_ms):
            buf = ctypes.create_string_buffer(16384)
            n = self._io(lambda ov: _k32.ReadFile(self.handle, _ptr(buf), 16384, None, ov),
                         timeout_ms)
            return buf.raw[:n]

        def device_id(self):
            buf = ctypes.create_string_buffer(1024)
            n = self._io(lambda ov: _k32.DeviceIoControl(
                self.handle, IOCTL_USBPRINT_GET_1284_ID, None, 0, _ptr(buf), 1024, None, ov),
                2000)
            return parse_device_id(buf.raw[:n])

        def release(self):
            pass

        def close(self):
            if self.handle:
                _k32.CloseHandle(self.handle)
                self.handle = None
else:
    def device_paths():
        return []

    class UsbPrintDevice:
        def __init__(self, path):
            raise WinUsbError("usbprint.sys só existe no Windows")
