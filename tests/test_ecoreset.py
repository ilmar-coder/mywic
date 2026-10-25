import json
import os
import socket
import tempfile
import threading
import unittest

from ecoreset import maintenance, snmp
from ecoreset.models import MODELS, find_model
from ecoreset.printer import (EpsonPrinter, PrinterError, read_oid, shift_key,
                              waste_percent, write_oid)
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
