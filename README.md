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

- **Diagnóstico de erros:** lê o erro atual da impressora e explica causa,
  correção e se precisa de assistência. Também consulta um código digitado
  (ex.: `1300`, `5B00`, `0x10`) ou uma palavra (ex.: `papel`).
- **Regravar EEPROM:** a partir de um arquivo (backup/leitura do EcoReset em
  JSON, ou um dump binário).

## Canon

- **Cabo USB (IVEC):** lê o **código exato do erro** (o mesmo do visor), o
  estado e o nível da caixa de manutenção. Também faz limpeza, limpeza só do
  preto ou só das cores, limpeza profunda, ink flush, limpeza do rolete e da
  placa inferior, e teste de bicos. Testado numa G2060 real: o código 7800 e a
  caixa MC-G02 em 90% foram lidos. Se a impressora estiver com código de erro,
  a manutenção não é enviada.
- **Rede:** diagnóstico pelo IPP (estado, alertas e níveis), mais a base de
  "support codes" do manual oficial da G2060 series, explicados em português.
- **Modelos antigos (BJL):** teste de bicos e limpeza pela fila da impressora.
  O programa confere se o modelo aceita BJL antes de enviar.
- **Almofada:** não existe forma pública de zerar o contador da almofada Canon
  (erro 5B00). Isso é feito pela ferramenta de serviço da Canon, que é
  proprietária. Modelos com caixa de manutenção (G2060, G3160...) não precisam
  de reset: troca-se a caixa (códigos 1726/1727).
- **Código exato do erro:** pelo IPP (rede), a G2060 não informa o código.
  Use o cabo USB, ou olhe o visor e use **Consultar código**.

O protocolo IVEC foi implementado a partir da descrição pública do projeto
[canon-universal-driver](https://github.com/noble6/canon-universal-driver)
(docs/PROTOCOL_IVEC.md). Não usa nenhum código ou biblioteca da Canon.

## Regravar a EEPROM (Epson)

Nenhum fabricante publica imagens de EEPROM "originais". A regravação usa um
arquivo que você fornece: o backup que o próprio EcoReset salva, a leitura
salva na aba EEPROM ou um dump binário (informando o endereço inicial).

Proteções:

- só grava o que é diferente do que está na impressora, e mostra a lista antes;
- número de série e MAC ficam protegidos, a menos que você libere (use só com
  backup da mesma impressora);
- avisa se o arquivo é de outro modelo;
- salva os valores atuais antes de gravar e confere cada endereço depois.

## Descobrir a chave de um modelo fora da lista

Na aba EEPROM, **Descobrir chave…** procura a chave de leitura da Epson
conectada. É **só leitura**: a chave errada só recebe "NA" da impressora.

- **Ordem da busca:** primeiro as chaves dos modelos conhecidos, depois as
  combinações em que caem quase todas as chaves conhecidas, e por fim as 65.536.
  O botão vira **Parar busca** enquanto roda.
- **Quanto demora:** de segundos (chave de família conhecida) a mais de uma hora
  (busca completa), conforme a conexão.
- **Depois de achar:** a chave é preenchida no campo ao lado, pronta para a
  **Leitura completa**. O programa também oferece testar as **chaves de
  gravação conhecidas**, gravando no endereço 0 o mesmo valor que já está lá (não
  muda nada). Uma chave de gravação nova (8 bytes) não dá para descobrir por
  tentativa.
- **Pela rede, num firmware bloqueado** (como o da L3250), nenhuma chave
  funciona. Use o cabo USB.

```
python -m ecoreset find-key usb --write
```

## Descobrir o reset de um modelo novo (leitura completa e comparação)

Para modelos que ainda não estão na lista, dá para descobrir quais endereços o
reset muda observando a própria impressora, sem mexer em outras ferramentas:

1. Na aba EEPROM, clique em **Leitura completa…** e salve, por exemplo, como
   "antes". Para um modelo fora da lista, preencha antes a **chave de leitura**.
2. Faça o reset com a outra ferramenta.
3. Faça outra **Leitura completa…** ("depois").
4. Clique em **Comparar leituras…**: aparecem os endereços que mudaram, com um
   palpite de cada um (contador zerado, nível 94...), e o programa monta a
   entrada de modelo sugerida, que pode ser salva em arquivo.

A leitura completa só lê, nunca grava. O arquivo dela também serve para a
regravação (é um backup completo da EEPROM).

Pela linha de comando:

```
python -m ecoreset snapshot usb -o antes.json --label antes
python -m ecoreset snapshot usb -o depois.json --label depois
python -m ecoreset compare antes.json depois.json --save modelo-sugerido.json
python -m ecoreset snapshot usb -o antes.json --key 74,54   # modelo fora da lista
```

## Aprender um modelo pelo log do WICReset

Faça um reset no WICReset (o `trial` gratuito, que deixa a almofada em 80%,
serve) e importe o log que ele grava em texto:

- Windows: `%APPDATA%\wicreset\application.log`
- Mac: `~/Library/Application Support/wicreset/`

O log mostra os comandos de gravação que foram para a impressora. Deles o
EcoReset tira a chave de leitura, a chave de gravação e os endereços do reset,
e calcula o divisor dos contadores pelo valor do trial. Daí em diante o reset
completo (contadores em 0) é feito pelo EcoReset, pelo cabo USB. Só o texto do
log é lido; nada da licença do WICReset é usado.

    python -m ecoreset wic-log application.log            # mostra o que aprendeu
    python -m ecoreset wic-log application.log --save     # grava no banco

Na tela: aba **Modelos aprendidos → Importar log do WICReset…**.

Endereços a partir de 0x100 (flags de estado) ficam de fora, porque variam por
unidade. Se o modelo tiver as mesmas chaves e endereços de um da lista, os
contadores são lidos como nele.

### Banco de modelos em várias máquinas

Os modelos aprendidos ficam em `modelos.json`, na pasta do EcoReset
(`~/EcoReset` por padrão). Para todas as máquinas usarem o mesmo banco, aponte
a pasta para uma pasta sincronizada (Google Drive, OneDrive, Dropbox, iCloud
ou pasta de rede). Cada máquina lê o banco ao abrir e, ao salvar, junta com o
que as outras gravaram.

    python -m ecoreset library --folder "G:\Meu Drive\EcoReset"
    python -m ecoreset library                          # lista os aprendidos
    python -m ecoreset library --merge outro/modelos.json
    python -m ecoreset library --remove XP-9999

A variável `ECORESET_PASTA` também escolhe a pasta.

## Modelos

L3150 (L3151, L3160, L3166, L3168), L4160 (L4150 a L4168), L355, L365, L366,
L395, L405, L3060, ET-2700, ET-2720, ET-2750, ET-2800, ET-2812 (ET-2810 a
ET-2818, L3250, L3251, L3256) e ET-4700.

A L3250 usa os parâmetros da ET-2812, que é da mesma família. As chaves de
leitura e gravação foram confirmadas numa L3250 real pelo USB.

Para acrescentar um modelo, coloque as chaves e os endereços dele em
`ecoreset/models.py`.

## Conexão: USB ou rede

- **Cabo USB:** fala o protocolo IEEE 1284.4 ("D4") da Epson e funciona até
  nos firmwares que bloqueiam a EEPROM pela rede.
  - **Mac/Linux:** pelo libusb. Testado numa L3250 real: leitura dos
    contadores, número de série e gravação. O `.app` já traz o libusb dentro.
    Rodando pelo código-fonte, instale com `brew install libusb`.
  - **Windows:** pelo usbprint.sys, o driver de impressora USB que o próprio
    Windows instala. Não precisa trocar driver nem usar Zadig. Se der erro de
    acesso, feche o monitor de status da Epson e espere a fila de impressão
    esvaziar. Ainda não foi testado num Windows real.
- **Rede:** usa SNMP, então a impressora precisa estar no Wi‑Fi ou no cabo de
  rede.

## Limitações

- **Firmwares novos bloqueiam a EEPROM pela rede.** É o caso da L3250 e de
  outras que vieram atualizadas. A impressora responde `NA`, o programa avisa e
  nada é gravado. Nesse caso use o **cabo USB** ou o **reset temporário**.
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
python -m ecoreset diagnose usb        # erros da Epson, com correção
python -m ecoreset code 1300           # explica um código
python -m ecoreset canon-status usb    # Canon: código exato pelo cabo
python -m ecoreset canon-maint limpeza # ou teste-bicos, limpeza-profunda...
python -m ecoreset write-image usb backup.json
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
