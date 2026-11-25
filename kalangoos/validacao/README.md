# Kalango Remote · validação entre duas máquinas (VMs da ISO 0.1.0)

Duas VMs QEMU/KVM da ISO publicada do KalangoOS 0.1.0 (`kalangoos-0.1.0-amd64.iso`), sessão ao vivo, 4 GB cada.
Cada VM tem uma placa NAT do QEMU (internet, sem entrada) e uma placa "LAN" ligada à outra por um socket (cabo
virtual de camada 2, 192.168.77.1 e .2). Sem root no anfitrião. Para repetir: `kalangoos/scripts/duas-vms.sh`
(liga as VMs), `testar-sessao.sh` com `MAQUINAS=1` ou `MAQUINAS=wayland` (sessão do núcleo A↔B),
`testar-ssh-mosh-duas-maquinas.sh` (SSH e Mosh). Os cliques na interface vieram do monitor do QEMU (QMP
`input-send-event`/`send-key`: dispositivo de entrada virtio de verdade, não xdotool).

## Resultados

| Cenário | Como | Resultado | Registro |
|---|---|---|---|
| Conexão direta por endereço na LAN | `testar-sessao.sh direto`, núcleo na B, probe na A | OK: TCP, cifrada de ponta a ponta, 30 quadros, ~12 qps | registros/ |
| Código na LAN (UDP) | `codigo` | falhou antes (`code_not_found`); **OK depois da correção do defeito 2** | registros/codigo.txt |
| Túnel Cloudflare ("sem servidor") | app na B compartilha; A conecta pelo link; e `tunel` | OK: o `cloudflared` da VM abre o túnel rápido pelo NAT do QEMU, A conecta por WSS (50 ms) | capturas/01-*, registros/tunel.txt |
| Fluxo de aprovação na B (clique humano) | app na B, pedido de acesso, clique pelo QMP | OK. Sem clique: nega sozinho aos 30 s e A mostra "A pessoa negou o acesso" | capturas/00-*, 00b-* |
| Captura e controle (wlr no niri da B) | sessão aprovada | OK **depois da correção do defeito 1**; A vê a tela da B, teclado e mouse da A chegam à B | capturas/01-* |
| Esc Esc na B encerra a sessão | patch 0009 | OK; A mostra "Conexão perdida" e tenta de novo | |
| Chat | botão Chat | OK (mensagem da B apareceu na A) | |
| Transferência de arquivos | `fixa`: senha fixa não abre arquivos; a aprovada por clique abre | OK no núcleo; **a tela de transferência na GUI não foi exercitada** | registros/fixa.txt |
| Acesso sem supervisão, senha fixa | GUI (config da B) e `fixa` | OK: entra sem aprovação, só com permissões limitadas; trocar a senha expulsa a sessão; desligar o acesso recusa a senha | capturas/02-*, registros/fixa.txt |
| Verificação em duas etapas | `duasetapas` | OK: sem o código não abre; com o código abre | registros/ |
| Defesas | `defesas` | OK: setup inválido derruba o núcleo; código usado não vale de novo; 5 erros travam | registros/ |
| Impostor (chave diferente do convite) | `impostor` | OK: recusado | registros/ |
| Gravação | GUI e `gravar` | OK: modos reais 700 (pasta) e 600 (arquivo) lidos na B por ssh, vídeo > 1 KiB; o teste falha se o `stat` falhar (mutante com pasta errada: FALHOU) | registros/gravar.txt |
| Modo privacidade (patch 0012) | sessão sem supervisão com `privacy_black` | OK: monitor da B preto com a faixa "controlando", A continua vendo a tela real | capturas/02b-* |
| SSH: chave desconhecida, confiar, conectar | `testar-ssh-mosh-duas-maquinas.sh`, sshd da B | OK: recusa sem confiar (`unknown`), conecta depois | registros/ssh-mosh.txt |
| SSH: chave do servidor mudou | idem | OK: `changed` e conexão bloqueada | |
| Mosh | app monta `mosh --ssh=…`; mosh-server da B; a placa da B cai por 8 s | OK, com prova: a A viu os números 1 a 7 até 6,3 s, nada por 9,8 s, e os números 8 a 16 de uma vez a 16,1 s, quando a rede voltou; a contagem terminou (marcador calculado pela B, `FIM-MOSH-42`) | registros/ssh-mosh.txt |
| Mosh: chave mudou | idem | OK: bloqueado | |
| Plano do agente | | **Não testado** (precisa do agente e do modelo) | |

> Correção de uma versão anterior deste teste: a primeira prova do Mosh procurava `FIM-MOSH` no log do `script`, e o
> cabeçalho do log repete o comando, então ela passava mesmo com o Mosh morto (sem `TERM`, e sem tamanho de terminal).
> Agora o marcador é calculado pela B e o teste mede, pelos tempos de cada pedaço recebido, o intervalo sem dados.

## Defeitos achados

1. **O computador controlado nunca mostrava a tela na imagem instalada** (corrigido no repositório do sistema,
   `kalangod`): `/usr/bin/kalango-remote` é um script que executa `/usr/lib/kalango/remote/kalango-remote`, o kernel
   nomeia o app pelo segundo caminho e a lista padrão `controle_remoto_apps` só tinha o primeiro. Mensagem:
   "…/kalango-remote não é o Kalango Remote"; o núcleo repetia "no zwlr_screencopy_manager_v1". Confirmado nas VMs:
   com os dois caminhos em `/etc/kalango/kalangod.toml` a sessão mostra a tela. Corrigido o padrão (com teste).
2. **O código da LAN só alcançava a placa da rota padrão** (`kalangoos/core/src/lan.rs`): a consulta ia só para
   255.255.255.255, que sai pela interface da rota padrão. Com um segundo adaptador (cabo ao lado do Wi-Fi, uma
   ponte de máquinas virtuais) o código não achava o computador. Agora a consulta vai também para o endereço de
   difusão de cada rede ativa com difusão; ficam de fora: interfaces fora do ar, loopback, ponto a ponto (tun/wg),
   redes /31 e /32 (sem endereço de difusão próprio) e as pontes do docker (`docker0`, `br-*`, `veth*`), onde ninguém
   que compartilha um código está. Teste: `lan::tests::the_query_goes_to_every_real_network`.

## Mosh: onde o código supõe `mosh` no PATH

- `kalangoos/app/src/ssh/ops.rs`, `command()`: o primeiro elemento do comando é a palavra `mosh`, sem caminho; quem
  executa (o Terminal) resolve pelo PATH. O mesmo vale para `ssh`, `ssh-add` e `ssh-keyscan`.
- A ISO já traz `mosh` em `imagem/pacotes.txt` (linha com `openssh-client mosh`), com `mosh-client` e `mosh-server`
  em `/usr/bin`. **Não traz `openssh-server`**: o computador da ISO é cliente de SSH/Mosh; para ser servidor tem de
  instalar `openssh-server` (e `mosh` já está lá para o lado servidor). Nas VMs foi instalado com `apt`.
- Mosh usa UDP 60000-61000 até o servidor; num computador com firewall isso precisa estar aberto.

## O que ainda precisa de máquinas de verdade

- Captura e codificação com GPU (as VMs usam virtio-gpu e VP8 por software), latência e fluidez reais.
- Wi-Fi de verdade, perda de pacotes e roaming; o comportamento do NAT residencial e do CGNAT (o túnel Cloudflare
  funcionou pelo NAT do QEMU, que é simples demais para provar o resto).
- Teclado e mouse físicos; vários monitores, escala e rotação; suspender com sessão remota aberta.
- A tela de transferência de arquivos e a de aprovação com leitor de tela.
- O diálogo "confiar nesta chave" do SSH foi simulado com `ssh-keyscan` (o que ele grava), não clicado.
