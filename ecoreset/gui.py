"""Interface gráfica (tkinter)."""

import datetime
import json
import queue
import sys
import threading
import tkinter as tk
from tkinter import filedialog, messagebox, simpledialog, ttk

from . import __version__, canon, ipp, maintenance
from .errors import explain_epson_error, search
from .models import model_names
from .printer import EpsonPrinter, PrinterError, load_image
from .d4 import D4Error
from .snmp import SnmpError
from .usb import UsbCtrlClient, UsbError

ERRORS = (PrinterError, SnmpError, UsbError, D4Error, ipp.IppError, OSError, ValueError)
# No Windows o USB ainda não tem acesso à EEPROM (precisa do USBPRINT).
USB_SUPPORTED = sys.platform != "win32"


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("EcoReset %s" % __version__)
        self.minsize(560, 460)
        self.results = queue.Queue()
        self.busy = False
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
        ttk.Combobox(top, textvariable=self.model, values=model_names(),
                     state="readonly", width=10).grid(row=0, column=5, sticky="w")

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
        ttk.Label(f, wraplength=480, foreground="#555", text=(
            "Coloque papel A4 antes do teste de bicos. Cada limpeza gasta tinta e "
            "enche a almofada: faça no máximo duas seguidas.")).grid(
            row=3, column=0, columnspan=2, sticky="w")
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
        self.ee_out = tk.Text(f, height=12, width=60, font=("Consolas", 10))
        self.ee_out.grid(row=2, column=0, columnspan=6, sticky="nsew")
        f.rowconfigure(2, weight=1)
        f.columnconfigure(5, weight=1)
        ttk.Label(f, foreground="#a00", text=(
            "Avançado: gravar endereços errados pode travar a impressora.")).grid(
            row=3, column=0, columnspan=6, sticky="w", pady=(5, 0))
        self.last_dump = {}
        return f

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
                if error:
                    self.status.set("Erro.")
                    messagebox.showerror("EcoReset", str(error))
                else:
                    self.status.set("Pronto.")
                    done(result)
        except queue.Empty:
            pass
        self.after(100, self._poll)

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

        def prepare(p):
            if model:
                p.set_model(model)
            else:
                p.detect()
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
        work = self.guarded(lambda: self.session(lambda p: (p.model_string(), p.model)))
        if not work:
            return

        def done(result):
            text, name = result
            if name:
                self.model.set(name)
                self.info.set("%s (%s) → parâmetros do %s" % (text, self.where(), name))
            else:
                self.info.set("%s: modelo não está na lista; escolha um compatível."
                              % (text or "Sem nome"))
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

    def maintain(self, what):
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
