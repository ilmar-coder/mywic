import json
import os
import re
import socket
import tempfile
import threading
import unittest

from ecoreset import d4, maintenance, snmp
from ecoreset.d4 import EpsonCtrlChannel
from ecoreset.models import MODELS, find_model
from ecoreset.printer import (EpsonPrinter, PrinterError, read_oid, shift_key,
                              waste_percent, write_oid)
from tests.fake_d4 import FakeD4Device
from tests.fake_printer import FakePrinter


class TestProtocol(unittest.TestCase):
    def test_read_oid_matches_documented_example(self):
        self.assertEqual(
            read_oid((25, 7), 173),
            "1.3.6.1.4.1.1248.1.2.2.44.1.1.2.1.124.124.7.0.25.7.65.190.160.173.0")

    def test_write_oid_matches_documented_bytes(self):
        oid = write_oid((73, 8), b"Arantifo", 48, 0x1A)
        data = bytes(int(x) for x in oid.split(".")[15:])
        self.assertEqual(data.hex(" ").upper(),
                         "7C 7C 10 00 49 08 42 BD 21 30 00 1A 42 73 62 6F 75 6A 67 70")

    def test_shift_key(self):
        self.assertEqual(shift_key(b"Gazania*").hex(" ").upper(),
                         "48 62 7B 62 6F 6A 62 2B")

    def test_waste_percent_is_little_endian(self):
        self.assertEqual(waste_percent([0xA4, 0x2A], 100.0), round(0x2AA4 / 100, 2))

    def test_oid_roundtrip_with_large_arcs(self):
        oid = "1.3.6.1.4.1.1248.255.128.300.70000"
        encoded = snmp._encode_oid(oid)
        self.assertEqual(snmp.decode_oid(encoded[2:]), oid)

    def test_long_length_encoding(self):
        packet = snmp.build_get("1.3.6.1." + ".".join(["200"] * 100))
        tag, value, end = snmp._decode_tlv(packet, 0)
        self.assertEqual(end, len(packet))


class TestModels(unittest.TestCase):
    def test_find_model(self):
        self.assertEqual(find_model("EPSON L3150 Series"), "L3150")
        self.assertEqual(find_model("EPSON L4150 Series"), "L4160")
        self.assertEqual(find_model("EPSON L395 Series"), "ET-2600")
        self.assertIsNone(find_model("EPSON L31500"))

    def test_models_are_consistent(self):
        for name, cfg in MODELS.items():
            self.assertEqual(len(cfg["write_key"]), 8, name)
            self.assertEqual(len(cfg["read_key"]), 2, name)
            self.assertIn("main_waste", cfg, name)


class TestWithFakePrinter(unittest.TestCase):
    def setUp(self):
        # contador principal 0x0C10 (~48%) e sem margem 0x0120 (~8%) num L3150
        self.fake = FakePrinter(eeprom={48: 0x10, 49: 0x0C, 47: 0x00, 50: 0x20, 51: 0x01})

    def tearDown(self):
        self.fake.close()

    def printer(self, **kw):
        return EpsonPrinter("127.0.0.1", port=self.fake.port, timeout=0.5, retries=0, **kw)

    def test_detect_and_levels(self):
        p = self.printer()
        text, name = p.detect()
        self.assertEqual((text, name), ("EPSON L3150 Series", "L3150"))
        levels = p.waste_levels()
        self.assertEqual(levels["principal"], round(0x0C10 / 63.46, 2))
        self.assertEqual(levels["sem margem"], round(0x0120 / 34.16, 2))

    def test_device_id(self):
        self.assertEqual(self.printer().device_id()["MDL"], "EPSON L3150 Series")

    def test_reset_writes_plan_and_backup(self):
        p = self.printer(model="L3150")
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "b.json")
            p.reset_waste(backup_path=path)
            with open(path) as f:
                backup = json.load(f)
        self.assertEqual(backup["eeprom"]["48"], 0x10)
        for address, value in MODELS["L3150"]["raw_waste_reset"].items():
            self.assertEqual(self.fake.eeprom[address], value)
        self.assertEqual(p.waste_levels()["principal"], 0)
        p.restore(backup)
        self.assertEqual(self.fake.eeprom[49], 0x0C)

    def test_wrong_read_key_is_reported(self):
        p = self.printer(model="L4160")
        with self.assertRaises(PrinterError):
            p.read_eeprom(48)

    def test_locked_firmware_refuses_write(self):
        self.fake.locked = True
        p = self.printer(model="L3150")
        with self.assertRaisesRegex(PrinterError, "recusou a gravação"):
            p.write_eeprom(48, 0)

    def test_temporary_reset_reads_serial_from_status(self):
        self.fake.locked = "all"
        p = self.printer(model="ET-2812")
        with self.assertRaisesRegex(PrinterError, "reset temporário"):
            p.read_eeprom(48)
        self.assertEqual(p.temporary_reset(), "X5NY012345")
        self.assertEqual(self.fake.temp_resets, 1)
        with self.assertRaises(PrinterError):
            p.temporary_reset("ERRADO")

    def test_serial_in_field_0x40_like_real_l3250(self):
        reply = (b"\x00@BDC ST2\r\n\x10\x00\x01\x01\x04\x19\x01\x00"
                 b"\x40\x0aXAAB402930")
        from ecoreset.printer import parse_status
        p = self.printer()
        p.status = lambda: parse_status(reply)
        self.assertEqual(p.serial_number(), "XAAB402930")

    def test_l3250_uses_et2812_and_commit_clears_bit0(self):
        self.fake.close()
        self.fake = FakePrinter(name="EPSON L3250 Series", read_key=(74, 54),
                                eeprom={48: 5, 0x100: 0x13, 254: 2})
        p = self.printer()
        self.assertEqual(p.detect()[1], "ET-2812")
        self.assertIn("terceira", p.waste_levels())
        p.reset_waste()
        self.assertEqual(self.fake.eeprom[0x100], 0x12)
        self.assertEqual(self.fake.eeprom[254], 0)
        self.assertEqual(self.fake.eeprom[255], 94)

    def test_timeout(self):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        try:
            client = snmp.SnmpClient("127.0.0.1", sock.getsockname()[1],
                                     timeout=0.2, retries=0)
            with self.assertRaises(snmp.SnmpError):
                client.get("1.3.6.1.2.1.1.1.0")
        finally:
            sock.close()


class TestD4(unittest.TestCase):
    def setUp(self):
        self.fake = FakePrinter(name="EPSON L3250 Series", read_key=(74, 54),
                                eeprom={48: 0x10, 49: 0x0C, 0x100: 0x13})
        self.fake.close()  # só a lógica de respostas, sem o servidor UDP

    def client(self, device):
        from ecoreset.usb import UsbCtrlClient
        client = UsbCtrlClient()
        channel = EpsonCtrlChannel(device)
        self.assertTrue(channel.open())
        client.channel = channel
        client.device = type("Dev", (), {"product": "L3250 Series",
                                         "close": lambda self: None})()
        return client

    def test_full_reset_over_d4(self):
        device = FakeD4Device(self.fake)
        p = EpsonPrinter("USB", client=self.client(device))
        self.assertEqual(p.detect(), ("L3250 Series", "ET-2812"))
        self.assertEqual(p.read_eeprom(48), 0x10)
        self.assertEqual(p.serial_number(), "X5NY012345")
        p.reset_waste()
        self.assertEqual(self.fake.eeprom[48], 0)
        self.assertEqual(self.fake.eeprom[0x100], 0x12)
        p.snmp.close()
        self.assertFalse(device.in_d4, "a sessão D4 precisa terminar com Exit")
        self.assertEqual(device.log[:3], [d4.INIT, d4.GET_SOCKET_ID, d4.OPEN_CHANNEL])

    def test_silent_interface_returns_false(self):
        channel = EpsonCtrlChannel(FakeD4Device(self.fake, silent=True))
        channel.d4.timeout_ms = 50
        self.assertFalse(channel.d4.enter(timeout_ms=50))

    def test_parse_interfaces_prefers_printer_class(self):
        from ecoreset.usb import parse_interfaces
        config = bytes([9, 2, 0, 0, 2, 1, 0, 0xC0, 1,
                        9, 4, 0, 0, 2, 0xFF, 0, 0, 0,
                        7, 5, 0x01, 2, 0, 2, 0,
                        7, 5, 0x82, 2, 0, 2, 0,
                        9, 4, 1, 0, 2, 7, 1, 2, 0,
                        7, 5, 0x03, 2, 0, 2, 0,
                        7, 5, 0x84, 2, 0, 2, 0,
                        9, 4, 2, 0, 1, 7, 1, 1, 0,
                        7, 5, 0x05, 2, 0, 2, 0])
        self.assertEqual(parse_interfaces(config),
                         [(1, 0, 7, 0x03, 0x84), (0, 0, 0xFF, 0x01, 0x82)])


def _ipp_response(attrs):
    """Monta uma resposta IPP com atributos de impressora (grupo 0x04)."""
    import struct
    body = struct.pack(">BBHI", 1, 1, 0, 1) + b"\x01" + b"\x04"
    for name, (tag, values) in attrs.items():
        for i, v in enumerate(values):
            n = name.encode() if i == 0 else b""
            raw = struct.pack(">i", v) if tag in (0x21, 0x23) else v.encode()
            body += struct.pack(">BH", tag, len(n)) + n + struct.pack(">H", len(raw)) + raw
    return body + b"\x03"


class TestDiagnostics(unittest.TestCase):
    def test_canon_codes_from_official_list(self):
        from ecoreset.errors import explain_canon, search
        self.assertEqual(explain_canon("1726").title, "Caixa de manutenção cheia")
        self.assertFalse(explain_canon("1300").service)
        self.assertTrue(explain_canon("5c30").service)
        self.assertIn("não catalogado", explain_canon("9999").title)
        self.assertTrue(any(p.code == "1300" for p in search("1300")))
        self.assertTrue(any(p.brand == "Epson" for p in search("almofada")))

    def test_ipp_parse_and_canon_diagnose(self):
        from ecoreset import canon, ipp
        data = _ipp_response({
            "printer-make-and-model": (0x41, ["Canon G2060 series"]),
            "printer-state": (0x23, [5]),
            "printer-state-reasons": (0x44, ["media-jam-error", "marker-supply-low-warning"]),
            "printer-state-message": (0x41, ["Support Code: 1300"]),
            "marker-names": (0x41, ["Black", "Color"]),
            "marker-levels": (0x21, [80, 15]),
        })
        status, attrs = ipp.parse_response(data)
        self.assertEqual(status, 0)
        self.assertEqual(attrs["marker-levels"], [80, 15])
        d = canon.diagnose_attrs(attrs)
        self.assertEqual(d["estado"], "Parada")
        self.assertEqual(d["motivos"], ["Papel enroscado", "Tinta baixa"])
        self.assertEqual([p.code for p in d["problemas"]], ["1300"])
        self.assertEqual(d["tintas"], [("Black", "80%"), ("Color", "15%")])

    def test_canon_command_sets_from_real_g2060_device_id(self):
        from ecoreset import canon
        attrs = {"printer-device-id": [
            "MFG:Canon;CMD:IVEC,URF;SOJ:CHMPu;MDL:G2060 series;CLS:PRINTER;STA:10;"]}
        self.assertEqual(canon.command_sets(attrs), ["IVEC", "URF"])
        stopped = canon.diagnose_attrs({"printer-state": [5],
                                        "printer-state-reasons": ["other-error"],
                                        "marker-names": ["Cyan", "MC"],
                                        "marker-levels": [-2, 90]})
        self.assertIn("não informou o código", stopped["mensagem"])
        self.assertEqual(stopped["tintas"], [("Cyan", "sem sensor"), ("MC", "90%")])

    def test_ipp_request_is_well_formed(self):
        from ecoreset import ipp
        req = ipp.build_request("ipp://1.2.3.4/ipp/print")
        self.assertEqual(req[:4], b"\x01\x01\x00\x0b")
        self.assertTrue(req.endswith(b"ipp://1.2.3.4/ipp/print\x03"))

    def test_canon_bjl_jobs(self):
        from ecoreset import canon
        self.assertEqual(canon.nozzle_check_job(),
                         b"\x1b[K\x02\x00\x00\x1fBJLSTART\n@TestPrint=NozzleCheck\nBJLEND\n")
        self.assertIn(b"@Cleaning=1ALL\n", canon.head_cleaning_job())

    def test_epson_diagnose_waste_ink_error(self):
        from ecoreset.printer import parse_status
        items = b"\x01\x01\x00" + b"\x02\x01\x10" + b"\x04\x01\x10"
        reply = b"\x00@BDC ST2\r\n" + len(items).to_bytes(2, "little") + items
        fake = FakePrinter()
        fake.close()
        p = EpsonPrinter("x", client=fake)
        p.status = lambda: parse_status(reply)
        d = p.diagnose()
        self.assertEqual(d["estado"], "Erro")
        self.assertEqual(d["problemas"][0].title, "Almofada de tinta cheia")
        self.assertEqual(d["avisos"], ["Tinta baixa (preto ou amarelo)"])


G2060_STATUS = (
    b'<?xml version="1.0" encoding="UTF-8"?>\r\n<cmd xmlns:ivec="http://www.canon.com/ns/'
    b'cmd/2008/07/common/">\r\n<ivec:contents>\r\n<ivec:operation>GetStatusResponse'
    b'</ivec:operation>\r\n<ivec:param_set servicetype="print">\r\n<ivec:response>OK'
    b'</ivec:response>\r\n<ivec:status>notready</ivec:status>\r\n<ivec:status_detail>'
    b'AttentionRequired</ivec:status_detail>\r\n<ivec:current_support_code>7800'
    b'</ivec:current_support_code>\r\n<ivec:marker_info><ivec:ink><ivec:level>-2'
    b'</ivec:level></ivec:ink></ivec:marker_info>\r\n<ivec:wasteink>\r\n<ivec:item>\r\n'
    b'<ivec:icon>warning</ivec:icon>\r\n<ivec:model>MC-G02</ivec:model>\r\n<ivec:level>90'
    b'</ivec:level>\r\n</ivec:item>\r\n</ivec:wasteink>\r\n<ivec:status_detail_list>'
    b'<ivec:item id="11"><ivec:error_code>7800</ivec:error_code></ivec:item>'
    b'</ivec:status_detail_list>\r\n</ivec:param_set>\r\n</ivec:contents>\r\n</cmd>')


class FakeIvecDevice:
    """Responde a toda leitura com o status, como as Canon IVEC."""

    def __init__(self, status):
        self.status, self.written, self.pending = status, [], status

    def read(self, timeout_ms):
        data, self.pending = self.pending, b""
        return data

    def write(self, data):
        self.written.append(data)
        self.pending = self.status


class TestIvec(unittest.TestCase):
    def test_parse_real_g2060_status(self):
        from ecoreset import ivec
        st = ivec.parse_status(G2060_STATUS)
        self.assertEqual(st["status"], "notready")
        self.assertEqual(st["support_code"], "7800")
        self.assertEqual(st["maintenance_box"], [("MC-G02", 90, "warning")])
        self.assertEqual(st["error_codes"], [])

    def test_maintenance_job_layout(self):
        import datetime as dt
        from ecoreset import ivec
        job = ivec.maintenance_job("limpeza", now=dt.datetime(2026, 10, 25, 17, 0, 0)).decode()
        ops = re.findall(r"<ivec:operation>(\w+)</ivec:operation>", job)
        self.assertEqual(ops, ["StartJob", "SetJobConfiguration", "Cleaning", "EndJob"])
        self.assertIn('<ivec:param_set servicetype="maintenance"><ivec:inkgroup>all'
                      '</ivec:inkgroup><ivec:type>regular</ivec:type><ivec:jobID>00000002'
                      '</ivec:jobID>', job)
        self.assertIn("<ivec:datetime>20261025170000</ivec:datetime>", job)
        self.assertEqual(job.count("<?xml"), 4)
        nozzle = ivec.maintenance_job("teste-bicos").decode()
        self.assertIn("<ivec:type>nozzle_check</ivec:type>", nozzle)
        self.assertNotIn("inkgroup", nozzle)

    def test_maintenance_refused_on_error_and_sent_when_idle(self):
        from ecoreset import ivec
        canon_usb = ivec.CanonUsb()
        canon_usb.device = FakeIvecDevice(G2060_STATUS)
        with self.assertRaisesRegex(ivec.IvecError, "7800"):
            canon_usb.maintenance("limpeza")
        self.assertEqual(canon_usb.device.written, [])
        idle = G2060_STATUS.replace(b"notready", b"idle").replace(
            b"<ivec:current_support_code>7800", b"<ivec:current_support_code>0").replace(
            b"<ivec:error_code>7800", b"<ivec:error_code>0")
        canon_usb.device = FakeIvecDevice(idle)
        canon_usb.maintenance("teste-bicos")
        self.assertIn(b"nozzle_check", canon_usb.device.written[-1])
        canon_usb.device = None

    def test_diagnose_explains_code(self):
        from ecoreset import ivec
        canon_usb = ivec.CanonUsb()
        canon_usb.device = FakeIvecDevice(G2060_STATUS)
        canon_usb.device.product = "G2060 series"
        d = canon_usb.diagnose()
        self.assertEqual(d["estado"], "Não está pronta")
        self.assertEqual([p.code for p in d["problemas"]], ["7800"])
        self.assertTrue(d["problemas"][0].service)
        self.assertEqual(d["tintas"], [("Caixa de manutenção MC-G02", "90%")])
        canon_usb.device = None


class TestSnapshot(unittest.TestCase):
    def setUp(self):
        self.fake = FakePrinter(name="EPSON L3210 Series", read_key=(74, 54),
                                eeprom={47: 0, 48: 151, 49: 13, 54: 30, 90: 7, 1604: 88})

    def tearDown(self):
        self.fake.close()

    def printer(self):
        p = EpsonPrinter("127.0.0.1", port=self.fake.port, timeout=0.5, retries=0)
        self.assertEqual(p.detect()[1], None)  # L3210 não está na lista
        p.read_key = (74, 54)  # chave informada à mão
        return p

    def test_take_compare_and_suggest(self):
        from ecoreset import snapshot
        p = self.printer()
        with tempfile.TemporaryDirectory() as d:
            before = snapshot.take(p, 0, 120, label="antes")
            snapshot.save(before, os.path.join(d, "a.json"))
            # "outra ferramenta" faz o reset
            self.fake.eeprom.update({48: 0, 49: 0, 54: 94, 90: 8})
            after = snapshot.take(p, 0, 120, label="depois")
            snapshot.save(after, os.path.join(d, "b.json"))
            before = snapshot.load(os.path.join(d, "a.json"))
            after = snapshot.load(os.path.join(d, "b.json"))
        self.assertEqual(before["serial"], "X5NY012345")
        self.assertEqual(len(before["eeprom"]), 121)
        result = snapshot.compare(before, after)
        self.assertEqual(result["avisos"], [])
        self.assertEqual(result["mudancas"], [(48, 151, 0), (49, 13, 0), (54, 30, 94),
                                              (90, 7, 8)])
        self.assertEqual(snapshot.classify(48, 151, 0), "provável contador zerado")
        self.assertIn("94", snapshot.classify(54, 30, 94))
        s = snapshot.suggest_model(before, after, result["mudancas"])
        self.assertEqual(s["raw_waste_reset"], {"48": 0, "49": 0, "54": 94})
        self.assertEqual(s["ignorados"], [90])
        self.assertEqual(s["read_key"], [74, 54])

    def test_wrong_key_stops_early(self):
        from ecoreset import snapshot
        p = self.printer()
        p.read_key = (1, 2)
        with self.assertRaisesRegex(PrinterError, "16 primeiros"):
            snapshot.take(p, 0, 2047)

    def test_compare_warns_on_different_printers(self):
        from ecoreset import snapshot
        a = {"serial": "AAA", "eeprom": {"1": 1}}
        b = {"serial": "BBB", "eeprom": {"1": 2, "2": 3}}
        result = snapshot.compare(a, b)
        self.assertEqual(len(result["avisos"]), 2)
        self.assertEqual(result["mudancas"], [(1, 1, 2)])

    def test_snapshot_file_is_a_valid_restore_image(self):
        from ecoreset import snapshot
        from ecoreset.printer import load_image
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "s.json")
            snapshot.save({"model": "ET-2812", "eeprom": {"48": 5}}, path)
            self.assertEqual(load_image(path), ({48: 5}, "ET-2812"))


class TestWindowsUsb(unittest.TestCase):
    def test_parse_device_id(self):
        from ecoreset.winusb import parse_device_id
        text = b"MFG:EPSON;CMD:ESCPL2,BDC,D4;MDL:L3250 Series;CLS:PRINTER;"
        info = parse_device_id(len(text).to_bytes(2, "big") + text)
        self.assertEqual(info["MFG"], "EPSON")
        self.assertEqual(info["MDL"], "L3250 Series")
        long_names = b"MANUFACTURER:Canon;COMMAND SET:IVEC;MODEL:G2060 series;"
        info = parse_device_id((len(long_names) + 2).to_bytes(2, "big") + long_names)
        self.assertEqual((info["MFG"], info["MDL"], info["CMD"]),
                         ("Canon", "G2060 series", "IVEC"))

    def test_windows_path_opens_epson_over_d4(self):
        """Simula o Windows: usbprint.sys com uma Canon e uma Epson ligadas."""
        from unittest import mock
        from ecoreset import usb, winusb
        fake = FakePrinter(name="EPSON L3250 Series", read_key=(74, 54),
                           eeprom={48: 151, 49: 13})
        fake.close()

        class FakeUsbPrint(FakeD4Device):
            def __init__(self, path):
                super().__init__(fake, silent=(path == "canon"))
                self.ids = {"canon": {"MFG": "Canon", "MDL": "G2060 series"},
                            "epson": {"MFG": "EPSON", "MDL": "L3250 Series"}}[path]
                self.closed = False

            def device_id(self):
                return self.ids

            def close(self):
                self.closed = True

        with mock.patch.object(usb.sys, "platform", "win32"), \
                mock.patch.object(winusb, "device_paths", lambda: ["canon", "epson"]), \
                mock.patch.object(winusb, "UsbPrintDevice", FakeUsbPrint):
            client = usb.UsbCtrlClient().open()
            p = EpsonPrinter("USB", client=client)
            self.assertEqual(p.detect(), ("L3250 Series", "ET-2812"))
            self.assertEqual(p.read_eeprom(49), 13)
            client.close()


class TestEepromImage(unittest.TestCase):
    def setUp(self):
        self.fake = FakePrinter(name="EPSON L3250 Series", read_key=(74, 54),
                                eeprom={48: 5, 49: 6, 1604: ord("X"), 0x780: 0xAA})

    def tearDown(self):
        self.fake.close()

    def test_json_image_plan_protects_serial_and_mac(self):
        from ecoreset.printer import load_image
        p = EpsonPrinter("127.0.0.1", port=self.fake.port, timeout=0.5, retries=0,
                         model="ET-2812")
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "img.json")
            with open(path, "w") as f:
                json.dump({"model": "ET-2812",
                           "eeprom": {"48": 0, "49": 6, "1604": ord("Z"), "1920": 0}}, f)
            image, model = load_image(path)
            changes, skipped = p.plan_image(image)
            self.assertEqual(model, "ET-2812")
            self.assertEqual(changes, [(48, 5, 0)])
            self.assertEqual(skipped, [1604, 1920])
            backup = os.path.join(d, "b.json")
            p.write_image(changes, backup_path=backup)
            with open(backup) as f:
                self.assertEqual(json.load(f)["eeprom"], {"48": 5})
        self.assertEqual(self.fake.eeprom[48], 0)
        self.assertEqual(self.fake.eeprom[1604], ord("X"))

    def test_binary_image_with_start_address(self):
        from ecoreset.printer import load_image
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "dump.bin")
            with open(path, "wb") as f:
                f.write(bytes([1, 2, 3]))
            image, model = load_image(path, start=47)
        self.assertEqual(image, {47: 1, 48: 2, 49: 3})
        self.assertIsNone(model)


class TestMaintenance(unittest.TestCase):
    def test_jobs(self):
        job = maintenance.nozzle_check_job()
        self.assertIn(b"\x1b(R\x08\x00\x00REMOTE1NC\x02\x00\x00\x00\x1b\x00\x00\x00", job)
        self.assertTrue(job.startswith(maintenance.EXIT_PACKET_MODE))
        self.assertIn(b"REMOTE1CH\x02\x00\x00\x00", maintenance.head_cleaning_job())

    def test_send_raw_network(self):
        server = socket.socket()
        server.bind(("127.0.0.1", 0))
        server.listen(1)
        got = []

        def accept():
            conn, _ = server.accept()
            with conn:
                while True:
                    chunk = conn.recv(4096)
                    if not chunk:
                        break
                    got.append(chunk)

        t = threading.Thread(target=accept)
        t.start()
        maintenance.send_raw_network("127.0.0.1", b"abc", port=server.getsockname()[1])
        t.join(2)
        server.close()
        self.assertEqual(b"".join(got), b"abc")


if __name__ == "__main__":
    unittest.main()
