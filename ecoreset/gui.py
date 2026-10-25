"""Interface gráfica (tkinter)."""

import datetime
import json
import queue
import threading
import tkinter as tk
from tkinter import filedialog, messagebox, simpledialog, ttk

from . import __version__, maintenance
from .models import model_names
from .printer import EpsonPrinter, PrinterError
from .snmp import SnmpError

ERRORS = (PrinterError, SnmpError, OSError, ValueError)


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

        ttk.Label(top, text="IP:").grid(row=0, column=0, sticky="w", padx=5, pady=5)
        self.host = tk.StringVar()
        ttk.Entry(top, textvariable=self.host, width=18).grid(row=0, column=1, sticky="w")
        ttk.Button(top, text="Detectar", command=self.detect).grid(row=0, column=2, padx=5)

        ttk.Label(top, text="Modelo:").grid(row=0, column=3, sticky="w", padx=(15, 5))
        self.model = tk.StringVar()
        ttk.Combobox(top, textvariable=self.model, values=model_names(),
                     state="readonly", width=12).grid(row=0, column=4, sticky="w")

        self.info = tk.StringVar(value="Digite o IP da impressora e clique em Detectar.")
        ttk.Label(top, textvariable=self.info, foreground="#555").grid(
            row=1, column=0, columnspan=5, sticky="w", padx=5, pady=(0, 5))

        tabs = ttk.Notebook(self)
        tabs.pack(fill="both", expand=True, padx=10, pady=5)
        tabs.add(self._tab_waste(tabs), text="Almofada de tinta")
        tabs.add(self._tab_maintenance(tabs), text="Manutenção")
        tabs.add(self._tab_eeprom(tabs), text="EEPROM")

        self.status = tk.StringVar(value="Pronto.")
        ttk.Label(self, textvariable=self.status, relief="sunken", anchor="w").pack(
            fill="x", side="bottom")

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

    def printer(self):
        host = self.host.get().strip()
        if not host:
            raise PrinterError("digite o IP da impressora")
        p = EpsonPrinter(host)
        if self.model.get():
            p.set_model(self.model.get())
        return p

    def guarded(self, fn):
        try:
            return fn()
        except ERRORS as e:
            messagebox.showerror("EcoReset", str(e))

    # ---- ações ------------------------------------------------------------

    def detect(self):
        host = self.host.get().strip()
        if not host:
            return messagebox.showerror("EcoReset", "Digite o IP da impressora.")
        p = EpsonPrinter(host)

        def done(result):
            text, name = result
            if name:
                self.model.set(name)
                self.info.set("%s → parâmetros do %s" % (text, name))
            else:
                self.info.set("%s: modelo não está na lista; escolha um compatível."
                              % (text or "Sem nome"))
        self.run("Detectando", p.detect, done)

    def read_levels(self):
        p = self.guarded(self.printer)
        if p:
            self.run("Lendo contadores", p.waste_levels, self._show_levels)

    def _show_levels(self, levels):
        for label, (bar, text) in self.bars.items():
            if label in levels:
                bar["value"] = min(levels[label], 100)
                text.set("%.2f%%" % levels[label])
            else:
                bar["value"] = 0
                text.set("n/d")

    def reset(self):
        p = self.guarded(self.printer)
        if not p:
            return
        if not messagebox.askyesno("EcoReset", (
                "Zerar os contadores da almofada da %s em %s?\n\n"
                "Escolha onde salvar o backup a seguir.") % (p.model, p.host)):
            return
        path = filedialog.asksaveasfilename(
            defaultextension=".json", filetypes=[("Backup", "*.json")],
            initialfile="backup-%s-%s.json" % (
                p.model, datetime.datetime.now().strftime("%Y%m%d-%H%M%S")))
        if not path:
            return

        def done(_):
            messagebox.showinfo("EcoReset", "Contadores zerados.\n"
                                "Desligue a impressora e ligue de novo.")
            self.read_levels()
        self.run("Resetando", lambda: p.reset_waste(backup_path=path), done)

    def temp_reset(self):
        p = self.guarded(self.printer)
        if not p:
            return

        def ask(serial):
            serial = simpledialog.askstring(
                "EcoReset", "Número de série da impressora (está na etiqueta "
                "e na folha de status):", initialvalue=serial or "", parent=self)
            if serial and serial.strip():
                self.run("Reset temporário", lambda: p.temporary_reset(serial.strip()),
                         lambda _: messagebox.showinfo(
                             "EcoReset", "Impressora liberada.\n"
                             "Desligue e ligue de novo."))

        def read_serial():
            try:
                return p.serial_number()
            except ERRORS:
                return ""
        self.run("Lendo número de série", read_serial, ask)

    def restore(self):
        p = self.guarded(self.printer)
        if not p:
            return
        path = filedialog.askopenfilename(filetypes=[("Backup", "*.json")])
        if not path:
            return
        with open(path, encoding="utf-8") as f:
            backup = json.load(f)
        if backup.get("model") != p.model and not messagebox.askyesno(
                "EcoReset", "Este backup é de %s, não de %s. Continuar?"
                % (backup.get("model"), p.model)):
            return
        self.run("Restaurando", lambda: p.restore(backup),
                 lambda _: messagebox.showinfo("EcoReset", "Backup restaurado."))

    def maintain(self, what):
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
            p = self.printer()
            start = self._int(self.addr, "endereço")
            end = self._int(self.addr_end, "endereço final") if self.addr_end.get().strip() else start
            if end < start or end - start > 1023:
                raise ValueError("intervalo inválido (máximo 1024 endereços)")
            return p, start, end
        args = self.guarded(prepare)
        if not args:
            return
        p, start, end = args

        def done(data):
            self.last_dump = data
            self.ee_out.delete("1.0", "end")
            for a, v in data.items():
                self.ee_out.insert("end", "%5d (0x%04X): %3d (0x%02X)\n" % (a, a, v, v))
        self.run("Lendo EEPROM", lambda: p.dump(start, end), done)

    def ee_write(self):
        args = self.guarded(lambda: (self.printer(), self._int(self.addr, "endereço"),
                                     self._int(self.value, "valor")))
        if not args:
            return
        p, address, value = args
        if not messagebox.askyesno("EcoReset", "Gravar %d no endereço %d?" % (value, address)):
            return
        self.run("Gravando", lambda: p.write_eeprom(address, value),
                 lambda _: self.ee_out.insert("end", "Gravado %d em %d.\n" % (value, address)))

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
