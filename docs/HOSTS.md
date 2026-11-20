# Kalango Remote · servidores SSH (`hosts.json`) e o contrato com o Terminal

O Kalango Remote é o dono dos servidores SSH, dos grupos, das chaves, das impressões digitais e dos
encaminhamentos. As sessões abrem no **Terminal do KalangoOS**. Este arquivo descreve o que os dois
combinam; o Terminal implementa o lado dele contra isto.

## Abrir um servidor no Terminal

```sh
kalango-shell terminal-ssh <host-id>              # abre (ou foca) a aba do servidor
kalango-shell terminal-ssh <host-id> --arquivos   # a mesma aba, com o painel de arquivos (SFTP) aberto
```

O Kalango Remote chama isso em "Abrir no Terminal", no clique duplo e nos Recentes, **depois** de conferir
a chave do servidor (primeira conexão: diálogo com a impressão digital; chave mudada: bloqueio) e, em
servidores com "Pedir confirmação antes de conectar", depois da confirmação.

Para montar a conexão, o Terminal pergunta ao Kalango Remote:

```sh
kalango-remote ssh-command <host-id>
# → um array JSON com o comando a rodar, por exemplo:
# ["ssh","-F","/dev/null","-o","UserKnownHostsFile=/home/…/.config/kalango/remote/known_hosts",
#  "-o","GlobalKnownHostsFile=/dev/null","-o","StrictHostKeyChecking=yes","-o","ServerAliveInterval=30",
#  "-i","/run/user/1000/kalango-remote/keys/deploy-ed25519","-o","IdentitiesOnly=yes",
#  "-o","ProxyCommand=ssh -F /dev/null … -W %h:%p -p 2222 ops@bastiao.example.com",
#  "-L","127.0.0.1:5433:db-staging:5432","-p","22","deploy@203.0.113.24"]
```

- Sai com 0 e o array no stdout; com 2 se o id não existe; com 1 e a causa no stderr se algo falta (por
  exemplo, a chave não está no chaveiro).
- O Terminal roda o array como está (sem shell) num pty. Para Mosh, o array começa com `mosh` e traz
  `--ssh=…`: o `mosh-client` do sistema roda como programa separado, autenticado pelo SSH antes.
- `StrictHostKeyChecking=yes` com o `known_hosts` do Kalango Remote: o Terminal nunca aceita chave de
  servidor sozinho. Se a conexão falhar por chave, o Terminal manda abrir o Kalango Remote
  (`kalango-remote --page ssh`), que mostra o diálogo certo.
- A chave privada do cofre é escrita num arquivo 0600 em `$XDG_RUNTIME_DIR/kalango-remote/keys/<id>`
  (tmpfs, pasta 0700) só para o ssh ler. **O Terminal apaga esse arquivo quando a conexão termina.**
  Quando o cofre do `kalangod` existir, isto vira um pedido por D-Bus e a chave deixa de passar por arquivo.
- Encaminhamentos com `with_host: true` vão junto no mesmo comando.

## O arquivo

`$XDG_CONFIG_HOME/kalango/remote/hosts.json` (normalmente `~/.config/kalango/remote/hosts.json`). Escrito
inteiro e trocado por `rename`, então quem lê nunca vê metade de um arquivo. O Terminal **só lê**.

```json
{
  "version": 1,
  "groups": [
    { "id": "trabalho", "name": "Trabalho", "parent": null,
      "defaults": { "user": "deploy", "identity": "deploy-ed25519", "jump": "bastiao-trabalho",
                    "keepalive": 30, "forward_agent": null } },
    { "id": "producao", "name": "produção", "parent": "trabalho", "defaults": { "keepalive": 15 } }
  ],
  "hosts": [
    { "id": "app-prod-01", "name": "app-prod-01", "address": "203.0.113.24", "port": 22,
      "group": "producao", "tags": ["trabalho", "produção"], "type": "ssh",
      "user": null, "identity": null, "jump": null, "keepalive": null, "forward_agent": null,
      "confirm": true, "last_used": 1795140000 }
  ],
  "identities": [
    { "id": "deploy-ed25519", "kind": "vault-key", "public_key": "ssh-ed25519 AAAA… deploy@kalango",
      "fingerprint": "SHA256:…", "key_type": "ED25519", "origin": "generated here",
      "confirm_each_use": true, "created": 1795130000 }
  ],
  "forwards": [
    { "id": "banco-de-staging", "name": "Banco de staging", "kind": "local",
      "listen": "localhost:5433", "target": "db-staging:5432", "host": "app-staging",
      "with_host": true, "loopback_only": true }
  ]
}
```

### Campos

| Onde | Campo | O quê |
|---|---|---|
| `hosts[]` | `id` | Identificador estável (letras minúsculas, números e `-`). É o que vai em `terminal-ssh`. |
| | `name` | O nome que a pessoa vê (título da aba). |
| | `address`, `port` | Endereço e porta (22 por padrão). |
| | `group` | Id do grupo, ou `null`. |
| | `tags` | Etiquetas. `produção` (ou `producao`, `production`, `prod`) marca a aba como produção. |
| | `type` | `"ssh"` ou `"mosh"`. |
| | `user`, `identity`, `jump`, `keepalive`, `forward_agent` | `null` = herda do grupo (e dos grupos acima). Um valor aqui é uma exceção só deste servidor. |
| | `confirm` | Pedir confirmação antes de conectar (o Kalango Remote pede; o Terminal confirma comandos destrutivos nessas abas). |
| | `last_used` | Segundos Unix. |
| `groups[]` | `parent` | Grupos aninhados. O valor efetivo é o do servidor; senão o do grupo mais próximo que tiver; senão o padrão (usuário local, sem chave, sem salto, keepalive 0). Um ciclo de grupos para a busca. |
| `identities[]` | `kind` | `vault-key` (chave privada no cofre), `vault-password` (senha no cofre), `agent` (chave do ssh-agent do sistema, inclusive chaves físicas). Os segredos **nunca** estão neste arquivo. |
| `forwards[]` | `kind` | `local` (`-L listen:target`), `remote` (`-R listen:target`), `dynamic` (`-D listen`, SOCKS). |
| | `loopback_only` | Escuta só em 127.0.0.1. |

`jump` aponta para o `id` de outro servidor; uma cadeia de saltos vira `ProxyCommand` aninhados (o
`ProxyJump` do OpenSSH não passa o `known_hosts` nem a chave para o ssh de dentro). Um ciclo de saltos é
cortado.

## known_hosts

`$XDG_CONFIG_HOME/kalango/remote/known_hosts`, no formato do OpenSSH. "Importar config e known_hosts" copia
as linhas do `~/.ssh/known_hosts` (hasheadas também). Chave nova só entra pelo diálogo de primeira conexão
do Kalango Remote; chave mudada só sai por "Servidores conhecidos › Remover", que pede a senha de
administrador. O agente não tem nenhuma das duas ações.

## O que foi testado

`kalangoos/scripts/testar-ssh.sh` sobe um `sshd` descartável (usuário comum, porta livre, chave de
servidor do teste) e confere: importar `~/.ssh/config` (com `IdentityFile` indo para o cofre e `ProxyJump`
virando `jump`), recusa com chave desconhecida, conexão com a chave do cofre direto e pelo salto, um
encaminhamento local pelo salto, e o bloqueio quando a chave do servidor muda. Mosh não foi testado (o
`mosh` não está instalado nesta máquina).
