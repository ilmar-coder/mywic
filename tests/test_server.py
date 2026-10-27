import json
import os
import tempfile
import threading
import unittest
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

from ecoreset import library, models, server, wiclog
from tests.test_wiclog import TRIAL, make_log

TOKEN = "chave-de-teste-1234567890"


class FakeApi(BaseHTTPRequestHandler):
    """Imita as rotas /api/ecoreset do site."""
    store = {}
    online = True

    def log_message(self, *args):
        pass

    def _reply(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _handle(self):
        if not self.online:
            return self._reply(503, {"error": "fora do ar"})
        if self.headers.get("X-EcoReset-Token") != TOKEN:
            return self._reply(401, {"error": "chave do EcoReset inválida"})
        prefix = "/api/ecoreset/models"
        name = urllib.parse.unquote(self.path[len(prefix) + 1:])
        if self.command == "GET":
            return self._reply(200, {"modelos": self.store})
        if self.command == "PUT":
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            body["updated_at"] = "2026-10-27T10:00:00Z"
            self.store[name] = body
            return self._reply(200, {"ok": True})
        if name not in self.store:
            return self._reply(404, {"error": "modelo não encontrado"})
        del self.store[name]
        return self._reply(200, {"ok": True})

    do_GET = do_PUT = do_DELETE = _handle


class TestServerSync(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.old_env = os.environ.get("ECORESET_PASTA")
        self.old_config = library.CONFIG
        os.environ["ECORESET_PASTA"] = os.path.join(self.tmp.name, "maquina1")
        library.CONFIG = os.path.join(self.tmp.name, "config.json")
        FakeApi.store, FakeApi.online = {}, True
        self.httpd = HTTPServer(("127.0.0.1", 0), FakeApi)
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.url = "http://127.0.0.1:%d" % self.httpd.server_port
        server.configure(self.url, TOKEN)

    def tearDown(self):
        self.httpd.shutdown()
        self.httpd.server_close()
        library.CONFIG = self.old_config
        if self.old_env is None:
            os.environ.pop("ECORESET_PASTA", None)
        else:
            os.environ["ECORESET_PASTA"] = self.old_env
        models.reload_learned()
        self.tmp.cleanup()

    def learn(self):
        name, cfg, _ = wiclog.learn(make_log(TRIAL))
        return name, cfg

    def test_publish_then_other_machine_updates(self):
        name, cfg = self.learn()
        self.assertIsNone(server.publish(name, cfg))
        self.assertEqual(FakeApi.store[name]["write_key"], "Testkey1")
        self.assertNotIn("pendente", FakeApi.store[name])
        os.environ["ECORESET_PASTA"] = os.path.join(self.tmp.name, "maquina2")
        self.assertEqual(server.sync(), (0, 1, ["XP-9999"]))
        self.assertEqual(models.reload_learned(), ["XP-9999"])
        self.assertEqual(models.MODELS["XP-9999"]["raw_waste_reset"][60], 94)

    def test_offline_save_is_sent_on_next_update(self):
        FakeApi.online = False
        name, cfg = self.learn()
        self.assertIn("503", server.publish(name, cfg))
        self.assertTrue(library.load()[name]["pendente"])
        FakeApi.online = True
        self.assertEqual(server.sync(), (1, 1, []))
        self.assertIn(name, FakeApi.store)
        self.assertNotIn("pendente", library.load()[name])

    def test_delete_reaches_other_machines(self):
        name, cfg = self.learn()
        server.publish(name, cfg)
        self.assertTrue(server.remove(name))
        self.assertEqual(FakeApi.store, {})
        os.environ["ECORESET_PASTA"] = os.path.join(self.tmp.name, "maquina2")
        library.save(name, cfg)  # cópia antiga, já enviada antes
        server.sync()
        self.assertEqual(library.load(), {})

    def test_wrong_token_and_plain_http_are_refused(self):
        server.configure(self.url, "errada")
        with self.assertRaisesRegex(server.ServerError, "401"):
            server.sync()
        with self.assertRaisesRegex(server.ServerError, "https"):
            server.configure("http://download.exemplo.com", TOKEN)

    def test_config_keeps_folder_and_is_private(self):
        library.set_folder(os.path.join(self.tmp.name, "pasta"))
        self.assertEqual(server.settings(), (self.url, TOKEN))
        if os.name != "nt":
            self.assertEqual(os.stat(library.CONFIG).st_mode & 0o777, 0o600)


if __name__ == "__main__":
    unittest.main()
