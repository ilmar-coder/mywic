import os
import tempfile
import unittest

from ecoreset import library, models, wiclog
from ecoreset.models import MODELS, find_model
from ecoreset.printer import EpsonPrinter, write_oid
from tests.fake_printer import FakePrinter


def _hex_line(n, read_key, write_key, address, value):
    oid = write_oid(read_key, write_key, address, value)
    data = bytes(int(x) for x in oid.split(".")[15:])
    return "INFO RESET_DATA RESET DATA: %d - %s REAL" % (n, data.hex(" ").upper())


def make_log(writes, read_key=(11, 22), write_key=b"Testkey1", model="XP-9999",
             decimal_line=None):
    lines = ["INFO RESET_GUID RESET GUID: %s 123456 KEY trial" % model,
             "INFO Reset started. Do not turn off the printer"]
    lines += [_hex_line(i, read_key, write_key, a, v) for i, (a, v) in enumerate(writes)]
    if decimal_line:
        lines.append("DEBUG snmp get " + write_oid(read_key, write_key, *decimal_line))
    lines.append("INFO Reset complete")
    return "\n".join(lines)


# reset trial de um modelo desconhecido: 2 contadores em 80%, reais zerados,
# níveis de manutenção e uma flag em 0x100
TRIAL = [(20, 104), (21, 61), (22, 68), (23, 16), (24, 0), (25, 0), (59, 0),
         (60, 94), (61, 94), (0x100, 0x3E)]


class LibraryCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.old = os.environ.get("ECORESET_PASTA")
        os.environ["ECORESET_PASTA"] = self.tmp.name
        models.reload_learned()

    def tearDown(self):
        if self.old is None:
            os.environ.pop("ECORESET_PASTA", None)
        else:
            os.environ["ECORESET_PASTA"] = self.old
        models.reload_learned()
        self.tmp.cleanup()


class TestWicLog(LibraryCase):
    def test_parse_keys_and_writes(self):
        info = wiclog.parse(make_log(TRIAL))
        self.assertEqual(info["model"], "XP-9999")
        self.assertEqual(info["read_key"], (11, 22))
        self.assertEqual(info["write_key"], b"Testkey1")
        self.assertEqual(info["writes"], TRIAL)

    def test_decimal_oid_lines_are_read(self):
        info = wiclog.parse(make_log([], decimal_line=(300, 7)))
        self.assertEqual(info["writes"], [(300, 7)])

    def test_learn_trial_zeroes_counters_and_estimates_divider(self):
        name, cfg, warnings = wiclog.learn(make_log(TRIAL))
        self.assertEqual(name, "XP-9999")
        self.assertEqual(cfg["raw_waste_reset"], {20: 0, 21: 0, 22: 0, 23: 0, 24: 0,
                                                  25: 0, 59: 0, 60: 94, 61: 94})
        self.assertEqual(cfg["main_waste"],
                         {"oids": [20, 21], "divider": round(0x3D68 / 80, 2)})
        self.assertEqual(cfg["borderless_waste"]["oids"], [22, 23])
        self.assertTrue(any("0x100" in w for w in warnings))

    def test_learn_same_keys_as_listed_model_copies_it(self):
        writes = [(a, 94 if v == 94 else 7) for a, v in
                  MODELS["L3150"]["raw_waste_reset"].items()]
        name, cfg, warnings = wiclog.learn(make_log(
            writes, read_key=(151, 7), write_key=b"Maribaya", model="L3110"))
        self.assertEqual(cfg["main_waste"], MODELS["L3150"]["main_waste"])
        self.assertTrue(any("L3150" in w for w in warnings))

    def test_learn_same_addresses_other_keys_estimates_divider(self):
        writes = [(a, 94 if v == 94 else 0) for a, v in
                  MODELS["L3150"]["raw_waste_reset"].items()]
        writes += [(48, 0x40), (49, 0x1F)]
        name, cfg, warnings = wiclog.learn(make_log(writes, model="L9999"))
        self.assertEqual(cfg["main_waste"]["oids"], [48, 49, 47])
        self.assertEqual(cfg["main_waste"]["divider"], round(0x1F40 / 80, 2))
        self.assertNotEqual(MODELS["L3150"]["main_waste"]["divider"],
                            cfg["main_waste"]["divider"])  # a lista não mudou

    def test_log_without_reset_is_refused(self):
        with self.assertRaisesRegex(wiclog.WicLogError, "nenhuma gravação"):
            wiclog.learn("INFO nada aqui\n7C 7C 07 00 0B 16 41 BE A0 30 00 00")

    def test_two_printers_in_one_log_are_refused(self):
        text = make_log(TRIAL[:2]) + "\n" + make_log(TRIAL[:2], read_key=(1, 2))
        with self.assertRaisesRegex(wiclog.WicLogError, "mais de uma"):
            wiclog.parse(text)


class TestLibrary(LibraryCase):
    def test_save_reload_and_detect(self):
        name, cfg, _ = wiclog.learn(make_log(TRIAL))
        library.save(name, cfg)
        self.assertEqual(models.reload_learned(), ["XP-9999"])
        self.assertEqual(MODELS["XP-9999"]["write_key"], b"Testkey1")
        self.assertEqual(MODELS["XP-9999"]["raw_waste_reset"][60], 94)
        self.assertEqual(find_model("EPSON XP-9999 Series"), "XP-9999")
        self.assertTrue(library.remove("XP-9999"))
        self.assertEqual(models.reload_learned(), [])
        self.assertNotIn("XP-9999", MODELS)

    def test_merge_from_other_machine(self):
        other = os.path.join(self.tmp.name, "outra", "modelos.json")
        name, cfg, _ = wiclog.learn(make_log(TRIAL))
        library.save(name, cfg, path=other)
        self.assertEqual(library.merge_file(other), ["XP-9999"])
        self.assertEqual(library.merge_file(other), [])
        self.assertIn("XP-9999", library.load())

    def test_learned_model_resets_printer(self):
        name, cfg, _ = wiclog.learn(make_log(TRIAL))
        library.save(name, cfg)
        models.reload_learned()
        fake = FakePrinter(name="EPSON XP-9999 Series", read_key=(11, 22),
                           write_key=b"Testkey1",
                           eeprom={20: 0x10, 21: 0x50, 22: 3, 23: 9, 60: 50})
        try:
            p = EpsonPrinter("127.0.0.1", port=fake.port, timeout=0.5, retries=0)
            self.assertEqual(p.detect()[1], "XP-9999")
            self.assertEqual(p.waste_levels()["principal"],
                             round(0x5010 / cfg["main_waste"]["divider"], 2))
            p.reset_waste()
            self.assertEqual(p.waste_levels()["principal"], 0)
            self.assertEqual(fake.eeprom[60], 94)
        finally:
            fake.close()


if __name__ == "__main__":
    unittest.main()
