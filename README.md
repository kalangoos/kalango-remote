<p align="center">
  <img src="kalangoos/logo.png" alt="Kalango Remote" width="300">
</p>

<h1 align="center">Kalango Remote</h1>

<p align="center">
  O acesso remoto nativo do <a href="https://kalangoos.com">KalangoOS</a>: controlar outro computador, compartilhar a
  tela e acessar servidores por SSH e Mosh — sem domínio e sem servidor próprio.
</p>

---

## O que ele faz

- **Compartilhar a tela e controlar outro computador.** Quem compartilha recebe um código e um link para passar
  adiante; quem recebe o pedido aprova antes, vendo quem é e o que a pessoa vai poder fazer (ver a tela, controlar,
  arquivos, áudio). Enquanto alguém controla, o computador mostra sempre: um aviso na barra, um contorno na tela e o
  cursor com o nome de quem está controlando — e um botão para parar.
- **Conexão sem servidor.** O Kalango Remote abre na hora um túnel temporário do Cloudflare (`cloudflared`), sem
  conta, domínio nem servidor do KalangoOS no meio. Na mesma rede, conecta direto. Quem preferir pode usar um servidor
  próprio compatível com o RustDesk.
- **Arquivos e chat** durante a sessão, com uma fila de transferências.
- **SSH e Mosh.** Servidores e grupos (com configurações herdadas do grupo), importação do `~/.ssh/config` e do
  `known_hosts`, chaves no cofre do sistema, confirmação da impressão digital na primeira conexão e bloqueio quando a
  chave do servidor muda, encaminhamento de portas e saltos (ProxyJump). As sessões abrem como abas do Terminal do
  KalangoOS, com o painel de arquivos (SFTP) ao lado.
- **O agente do KalangoOS ajuda** — "conecte no computador da minha mãe e…" — e só age com aprovação, dos dois
  lados. Ele nunca lê o cofre de senhas e chaves nem aceita a chave de um servidor por conta própria.

## Estado

Em desenvolvimento. As telas estão desenhadas no Design System do KalangoOS (página "Kalango Remote"); o app nativo
em GTK 4 e as provas de conceito (o túnel do Cloudflare e a captura e o controle da tela no compositor do KalangoOS,
Wayland) estão sendo feitos agora. As instruções para compilar e rodar entram aqui junto com o primeiro app.

## Como o repositório se organiza

- `kalangoos/` — o que é do Kalango Remote: o app nativo, a integração com o KalangoOS, o logo e o ícone.
- O resto é o núcleo do RustDesk (rede, codecs, captura, protocolo), que o Kalango Remote usa por baixo.
- O ramo `kalangoos` é o do Kalango Remote; o `master` acompanha o RustDesk.

## Contribuir

Issues e pull requests são bem-vindos no ramo `kalangoos`. As regras da casa do KalangoOS valem aqui também:
[AGENTS.md do KalangoOS](https://github.com/kalangoos/kalangoos/blob/main/AGENTS.md).

## Licença e créditos

AGPL-3.0 (veja [LICENCE](LICENCE)). O Kalango Remote é baseado no
[RustDesk](https://github.com/rustdesk/rustdesk), da Purslane Tech Pte. Ltd. e colaboradores, também sob a AGPL-3.0: quem
distribui ou oferece o Kalango Remote como serviço precisa disponibilizar o código-fonte das suas mudanças.
