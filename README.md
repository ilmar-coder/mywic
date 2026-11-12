# EcoReset

Manutenção de impressoras **Epson L/EcoTank** e **Canon PIXMA/MegaTank**, a nossa
alternativa ao WICReset. Funciona no **Windows**, no **Mac** e no **Fedora**, sem
comprar chave. Escrito do zero em **Rust** (a versão anterior, em Python, está na
tag `python-final`).

Não usa código do WICReset. O protocolo (comandos EPSON-CTRL por SNMP e pelo
cabo USB) e as chaves e endereços de cada modelo vêm da documentação pública do
projeto [epson_print_conf](https://github.com/Ircama/epson_print_conf); os
códigos da Canon, do manual oficial da G2060 series.

## O que faz

**Epson** (pelo cabo USB ou pela rede):
- **Almofada de tinta:** lê os contadores, zera (com backup automático) e restaura o backup.
- **Reset temporário:** para firmwares que bloqueiam a EEPROM pela rede. Libera a
  impressora usando só o número de série.
- **Diagnóstico:** o erro atual com causa, correção e se precisa de assistência.
- **Teste de bicos e limpeza:** pela rede ou pela impressora instalada no computador.
- **EEPROM:** lê e grava endereços, leitura completa, comparação de leituras
  (antes e depois de um reset), regravação a partir de arquivo (série e MAC
  protegidos) e busca da chave de modelos fora da lista.

**Canon:**
- **Cabo USB (IVEC):** o código exato do erro, o estado, o nível da caixa de
  manutenção e as manutenções (limpeza, profunda, preto, cor, rolete, teste de bicos).
- **Rede (IPP):** estado, alertas e códigos; teste de bicos e limpeza (BJL) nos
  modelos que aceitam.
- O contador da almofada da Canon só zera com a ferramenta de serviço da marca;
  modelos com caixa de manutenção (G2060...) só precisam trocar a caixa.

**Consulta de códigos:** `1726`, `5B00`, `0x10` ou uma palavra (`papel`).

## Modelos aprendidos (log do WICReset)

Faça um reset no WICReset (o `trial` gratuito, que deixa a almofada em 80%,
serve) e importe o `application.log` (aba **Modelos aprendidos**):

- Windows: `%APPDATA%\wicreset\application.log`
- Mac: `~/Library/Application Support/wicreset/`

O EcoReset tira do log as chaves e o reset do modelo, calcula os divisores e
passa a fazer o reset completo sozinho, pelo cabo USB. Só o texto do log é lido.

### Banco no servidor

Os modelos aprendidos vão para o banco do site (`https://download.ilmar.com.br`,
rotas `/api/ecoreset/models`, chave própria). Em cada máquina, uma vez:
**Modelos aprendidos → Servidor…**, com o endereço e a chave. O EcoReset atualiza
ao abrir; o que for aprendido sem internet sobe no próximo **Atualizar**.

A chave fica no Raspberry:

    ssh ilmar@192.168.0.69 cat /www/wwwroot/baixar-go/data/ecoreset_token

## Instalar

O GitHub Actions gera tudo a cada push (aba *Actions*):

- **Windows:** `EcoReset.exe` (tela) e `ecoreset-cli.exe`. Não precisa instalar
  nada. O USB usa o driver de impressora do próprio Windows: a impressora
  precisa estar instalada. Na primeira vez, o SmartScreen pode avisar
  (**Mais informações → Executar assim mesmo**).
- **Mac:** `EcoReset-mac.zip`, app universal (Apple Silicon e Intel) com o libusb embutido.
- **Fedora:** `sudo dnf install ./ecoreset-*.rpm`. Traz a regra que libera o USB
  das Epson e Canon para o usuário logado (se a impressora já estava ligada,
  desligue e ligue o cabo uma vez).

## Linha de comando

    ecoreset-cli detect usb
    ecoreset-cli levels usb
    ecoreset-cli reset usb
    ecoreset-cli temp-reset 192.168.0.50
    ecoreset-cli diagnose usb
    ecoreset-cli canon-status usb
    ecoreset-cli canon-maint limpeza
    ecoreset-cli code 1726
    ecoreset-cli snapshot usb -o antes.json
    ecoreset-cli compare antes.json depois.json
    ecoreset-cli find-key usb --write
    ecoreset-cli write-image usb backup.json
    ecoreset-cli wic-log application.log --save
    ecoreset-cli library --server
    ecoreset-cli library --sync

`usb` no lugar do IP usa o cabo. `ecoreset-cli --help` mostra tudo.

## Desenvolvimento

    cargo test -p ecoreset-core          # testes com impressoras simuladas
    cargo run -p ecoreset                # a tela
    SHOTS=/tmp/telas cargo test -p ecoreset -- --ignored screenshots

- `crates/core`: protocolo (SNMP, D4/USB, IVEC, IPP), modelos, erros, banco.
- `crates/cli`: a linha de comando.
- `crates/app`: a tela (egui).

## Limitações

- Firmwares novos bloqueiam a EEPROM pela rede; pelo cabo USB costuma funcionar
  (testado na L3250 com o firmware XF26P8).
- A causa do travamento do teste de bicos no Windows ainda não foi confirmada
  numa impressora: o fim do trabalho foi alinhado ao do escputil (gutenprint).
