"""Interface gráfica (tkinter)."""

import datetime
import json
import queue
import threading
import tkinter as tk
from tkinter import filedialog, messagebox, simpledialog, ttk

from . import (__version__, canon, ipp, keyfinder, library, maintenance, server, snapshot,
               wiclog)
from .errors import explain_epson_error, search
from .models import MODELS, model_names, reload_learned
from .printer import EpsonPrinter, PrinterError, load_image
from .d4 import D4Error
from .ivec import MAINTENANCE, CanonUsb, IvecError
from .snmp import SnmpError
from .usb import UsbCtrlClient, UsbError

ERRORS = (wiclog.WicLogError, server.ServerError, PrinterError, SnmpError, UsbError, D4Error, ipp.IppError, IvecError, OSError,
          ValueError)
# Mac/Linux pelo libusb; Windows pelo usbprint.sys (winusb.py).
USB_SUPPORTED = True


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("EcoReset %s" % __version__)
        self.minsize(560, 460)
        self.results = queue.Queue()
        self.busy = False
        self.progress = None
        self._build()
        self.after(100, self._poll)

    # ---- layout -----------------------------------------------------------

    def _build(self):
        top = ttk.LabelFrame(self, text="Impressora")
        top.pack(fill="x", padx=10, pady=(10, 5))

        self.conn = tk.StringVar(value="usb" if USB_SUPPORTED else "rede")
        ttk.Radiobutton(top, text="Cabo USB", variable=self.conn, value="usb",
                        state="normal" if USB_SUPPORTED else "disabled").grid(
            row=0, column=0, sticky="w", padx=5, pady=5)
        ttk.Radiobutton(top, text="Rede, IP:", variable=self.conn, value="rede").grid(
            row=0, column=1, sticky="w")
        self.host = tk.StringVar()
        ttk.Entry(top, textvariable=self.host, width=16).grid(row=0, column=2, sticky="w")
        ttk.Button(top, text="Detectar", command=self.detect).grid(row=0, column=3, padx=5)

        ttk.Label(top, text="Modelo:").grid(row=0, column=4, sticky="w", padx=(10, 5))
        self.model = tk.StringVar()
        self.model_box = ttk.Combobox(top, textvariable=self.model, values=model_names(),
                                      state="readonly", width=10)
        self.model_box.grid(row=0, column=5, sticky="w")

        ttk.Label(top, text="Marca:").grid(row=1, column=0, sticky="w", padx=5)
        self.brand = tk.StringVar(value="epson")
        brands = ttk.Frame(top)
        brands.grid(row=1, column=1, columnspan=5, sticky="w")
        ttk.Radiobutton(brands, text="Epson", variable=self.brand, value="epson",
                        command=self._brand_changed).pack(side="left")
        ttk.Radiobutton(brands, text="Canon", variable=self.brand, value="canon",
                        command=self._brand_changed).pack(side="left", padx=10)

        self.info = tk.StringVar(value=(
            "Escolha a conexão e clique em Detectar. Pelo USB funciona até em "
            "firmwares que bloqueiam a rede (como a L3250)."))
        ttk.Label(top, textvariable=self.info, foreground="#555", wraplength=620).grid(
            row=2, column=0, columnspan=6, sticky="w", padx=5, pady=(0, 5))

        tabs = ttk.Notebook(self)
        tabs.pack(fill="both", expand=True, padx=10, pady=5)
        tabs.add(self._tab_diagnose(tabs), text="Diagnóstico")
        tabs.add(self._tab_waste(tabs), text="Almofada de tinta")
        tabs.add(self._tab_maintenance(tabs), text="Manutenção")
        tabs.add(self._tab_eeprom(tabs), text="EEPROM")
        tabs.add(self._tab_library(tabs), text="Modelos aprendidos")

        self.status = tk.StringVar(value="Pronto.")
        ttk.Label(self, textvariable=self.status, relief="sunken", anchor="w").pack(
            fill="x", side="bottom")

    def _brand_changed(self):
        if self.brand.get() == "canon":
            self.model.set("")
            self.info.set(
                "Canon: diagnóstico (códigos de erro) e manutenção. O contador da "
                "almofada da Canon só zera com a ferramenta de serviço da marca; "
                "modelos com caixa de manutenção (G2060...) só precisam trocar a caixa.")
            names = [n for n in maintenance.installed_printers() if "CANON" in n.upper()]
        else:
            self.info.set("Escolha a conexão e clique em Detectar.")
            names = [n for n in maintenance.installed_printers() if "EPSON" in n.upper()]
        if names:
            self.winprinter.set(names[0])

    def _tab_diagnose(self, parent):
        f = ttk.Frame(parent, padding=10)
        top = ttk.Frame(f)
        top.grid(row=0, column=0, sticky="w")
        ttk.Button(top, text="Ler erros da impressora", command=self.diagnose).pack(
            side="left")
        ttk.Label(top, text="   Consultar código:").pack(side="left")
        self.code = tk.StringVar()
        entry = ttk.Entry(top, textvariable=self.code, width=10)
        entry.pack(side="left", padx=5)
        entry.bind("<Return>", lambda _: self.lookup_code())
        ttk.Button(top, text="Buscar", command=self.lookup_code).pack(side="left")
        self.diag_out = tk.Text(f, height=16, width=72, wrap="word", font=("Helvetica", 11))
        self.diag_out.grid(row=1, column=0, sticky="nsew", pady=(8, 0))
        f.rowconfigure(1, weight=1)
        f.columnconfigure(0, weight=1)
        ttk.Label(f, foreground="#555", wraplength=620, text=(
            "Epson: lê pelo USB ou pela rede. Canon: pela rede (IP) ou pela impressora "
            "instalada no computador escolhida na aba Manutenção. Também dá para "
            "digitar um código (ex.: 1300, 5B00, 0x10) ou uma palavra (ex.: papel).")).grid(
            row=2, column=0, sticky="w", pady=(5, 0))
        return f

    def _tab_waste(self, parent):
        f = ttk.Frame(parent, padding=10)
        self.bars = {}
        for i, label in enumerate(("principal", "sem margem", "terceira")):
            ttk.Label(f, text="Almofada %s:" % label).grid(row=i, column=0, sticky="w", pady=6)
            bar = ttk.Progressbar(f, length=280, maximum=100)
            bar.grid(row=i, column=1, padx=10)
            text = tk.StringVar(value="—")
            ttk.Label(f, textvariable=text, width=9).grid(row=i, column=2, sticky="w")
            self.bars[label] = (bar, text)
        buttons = ttk.Frame(f)
        buttons.grid(row=3, column=0, columnspan=3, pady=(15, 5), sticky="w")
        ttk.Button(buttons, text="Ler contadores", command=self.read_levels).pack(side="left")
        ttk.Button(buttons, text="Resetar almofada", command=self.reset).pack(
            side="left", padx=10)
        ttk.Button(buttons, text="Restaurar backup", command=self.restore).pack(side="left")
        ttk.Button(f, text="Reset temporário (firmware bloqueado)",
                   command=self.temp_reset).grid(row=4, column=0, columnspan=3,
                                                 pady=(0, 10), sticky="w")
        ttk.Label(f, wraplength=480, foreground="#555", text=(
            "O reset só zera o contador. Se a almofada estiver encharcada, troque-a "
            "ou instale um reservatório externo, senão a tinta pode vazar. Um backup "
            "dos valores antigos é salvo antes de gravar. Se a impressora recusar "
            "(L3250 e firmwares novos), use o reset temporário: ele libera a "
            "impressora usando só o número de série.")).grid(
            row=5, column=0, columnspan=3, sticky="w")
        return f

    def _tab_maintenance(self, parent):
        f = ttk.Frame(parent, padding=10)
        self.via = tk.StringVar(value="rede")
        ttk.Radiobutton(f, text="Pela rede (usa o IP acima)", variable=self.via,
                        value="rede").grid(row=0, column=0, columnspan=2, sticky="w")
        ttk.Radiobutton(f, text="Impressora instalada no computador (USB ou rede):",
                        variable=self.via, value="instalada").grid(
            row=1, column=0, sticky="w")
        self.winprinter = tk.StringVar()
        names = maintenance.installed_printers()
        if names:
            epsons = [n for n in names if "EPSON" in n.upper()]
            self.winprinter.set((epsons or names)[0])
        ttk.Combobox(f, textvariable=self.winprinter, values=names, width=30,
                     state="readonly").grid(row=1, column=1, padx=5, sticky="w")
        buttons = ttk.Frame(f)
        buttons.grid(row=2, column=0, columnspan=2, pady=15, sticky="w")
        ttk.Button(buttons, text="Teste de bicos", command=lambda: self.maintain(
            "nozzle")).pack(side="left")
        ttk.Button(buttons, text="Limpeza de cabeça", command=lambda: self.maintain(
            "clean")).pack(side="left", padx=10)
        canon_row = ttk.Frame(f)
        canon_row.grid(row=3, column=0, columnspan=2, pady=(0, 10), sticky="w")
        ttk.Label(canon_row, text="Canon pelo cabo USB (IVEC):").pack(side="left")
        self.ivec_op = tk.StringVar(value="limpeza")
        ttk.Combobox(canon_row, textvariable=self.ivec_op, values=list(MAINTENANCE),
                     state="readonly", width=16).pack(side="left", padx=5)
        ttk.Button(canon_row, text="Executar", command=self.maintain_ivec).pack(side="left")
        ttk.Label(f, wraplength=480, foreground="#555", text=(
            "Coloque papel A4 antes do teste de bicos. Cada limpeza gasta tinta e "
            "enche a almofada/caixa de manutenção: faça no máximo duas seguidas. "
            "Limpeza profunda e ink-flush gastam muita tinta. Na Canon, o programa "
            "não envia nada se a impressora estiver com código de erro.")).grid(
            row=4, column=0, columnspan=2, sticky="w")
        return f

    def _tab_eeprom(self, parent):
        f = ttk.Frame(parent, padding=10)
        self.addr = tk.StringVar(value="0")
        self.addr_end = tk.StringVar(value="")
        self.value = tk.StringVar()
        ttk.Label(f, text="Endereço:").grid(row=0, column=0, sticky="w")
        ttk.Entry(f, textvariable=self.addr, width=8).grid(row=0, column=1, sticky="w")
        ttk.Label(f, text="até:").grid(row=0, column=2, padx=(10, 0))
        ttk.Entry(f, textvariable=self.addr_end, width=8).grid(row=0, column=3, sticky="w")
        ttk.Label(f, text="Valor:").grid(row=0, column=4, padx=(10, 0))
        ttk.Entry(f, textvariable=self.value, width=6).grid(row=0, column=5, sticky="w")
        buttons = ttk.Frame(f)
        buttons.grid(row=1, column=0, columnspan=6, pady=8, sticky="w")
        ttk.Button(buttons, text="Ler", command=self.ee_read).pack(side="left")
        ttk.Button(buttons, text="Gravar", command=self.ee_write).pack(side="left", padx=10)
        ttk.Button(buttons, text="Salvar leitura…", command=self.ee_save).pack(side="left")
        ttk.Button(buttons, text="Regravar de arquivo…", command=self.ee_image).pack(
            side="left", padx=10)
        study = ttk.Frame(f)
        study.grid(row=2, column=0, columnspan=6, pady=(0, 8), sticky="w")
        ttk.Button(study, text="Leitura completa…", command=self.snapshot_take).pack(
            side="left")
        ttk.Button(study, text="Comparar leituras…", command=self.snapshot_compare).pack(
            side="left", padx=10)
        ttk.Label(study, text="Chave de leitura (modelo fora da lista):").pack(side="left")
        self.manual_key = tk.StringVar()
        ttk.Entry(study, textvariable=self.manual_key, width=8).pack(side="left", padx=5)
        self.find_key_text = tk.StringVar(value="Descobrir chave…")
        ttk.Button(study, textvariable=self.find_key_text, command=self.find_key).pack(
            side="left")
        self.cancel_search = None
        self.ee_out = tk.Text(f, height=12, width=60, font=("Consolas", 10))
        self.ee_out.grid(row=3, column=0, columnspan=6, sticky="nsew")
        f.rowconfigure(3, weight=1)
        f.columnconfigure(5, weight=1)
        ttk.Label(f, foreground="#a00", wraplength=620, text=(
            "Avançado: gravar endereços errados pode travar a impressora. Para "
            "descobrir o reset de um modelo novo: faça uma leitura completa, zere a "
            "impressora com outra ferramenta, faça outra leitura e compare.")).grid(
            row=4, column=0, columnspan=6, sticky="w", pady=(5, 0))
        self.last_dump = {}
        return f

    def _tab_library(self, parent):
        f = ttk.Frame(parent, padding=10)
        self.lib_folder = tk.StringVar()
        self.lib_server = tk.StringVar()
        ttk.Label(f, text="Servidor:").grid(row=0, column=0, sticky="w")
        ttk.Label(f, textvariable=self.lib_server, foreground="#555").grid(
            row=0, column=1, sticky="w")
        ttk.Label(f, text="Cópia local:").grid(row=1, column=0, sticky="w")
        ttk.Label(f, textvariable=self.lib_folder, foreground="#555").grid(
            row=1, column=1, sticky="w")
        buttons = ttk.Frame(f)
        buttons.grid(row=2, column=0, columnspan=2, pady=8, sticky="w")
        ttk.Button(buttons, text="Atualizar", command=self.sync_library).pack(side="left")
        ttk.Button(buttons, text="Importar log do WICReset…",
                   command=self.import_wic_log).pack(side="left", padx=10)
        ttk.Button(buttons, text="Apagar", command=self.remove_learned).pack(side="left")
        ttk.Button(buttons, text="Servidor…", command=self.choose_server).pack(
            side="left", padx=10)
        ttk.Button(buttons, text="Pasta…", command=self.choose_folder).pack(side="left")
        ttk.Button(buttons, text="Juntar arquivo…", command=self.merge_library).pack(
            side="left", padx=10)
        self.lib_list = ttk.Treeview(f, columns=("chave", "maquina", "data"), height=8)
        for col, text, width in (("#0", "Modelo", 120), ("chave", "Chave de leitura", 110),
                                 ("maquina", "Aprendido em", 150), ("data", "Data", 170)):
            self.lib_list.heading(col, text=text)
            self.lib_list.column(col, width=width)
        self.lib_list.grid(row=3, column=0, columnspan=2, sticky="nsew")
        f.rowconfigure(3, weight=1)
        f.columnconfigure(1, weight=1)
        ttk.Label(f, foreground="#555", wraplength=620, text=(
            "Faça um reset (pode ser o trial gratuito) no WICReset e importe o "
            "application.log dele: o EcoReset aprende as chaves e o reset do modelo "
            "e passa a zerar sozinho pelo cabo USB. Com o servidor configurado, o "
            "modelo vai para o banco do servidor e as outras máquinas o recebem ao "
            "clicar em Atualizar (o EcoReset também atualiza ao abrir).")).grid(
            row=4, column=0, columnspan=2, sticky="w", pady=(5, 0))
        self.refresh_library()
        if server.configured():
            self.after(500, lambda: self.sync_library(quiet=True))
        return f

    def refresh_library(self):
        learned = self.guarded(reload_learned) or []
        self.lib_folder.set(library.db_path())
        url, token = server.settings()
        self.lib_server.set(url if token else "não configurado (clique em Servidor…)")
        self.model_box["values"] = model_names()
        self.lib_list.delete(*self.lib_list.get_children())
        for name in learned:
            cfg = MODELS[name]
            self.lib_list.insert("", "end", iid=name, text=name, values=(
                "%d,%d" % cfg["read_key"], cfg.get("learned_on", ""),
                cfg.get("learned_at", "") + ("  (não enviado)" if cfg.get("pendente")
                                             else "")))

    def sync_library(self, quiet=False):
        if not server.configured():
            if not quiet:
                self.choose_server()
            return
        if self.busy:
            return

        def done(result):
            self.refresh_library()
            sent, total, new = result
            self.status.set("Banco atualizado: %d modelo(s) no servidor%s%s." % (
                total, ", %d enviado(s)" % sent if sent else "",
                ", novos: " + ", ".join(new) if new else ""))

        if quiet:  # ao abrir: sem janela de erro se estiver sem internet
            def work():
                try:
                    return server.sync()
                except server.ServerError as e:
                    return e
            self.run("Atualizando o banco", work, lambda r: done(r) if isinstance(
                r, tuple) else self.status.set("Banco não atualizado: %s" % r))
        else:
            self.run("Atualizando o banco", server.sync, done)

    def choose_server(self):
        url, _ = server.settings()
        url = simpledialog.askstring("EcoReset", "Endereço do servidor:", initialvalue=url)
        if not url:
            return
        token = simpledialog.askstring("EcoReset", "Chave do EcoReset no servidor:",
                                       show="*")
        if not token:
            return
        if self.guarded(lambda: server.configure(url, token) or True):
            self.refresh_library()
            self.sync_library()

    def import_wic_log(self):
        path = filedialog.askopenfilename(title="application.log do WICReset",
                                          filetypes=[("Log", "*.log *.txt"), ("Todos", "*")])
        if not path:
            return
        learned = self.guarded(lambda: wiclog.learn_file(path))
        if not learned:
            return
        name, cfg, warnings = learned
        name = simpledialog.askstring("EcoReset", "Nome do modelo:", initialvalue=name)
        if not name:
            return
        name = name.strip().upper()
        lines = ["Chave de leitura: %d,%d" % cfg["read_key"],
                 "Chave de gravação: %s" % cfg["write_key"].decode("latin-1"),
                 "Reset grava %d endereços" % len(cfg["raw_waste_reset"])]
        lines += ["Atenção: %s" % w for w in warnings]
        if name in MODELS and not MODELS[name].get("learned_from"):
            lines.append("Este nome já está na lista do app: o aprendido vai valer "
                         "no lugar dele.")
        if not messagebox.askyesno("EcoReset", "\n".join(lines) + "\n\nSalvar no banco?"):
            return
        problem = self.guarded(lambda: server.publish(name, cfg) or "")
        if problem is None:
            return
        self.refresh_library()
        self.model.set(name)
        if problem:
            messagebox.showwarning("EcoReset", "Salvo nesta máquina, mas não foi ao "
                                   "servidor: %s" % problem)

    def choose_folder(self):
        path = filedialog.askdirectory(title="Pasta do banco de modelos")
        if path and self.guarded(lambda: library.set_folder(path)):
            self.refresh_library()

    def merge_library(self):
        path = filedialog.askopenfilename(title="modelos.json de outra máquina",
                                          filetypes=[("Banco", "*.json")])
        if not path:
            return
        new = self.guarded(lambda: library.merge_file(path))
        if new is not None:
            self.refresh_library()
            messagebox.showinfo("EcoReset", "Novos: %s" % (", ".join(new) or "nenhum"))

    def remove_learned(self):
        for name in self.lib_list.selection():
            if messagebox.askyesno("EcoReset", "Apagar %s do banco?" % name):
                self.guarded(lambda: server.remove(name))
        self.refresh_library()

    # ---- execução em segundo plano -------------------------------------------

    def run(self, label, work, done):
        if self.busy:
            return
        self.busy = True
        self.status.set(label + "…")

        def target():
            try:
                self.results.put((done, work(), None))
            except ERRORS as e:
                self.results.put((done, None, e))

        threading.Thread(target=target, daemon=True).start()

    def _poll(self):
        try:
            while True:
                done, result, error = self.results.get_nowait()
                self.busy = False
                self.progress = None
                self.cancel_search = None
                self.find_key_text.set("Descobrir chave…")
                if error:
                    self.status.set("Erro.")
                    messagebox.showerror("EcoReset", str(error))
                else:
                    self.status.set("Pronto.")
                    done(result)
        except queue.Empty:
            pass
        if self.busy and self.progress:
            self.status.set(self.progress)
        self.after(100, self._poll)

    def _manual_key(self):
        text = self.manual_key.get().strip()
        if not text:
            return None
        parts = [p for p in text.replace(";", ",").replace(" ", ",").split(",") if p]
        try:
            key = tuple(int(p, 0) for p in parts)
        except ValueError:
            key = ()
        if len(key) != 2 or not all(0 <= k <= 255 for k in key):
            raise ValueError("chave de leitura inválida: use dois números, ex.: 74,54")
        return key

    # ---- leitura completa e comparação --------------------------------------

    def snapshot_take(self):
        label = simpledialog.askstring(
            "EcoReset", "Nome desta leitura (ex.: antes do reset):", parent=self,
            initialvalue="antes")
        if label is None:
            return
        end = simpledialog.askinteger(
            "EcoReset", "Ler do endereço 0 até (2047 cobre os modelos conhecidos):",
            initialvalue=snapshot.DEFAULT_RANGE[1], minvalue=0, maxvalue=0xFFFF, parent=self)
        if end is None:
            return
        path = filedialog.asksaveasfilename(
            defaultextension=".json", filetypes=[("Leitura", "*.json")],
            initialfile="leitura-%s-%s.json" % (
                label.replace(" ", "-") or "eeprom",
                datetime.datetime.now().strftime("%Y%m%d-%H%M%S")))
        if not path:
            return

        def progress(i, n):
            self.progress = "Lendo EEPROM… %d de %d" % (i, n)

        def take(p):
            snap = snapshot.take(p, 0, end, label=label, progress=progress)
            snapshot.save(snap, path)
            return snap
        work = self.guarded(lambda: self.session(take))
        if not work:
            return
        self.progress = None

        def done(snap):
            self.progress = None
            self.ee_out.delete("1.0", "end")
            self.ee_out.insert("end", "Leitura '%s' salva em %s\n%d endereços lidos, %d "
                               "recusados.\nModelo: %s  Série: %s\n" % (
                                   label, path, len(snap["eeprom"]),
                                   len(snap["sem_leitura"]), snap["model_name"] or "?",
                                   snap["serial"] or "?"))
        self.run("Lendo EEPROM", work, done)

    def find_key(self):
        if self.cancel_search is not None:  # o botão vira "Parar" durante a busca
            self.cancel_search.set()
            return
        if self.busy:
            return
        if not messagebox.askyesno("EcoReset", (
                "Procurar a chave de leitura desta Epson?\n\nSó faz leituras, não grava "
                "nada. As chaves conhecidas vão primeiro; se não for nenhuma, testa "
                "todas as 65.536 combinações, o que pode levar de minutos a mais de "
                "uma hora. Dá para parar a qualquer momento.")):
            return
        cancel = threading.Event()

        def progress(i, n, key):
            self.progress = "Procurando chave… %d de %d (testando %d,%d)" % (
                i, n, key[0], key[1])

        def search(p):
            if p.model:
                raise PrinterError("este modelo (%s) já está na lista e tem chave "
                                   "conhecida" % p.model)
            return keyfinder.find_read_key(p, progress=progress, cancel=cancel)
        work = self.guarded(lambda: self.session(search))
        if not work:
            return
        self.cancel_search = cancel
        self.find_key_text.set("Parar busca")
        self.run("Procurando chave", work, self._key_found)

    def _key_found(self, result):
        out = self.ee_out
        out.delete("1.0", "end")
        if result is None:
            return out.insert("end", "Busca interrompida.\n")
        key = result["read_key"]
        self.manual_key.set("%d,%d" % key)
        out.insert("end", "Chave de leitura encontrada: %d,%d (em %d tentativas)\n"
                   % (key[0], key[1], result["tentativas"]))
        if result["conhecida"]:
            out.insert("end", "É a chave de um modelo já conhecido: provavelmente da "
                       "mesma família.\n")
        if result["aceita_qualquer"]:
            out.insert("end", "Atenção: a impressora aceitou também outra chave. Ela "
                       "parece não conferir a chave de leitura.\n")
        elif not result["confirmada"]:
            out.insert("end", "Atenção: a chave não se confirmou num segundo endereço.\n")
        out.insert("end", "A chave já foi preenchida no campo ao lado. Agora dá para "
                   "fazer a Leitura completa.\n")
        if not messagebox.askyesno("EcoReset", (
                "Chave de leitura encontrada: %d,%d.\n\nTestar também as chaves de "
                "GRAVAÇÃO conhecidas? Isso grava no endereço 0 o mesmo valor que já está "
                "lá, então não muda nada na impressora.") % key):
            return
        work = self.guarded(lambda: self.session(keyfinder.find_write_key))
        if work:
            self.run("Testando chaves de gravação", work, self._write_key_found)

    def _write_key_found(self, key):
        if key:
            self.ee_out.insert("end", "Chave de gravação aceita: %s (de um modelo "
                               "conhecido).\n" % key.decode())
        else:
            self.ee_out.insert("end", "Nenhuma chave de gravação conhecida foi aceita. "
                               "Este modelo usa uma chave nova.\n")

    def snapshot_compare(self):
        first = filedialog.askopenfilename(title="Leitura de ANTES",
                                           filetypes=[("Leitura", "*.json")])
        if not first:
            return
        second = filedialog.askopenfilename(title="Leitura de DEPOIS",
                                            filetypes=[("Leitura", "*.json")])
        if not second:
            return
        loaded = self.guarded(lambda: (snapshot.load(first), snapshot.load(second)))
        if not loaded:
            return
        before, after = loaded
        result = snapshot.compare(before, after)
        changes = result["mudancas"]
        out = self.ee_out
        out.delete("1.0", "end")
        for w in result["avisos"]:
            out.insert("end", "Atenção: %s\n" % w)
        out.insert("end", "%d endereço(s) mudaram entre '%s' e '%s':\n" % (
            len(changes), before.get("label", "antes"), after.get("label", "depois")))
        for a, old, new in changes:
            out.insert("end", "%5d (0x%04X): %3d -> %3d   %s\n" % (
                a, a, old, new, snapshot.classify(a, old, new)))
        if not changes:
            return
        model = MODELS.get(after.get("model") or "", {})
        suggestion = snapshot.suggest_model(before, after, changes,
                                            model.get("serial_number", ()))
        out.insert("end", "\nReset sugerido (%d endereços): %s\n" % (
            len(suggestion["raw_waste_reset"]), suggestion["raw_waste_reset"]))
        if messagebox.askyesno("EcoReset", "Salvar a sugestão de modelo em arquivo? "
                               "(Mande para incluirmos o modelo no app.)"):
            path = filedialog.asksaveasfilename(defaultextension=".json",
                                                initialfile="modelo-sugerido.json")
            if path:
                with open(path, "w", encoding="utf-8") as f:
                    json.dump(suggestion, f, indent=2, ensure_ascii=False)

    def where(self):
        return "USB" if self.conn.get() == "usb" else self.host.get().strip()

    def session(self, fn):
        """Trabalho que conecta (rede ou USB), roda fn(impressora) e desconecta.

        O USB fica preso só durante o trabalho, para não atrapalhar a impressão.
        """
        conn, host, model = self.conn.get(), self.host.get().strip(), self.model.get()
        if self.brand.get() == "canon":
            raise PrinterError("esta função é só para Epson. Na Canon, use as abas "
                               "Diagnóstico e Manutenção.")
        if conn == "rede" and not host:
            raise PrinterError("digite o IP da impressora")
        key = self._manual_key()

        def prepare(p):
            if model:
                p.set_model(model)
            else:
                p.detect()
            if key and not p.model:  # modelo fora da lista: só a leitura funciona
                p.read_key = key
            return fn(p)

        def work():
            if conn == "usb":
                with UsbCtrlClient() as client:
                    return prepare(EpsonPrinter("USB", client=client))
            return prepare(EpsonPrinter(host))
        return work

    def guarded(self, fn):
        try:
            return fn()
        except ERRORS as e:
            messagebox.showerror("EcoReset", str(e))

    # ---- ações ------------------------------------------------------------

    def detect(self):
        self.model.set("")
        by_usb = self.conn.get() == "usb"  # lido aqui: a busca roda fora da tela

        def identify(p):
            text = p.model_string()
            client = p.snmp
            queues = []
            if by_usb:
                queues = maintenance.queues_for_usb(
                    port=client.port, serial=client.serial,
                    model=text.upper().replace("EPSON", "").strip())
            return text, p.model, getattr(client, "port", ""), queues

        work = self.guarded(lambda: self.session(identify))
        if not work:
            return

        def done(result):
            text, name, port, queues = result
            where = self.where()
            if port:
                where += ", porta %s" % port
            if queues:
                where += ", impressora instalada \"%s\"" % queues[0]
                self.winprinter.set(queues[0])  # a mesma na aba Manutenção
                self.via.set("instalada")
            elif by_usb:
                where += ", sem impressora instalada nessa porta"
            if name:
                self.model.set(name)
                self.info.set("%s (%s) → parâmetros do %s" % (text, where, name))
            else:
                self.info.set("%s (%s): modelo não está na lista; escolha um compatível."
                              % (text or "Sem nome", where))
        self.run("Detectando", work, done)

    def read_levels(self):
        work = self.guarded(lambda: self.session(lambda p: p.waste_levels()))
        if work:
            self.run("Lendo contadores", work, self._show_levels)

    def _show_levels(self, levels):
        for label, (bar, text) in self.bars.items():
            if label in levels:
                bar["value"] = min(levels[label], 100)
                text.set("%.2f%%" % levels[label])
            else:
                bar["value"] = 0
                text.set("n/d")

    def reset(self):
        model = self.model.get()
        if not model:
            return messagebox.showerror("EcoReset", "Clique em Detectar primeiro.")
        if not messagebox.askyesno("EcoReset", (
                "Zerar os contadores da almofada da %s (%s)?\n\n"
                "Escolha onde salvar o backup a seguir.") % (model, self.where())):
            return
        path = filedialog.asksaveasfilename(
            defaultextension=".json", filetypes=[("Backup", "*.json")],
            initialfile="backup-%s-%s.json" % (
                model, datetime.datetime.now().strftime("%Y%m%d-%H%M%S")))
        if not path:
            return
        work = self.guarded(lambda: self.session(lambda p: p.reset_waste(backup_path=path)))
        if not work:
            return

        def done(_):
            messagebox.showinfo("EcoReset", "Contadores zerados.\n"
                                "Desligue a impressora e ligue de novo.")
            self.read_levels()
        self.run("Resetando", work, done)

    def temp_reset(self):
        def read_serial(p):
            try:
                return p.serial_number()
            except ERRORS:
                return ""
        work = self.guarded(lambda: self.session(read_serial))
        if not work:
            return

        def ask(serial):
            serial = simpledialog.askstring(
                "EcoReset", "Número de série da impressora (está na etiqueta "
                "e na folha de status):", initialvalue=serial or "", parent=self)
            if not serial or not serial.strip():
                return
            reset = self.guarded(lambda: self.session(
                lambda p: p.temporary_reset(serial.strip())))
            if reset:
                self.run("Reset temporário", reset, lambda _: messagebox.showinfo(
                    "EcoReset", "Impressora liberada.\nDesligue e ligue de novo."))
        self.run("Lendo número de série", work, ask)

    def restore(self):
        path = filedialog.askopenfilename(filetypes=[("Backup", "*.json")])
        if not path:
            return
        with open(path, encoding="utf-8") as f:
            backup = json.load(f)
        model = self.model.get()
        if backup.get("model") != model and not messagebox.askyesno(
                "EcoReset", "Este backup é de %s, não de %s. Continuar?"
                % (backup.get("model"), model or "?")):
            return
        work = self.guarded(lambda: self.session(lambda p: p.restore(backup)))
        if work:
            self.run("Restaurando", work,
                     lambda _: messagebox.showinfo("EcoReset", "Backup restaurado."))

    def _show_problems(self, lines, problems):
        out = self.diag_out
        out.delete("1.0", "end")
        for line in lines:
            out.insert("end", line + "\n")
        if not problems:
            out.insert("end", "\nNenhum erro encontrado.\n")
        for p in problems:
            out.insert("end", "\n%s %s: %s\n" % (p.brand, p.code, p.title))
            out.insert("end", "Causa: %s\n" % p.cause)
            out.insert("end", "Correção: %s\n" % p.fix)
            if p.service:
                out.insert("end", "Pode exigir assistência técnica.\n")

    def diagnose(self):
        if self.brand.get() == "canon":
            host = self.host.get().strip()
            queue_name = self.winprinter.get()
            conn = self.conn.get()

            def work():
                if conn == "usb":
                    try:  # IVEC pelo cabo: traz o código exato do erro
                        with CanonUsb() as c:
                            return c.diagnose()
                    except (IvecError, UsbError):
                        pass  # sem IVEC no USB: tenta o IPP da fila instalada
                if conn == "rede" and host:
                    url = "ipp://%s:631/ipp/print" % host
                else:
                    url = ipp.find_queue_url(queue_name)
                return canon.diagnose(url)

            def done(d):
                lines = ["Modelo: %s" % d["modelo"], "Estado: %s" % d["estado"]]
                if d["mensagem"]:
                    lines.append("Mensagem: %s" % d["mensagem"])
                lines += ["Alerta: %s" % m for m in d["motivos"]]
                lines += ["Nível %s: %s" % (n, v) for n, v in d["tintas"]]
                self._show_problems(lines, d["problemas"])
            return self.run("Lendo erros", work, done)

        work = self.guarded(lambda: self.session(lambda p: p.diagnose()))
        if work:
            self.run("Lendo erros", work, lambda d: self._show_problems(
                ["Estado: %s" % d["estado"]] + ["Aviso: %s" % a for a in d["avisos"]],
                d["problemas"]))

    def lookup_code(self):
        text = self.code.get().strip()
        if not text:
            return
        if text.lower().startswith("0x"):
            try:
                found = [explain_epson_error(int(text, 16))]
            except ValueError:
                found = []
        else:
            found = search(text)
        self._show_problems(["Busca: %s" % text], found)
        if not found:
            self.diag_out.insert("end", "(nada encontrado)\n")

    def ee_image(self):
        path = filedialog.askopenfilename(filetypes=[
            ("EEPROM (JSON ou binário)", "*.json *.bin"), ("Todos", "*")])
        if not path:
            return
        start = 0
        if not path.lower().endswith(".json"):
            start = simpledialog.askinteger(
                "EcoReset", "Endereço inicial do arquivo binário:", initialvalue=0,
                minvalue=0, maxvalue=0xFFFF, parent=self)
            if start is None:
                return
        loaded = self.guarded(lambda: load_image(path, start))
        if not loaded:
            return
        image, file_model = loaded
        model = self.model.get()
        if file_model and model and file_model != model and not messagebox.askyesno(
                "EcoReset", "O arquivo é de %s e a impressora é %s. Gravar dados de "
                "outro modelo pode travá-la. Continuar mesmo assim?" % (file_model, model)):
            return
        include = messagebox.askyesno(
            "EcoReset", "Gravar também número de série e MAC que estiverem no arquivo?\n\n"
            "Responda NÃO, a menos que o arquivo seja backup desta mesma impressora.",
            default="no")
        work = self.guarded(lambda: self.session(
            lambda p: p.plan_image(image, include_protected=include)))
        if not work:
            return

        def planned(result):
            changes, skipped = result
            out = self.ee_out
            out.delete("1.0", "end")
            out.insert("end", "%d endereço(s) diferente(s); %d protegido(s) ignorado(s).\n"
                       % (len(changes), len(skipped)))
            for a, old, new in changes[:200]:
                out.insert("end", "%5d (0x%04X): %3d -> %3d\n" % (a, a, old, new))
            if not changes:
                return messagebox.showinfo("EcoReset", "Nada a gravar: a impressora já "
                                           "tem esses valores.")
            if not messagebox.askyesno("EcoReset", "Gravar %d endereço(s)? Os valores "
                                       "atuais serão salvos antes." % len(changes)):
                return
            backup = filedialog.asksaveasfilename(
                defaultextension=".json", filetypes=[("Backup", "*.json")],
                initialfile="backup-antes-regravar-%s.json"
                % datetime.datetime.now().strftime("%Y%m%d-%H%M%S"))
            if not backup:
                return
            write = self.guarded(lambda: self.session(
                lambda p: p.write_image(changes, backup_path=backup)))
            if write:
                self.run("Regravando", write, lambda _: messagebox.showinfo(
                    "EcoReset", "EEPROM regravada e conferida.\n"
                    "Desligue e ligue a impressora."))
        self.run("Comparando com a impressora", work, planned)

    def maintain_ivec(self, name=None):
        name = name or self.ivec_op.get()
        heavy = name in ("limpeza-profunda", "ink-flush")
        if not messagebox.askyesno("EcoReset", "Enviar '%s' para a Canon no USB?%s" % (
                name, "\n\nEsta operação gasta bastante tinta." if heavy else "")):
            return

        def work():
            with CanonUsb() as c:
                c.maintenance(name)
        self.run("Enviando %s" % name, work,
                 lambda _: self.status.set("%s enviado para a Canon." % name))

    def maintain(self, what):
        if self.brand.get() == "canon" and self.conn.get() == "usb":
            return self.maintain_ivec("teste-bicos" if what == "nozzle" else "limpeza")
        if self.brand.get() == "canon":
            job = canon.nozzle_check_job() if what == "nozzle" else canon.head_cleaning_job()
        else:
            job = (maintenance.nozzle_check_job() if what == "nozzle"
                   else maintenance.head_cleaning_job())
        if self.via.get() == "instalada":
            name = self.winprinter.get()
            if not name:
                return messagebox.showerror("EcoReset", "Escolha a impressora.")
            work = lambda: maintenance.send_raw_installed(name, job)
        else:
            host = self.host.get().strip()
            if not host:
                return messagebox.showerror("EcoReset", "Digite o IP da impressora.")
            work = lambda: maintenance.send_raw_network(host, job)
        if self.brand.get() == "canon":
            send = work
            url_of = ((lambda: ipp.find_queue_url(name)) if self.via.get() == "instalada"
                      else (lambda: "ipp://%s:631/ipp/print" % host))

            def work():
                canon.check_bjl(url_of())  # os modelos só-IVEC ignorariam o trabalho
                send()
        label = "Teste de bicos" if what == "nozzle" else "Limpeza de cabeça"
        self.run("Enviando " + label.lower(), work,
                 lambda _: self.status.set(label + " enviado."))

    def _int(self, var, name):
        text = var.get().strip()
        try:
            return int(text, 0)
        except ValueError:
            raise ValueError("%s inválido: %r (use decimal ou 0x..)" % (name, text))

    def ee_read(self):
        def prepare():
            start = self._int(self.addr, "endereço")
            end = self._int(self.addr_end, "endereço final") if self.addr_end.get().strip() else start
            if end < start or end - start > 1023:
                raise ValueError("intervalo inválido (máximo 1024 endereços)")
            return self.session(lambda p: p.dump(start, end))
        work = self.guarded(prepare)
        if not work:
            return

        def done(data):
            self.last_dump = data
            self.ee_out.delete("1.0", "end")
            for a, v in data.items():
                self.ee_out.insert("end", "%5d (0x%04X): %3d (0x%02X)\n" % (a, a, v, v))
        self.run("Lendo EEPROM", work, done)

    def ee_write(self):
        args = self.guarded(lambda: (self._int(self.addr, "endereço"),
                                     self._int(self.value, "valor")))
        if not args:
            return
        address, value = args
        if not messagebox.askyesno("EcoReset", "Gravar %d no endereço %d?" % (value, address)):
            return
        work = self.guarded(lambda: self.session(lambda p: p.write_eeprom(address, value)))
        if work:
            self.run("Gravando", work, lambda _: self.ee_out.insert(
                "end", "Gravado %d em %d.\n" % (value, address)))

    def ee_save(self):
        if not self.last_dump:
            return messagebox.showinfo("EcoReset", "Faça uma leitura primeiro.")
        path = filedialog.asksaveasfilename(defaultextension=".json",
                                            filetypes=[("JSON", "*.json")])
        if path:
            with open(path, "w", encoding="utf-8") as f:
                json.dump({"model": self.model.get(), "host": self.host.get(),
                           "eeprom": {str(a): v for a, v in self.last_dump.items()}},
                          f, indent=2)


def main():
    App().mainloop()
