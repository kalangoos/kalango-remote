# Kalango Remote · provas de viabilidade

Resultados medidos em 19/11/2026, numa máquina Ubuntu 24.04 (x86_64, NVIDIA), com o núcleo do RustDesk
deste fork compilado em modo de depuração. Tudo o que está aqui foi rodado; o que não deu para rodar está
na última seção.

## A. Conexão sem servidores próprios: túnel rápido do Cloudflare

### O túnel

- `cloudflared tunnel --no-autoupdate --url tcp://127.0.0.1:PORTA` devolve um endereço
  `https://<palavras>.trycloudflare.com` em **~7 s**, sem conta. O túnel registrou na borda de São Paulo
  (`location=gru11`, QUIC). Os avisos sobre ICMP (`ping_group_range`) são inofensivos.
- Do outro lado, `cloudflared access tcp --hostname <túnel> --url 127.0.0.1:PORTA` entrega a conexão TCP.
  Teste de eco: **RTT mediano 36,5 ms (p90 39,7 ms)**; primeira troca 833 ms.
- **Quem conecta não precisa do cloudflared.** Uma conexão WebSocket comum para `wss://<túnel>/` carrega o
  fluxo TCP em mensagens binárias (é o que o `access tcp` faz por dentro). Testado com um cliente
  WebSocket qualquer: eco correto, 938 ms até a primeira resposta. O núcleo do Kalango Remote faz isso em
  Rust (`kalangoos/core/src/bridge.rs`): abre uma porta local e liga cada conexão a um WSS.
- Vazão de um sentido só, as duas pontas na mesma máquina: **32 MiB a 1,31 MB/s (~10 Mbit/s)**. Nesse
  arranjo os dados sobem e descem pelo mesmo enlace de casa, então o número mede o limite de subida
  daqui, não o do Cloudflare. Dá para vídeo 1080p no modo "Equilibrada".
- O cloudflared é baixado na versão fixada (2026.10.7) com o SHA-256 conferido por
  `kalangoos/scripts/cloudflared.sh`; o binário nunca entra no repositório (Apache-2.0).

### O RustDesk através do túnel

A pergunta era se dava para usar o encontro e a retransmissão do RustDesk (hbbs/hbbr) pelo túnel ou se o
par devia escutar direto. **Decisão: o computador compartilhado escuta direto e o túnel leva essa porta.**
Sem hbbs/hbbr não há servidor nenhum para manter, e o túnel já faz o papel da retransmissão.

O que precisou mudar no núcleo (tudo neste fork, em `src/kalango/` e em ganchos pequenos):

1. **O `start_server` do RustDesk fala com servidores de fora** — registra nos servidores de encontro
   (públicos, por padrão), consulta STUN para o tipo de NAT e sincroniza com um servidor de API. O
   `kalango::direct` sobe só o necessário para conexões diretas: a IPC de configuração, os serviços e um
   ouvinte TCP (em 127.0.0.1 quando o túnel leva a porta; em todos os endereços para a rede local).
2. **Conexão direta por IP no RustDesk de fábrica não é cifrada.** O cliente só cifra quando conhece a
   chave pública do par, e essa chave vem do servidor de encontro. Sem ele, a sessão passaria em texto
   aberto pelo Cloudflare. Agora o **convite leva a identidade** (id do RustDesk + chave ed25519, no
   fragmento do link, que nunca é enviado a ninguém); o cliente fixa essa identidade para o endereço, faz
   o mesmo aperto de mão assinado das conexões com servidor e **recusa** qualquer outra chave — não cai
   para texto aberto (`src/client.rs`, `secure_connection`).
3. **"Perguntar antes de cada acesso"**: no RustDesk, a senha certa autoriza na hora. A opção
   `kalango-approve-after-password` faz a senha certa só levar o pedido até quem está no computador, que
   aprova (ou não) na janela do pedido.

Medições com `kalangoos/scripts/testar-sessao.sh` (dois processos do núcleo, cada um com a sua pasta
pessoal descartável; o lado compartilhado captura um Xvfb com uma bola animada a 30 qps, 1600×1000):

| Caminho | Primeiro quadro | Quadros por segundo | Atraso | Cifrada |
|---|---|---|---|---|
| Direto, 127.0.0.1 (identidade fixada) | 242 ms | 24,2 | 0 ms | sim |
| **Túnel do Cloudflare, cliente só com WSS** | **2,7 s** | **23,3** | **92 ms** | **sim** |
| Rede local, achado pelo código (UDP) | 350 ms | 18,3 | — | sim |

Casos negativos e de fluxo, também rodados:

- **Impostor**: convite com outra chave → `the peer's identity does not match the invite (signature does
  not verify)`, nenhum quadro, nada em texto aberto.
- **Aprovação**: com `--ask`, a senha certa gera o pedido (`connection`, `authorized: false`); o teste
  aprovou 3,6 s depois do início e o primeiro quadro chegou em 4,0 s — nenhum antes da aprovação.

### Código e link

- **Link** (`https://<túnel>.trycloudflare.com/#k=<id>.<chave>`): funciona de qualquer lugar.
- **Código** (`7Q4M-29XR`): os primeiros 40 bits do SHA-256 da chave pública em base32 de Crockford. Serve
  para conferir em voz alta que se chegou ao computador certo e **acha o computador na mesma rede** (o
  núcleo responde ao código por UDP na porta 21120; a resposta só vale se a chave dela der aquele código).
  Pela internet, um código curto não se resolve sem um servidor de nomes, então lá é preciso o link. Isso
  é uma limitação real do "sem servidor".

### Riscos do túnel rápido

- Os túneis do `trycloudflare.com` são gratuitos e sem garantia: o Cloudflare os descreve como para testes
  e desenvolvimento e pode limitar ou mudar o serviço. O app trata a falha como um estado ("O túnel não
  abriu") e oferece a rede local e o servidor próprio (compatível com o RustDesk) em Ajustes.
- O nome novo leva alguns segundos para resolver no DNS logo depois de criado (numa rodada o resolvedor
  ainda não o conhecia e a sessão caiu na hora). O núcleo só anuncia o link quando o nome resolve, e a
  ponte WSS tenta de novo por até 30 s; com isso, duas rodadas seguidas pelo túnel passaram (primeiro
  quadro em 1,2 s e 2,7 s, 22,5 qps).
- Redes que bloqueiam a porta 7844 (saída do cloudflared) impedem o túnel; o cliente só usa HTTPS (443).

## B. Captura e entrada no niri (Wayland)

Testado num niri aninhado (o 26.04 com os patches do KalangoOS, em Xvfb :89, renderização por software)
com uma prova em Rust: `kalangoos/poc/wlr-remote`.

### O caminho do RustDesk (portal) não serve no niri

- O RustDesk no Wayland captura com o portal ScreenCast (PipeWire) e injeta entrada com o portal
  **RemoteDesktop**. O niri implementa a API ScreenCast do Mutter (só com `--session`, e precisa de GBM: no
  niri aninhado ela nem aparece), mas **não implementa RemoteDesktop**: `org.gnome.Mutter.RemoteDesktop`
  não existe no barramento e o portal responde `AvailableDeviceTypes = 0`. Sem isso, o RustDesk de fábrica
  mostraria a tela mas não controlaria nada.

### O caminho que funciona: protocolos do próprio Wayland

O niri expõe a clientes sem restrição (todo app fora de sandbox) três globais:
`zwlr_screencopy_manager_v1`, `zwlr_virtual_pointer_manager_v1` e `zwp_virtual_keyboard_manager_v1`.

- **Captura** (`wlr-screencopy`): 1280×800 capturado corretamente. Sem damage e com renderização por
  software, 30 capturas em 5,84 s = **5,1 qps** (o número com GPU não foi medido; ver abaixo).
- **Ponteiro** (`virtual-pointer`): movimento absoluto e clique chegaram a uma janela GTK 4 (o botão
  registrou `CLICKED`).
- **Teclado** (`virtual-keyboard`): o texto `kalango remote 123` chegou inteiro a um campo GTK 4.
- Armadilhas achadas: a saída do niri aninhado tem transformação `Flipped180` (o quadro vem de ponta-
  cabeça; o cliente tem de aplicar a transformação do `wl_output`); depois de mandar o mapa de teclas é
  preciso esperar ~300 ms, senão as primeiras teclas se perdem.

**Decisão:** o Kalango Remote ganha, neste fork, um capturador `wlr-screencopy` no `scrap` e uma entrada
por `virtual-pointer`/`virtual-keyboard` no servidor, usados quando o compositor os oferece; o portal fica
para os outros compositores. Para a tela de entrada (antes do login), o caminho é o `uinput` do RustDesk
rodando como root — no KalangoOS isso passaria pelo `kalangod`, com aprovação explícita.

**Segurança:** no niri original, qualquer programa fora de sandbox na sessão pode usar esses três protocolos.
O KalangoOS fecha isso com o patch 0008 do niri: com `KALANGO_NIRI_CONTROLE`, os três só aparecem para quem o
`kalangod` (root) permite — o teclado na tela do shell sempre, e o núcleo do Kalango Remote só durante uma sessão
que a pessoa aprovou (o app pede `kalango/controle_remoto` logo antes de autorizar e tira quando a sessão acaba).
Testado em `kalangoos/scripts/testar-permissao.sh`.

## C. Arquitetura que saiu das provas

- **Dois processos.** O núcleo do RustDesk liga GTK 3 no Linux, e GTK 3 e GTK 4 no mesmo processo
  quebram (`cannot register existing type 'GdkDisplayManager'`, falha de segmentação ao abrir a janela —
  testado). O app (GTK 4, `kalangoos/app`) roda o núcleo em `kalango-remote-core` (`kalangoos/core`) e
  conversa com ele por pacotes no stdin/stdout (`kalangoos/proto`: JSON para eventos e comandos, quadros
  BGRA crus).
- **Instâncias separadas.** O RustDesk põe as sockets de IPC em `/tmp/<app>-<uid>`, fora da pasta pessoal:
  duas instâncias do mesmo usuário se misturam. O nome do app é `kalango-remote` (nunca mistura com um
  RustDesk instalado) e `KALANGO_REMOTE_INSTANCE` separa as instâncias dos testes.

## O que não foi testado

- **niri numa sessão de verdade** (DRM, `niri --session`) e numa VM: precisa de tty ou de `sudo` para a
  VM gráfica. Logo, a velocidade da captura com GPU, a captura por dmabuf e o ScreenCast do Mutter no niri
  ficaram sem número.
- **Dois computadores físicos.** As sessões rodaram entre dois processos isolados da mesma máquina; pelo
  túnel, o tráfego foi de verdade até a borda do Cloudflare e voltou.
- A captura usada nas medições de sessão foi a de X11 (Xvfb), a do RustDesk de fábrica; a do niri ainda
  não está no `scrap` (é a primeira parte da fase 2).
