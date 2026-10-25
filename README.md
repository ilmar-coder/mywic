# EcoReset

Utilitário de manutenção para impressoras Epson L/EcoTank, a nossa alternativa
ao WICReset. Funciona no Windows e no Mac e não precisa comprar chave.

- **Almofada de tinta:** lê os contadores principal e sem margem, zera os dois
  (com backup automático) e restaura o backup.
- **Reset temporário:** para firmwares que bloqueiam a EEPROM (como o da L3250).
  Libera a impressora usando só o número de série.
- **Manutenção:** teste de bicos e limpeza de cabeça, pela rede ou por qualquer
  impressora instalada no computador, USB também (spooler no Windows, CUPS no Mac).
- **EEPROM:** lê um endereço ou um intervalo e grava um endereço. Modo avançado.

Escrito do zero em Python, só com a biblioteca padrão. Não usa código do
WICReset. O protocolo (comandos EPSON-CTRL por SNMP) e as chaves e endereços
de cada modelo vêm da documentação pública do projeto
[epson_print_conf](https://github.com/Ircama/epson_print_conf), de Ircama.

## Modelos

L3150 (L3151, L3160, L3166, L3168), L4160 (L4150 a L4168), L355, L365, L366,
L395, L405, L3060, ET-2700, ET-2720, ET-2750, ET-2800, ET-2812 (ET-2810 a
ET-2818, L3250, L3251, L3256) e ET-4700.

A L3250 usa os parâmetros da ET-2812, que é da mesma família. As chaves de
leitura e gravação foram confirmadas numa L3250 real pelo USB.

Para acrescentar um modelo, coloque as chaves e os endereços dele em
`ecoreset/models.py`.

## Conexão: USB ou rede

- **Cabo USB (Mac e Linux):** fala o protocolo IEEE 1284.4 ("D4") da Epson,
  pelo libusb, e funciona até nos firmwares que bloqueiam a EEPROM pela rede.
  Testado numa L3250 real: leitura dos contadores, número de série e
  gravação. O `.app` já traz o libusb dentro. Rodando pelo código-fonte, instale
  com `brew install libusb`.
- **Rede:** usa SNMP, então a impressora precisa estar no Wi‑Fi ou no cabo de
  rede.

## Limitações

- **Firmwares novos bloqueiam a EEPROM pela rede.** É o caso da L3250 e de
  outras que vieram atualizadas. A impressora responde `NA`, o programa avisa e
  nada é gravado. Nesse caso use o **cabo USB** ou o **reset temporário**.
- **No Windows, o acesso à EEPROM pelo USB ainda não existe.** Lá ele precisa
  passar pelo driver da Epson (USBPRINT). Por enquanto, no Windows só funciona
  pela rede.
- **O teste de bicos e a limpeza pelo USB** vão pela fila da impressora
  instalada (CUPS no Mac, spooler no Windows), e não pelo D4.
- **O reset só zera o contador.** Se a almofada estiver cheia de verdade,
  troque-a ou instale um reservatório externo, senão a tinta vaza.

## Usar

Abra o `EcoReset.exe` (Windows) ou o `EcoReset.app` (Mac), digite o IP da
impressora (aparece na folha de status da rede) e clique em **Detectar**.

No Mac, como o app não é assinado, na primeira vez clique com o botão direito
nele e escolha **Abrir**.

Linha de comando (qualquer sistema com Python 3.8+):

```
python -m ecoreset levels usb          # pelo cabo USB
python -m ecoreset reset usb
python -m ecoreset detect 192.168.0.50
python -m ecoreset levels 192.168.0.50
python -m ecoreset reset 192.168.0.50
python -m ecoreset temp-reset 192.168.0.50
python -m ecoreset nozzle 192.168.0.50
python -m ecoreset clean 192.168.0.50
python -m ecoreset read 192.168.0.50 40 60
python -m ecoreset --model L3150 write 192.168.0.50 48 0
```

## Gerar os executáveis

- **Windows:** rode `build_windows.bat`. O executável sai em `dist\EcoReset.exe`.
- **Mac:** rode `./build_mac.sh`, que precisa de Python com Tk
  (`brew install python-tk`). O app sai em `dist/EcoReset.app`.
- **GitHub Actions:** o workflow em `.github/workflows/build.yml` gera os dois a
  cada push. Eles ficam na aba Actions, em Artifacts.

## Testes

```
python -m unittest discover -s . -p "test_*.py" -v
```

Os testes rodam contra uma impressora simulada, pela rede
(`tests/fake_printer.py`) e pelo USB/D4 (`tests/fake_d4.py`).

O protocolo D4 foi escrito a partir da descrição pública do d4lib do
gutenprint.
