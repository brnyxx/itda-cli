<p align="center"><img src="assets/hero.png" alt="itda: uma mão passa um pequeno cartão de site a um agente de IA ao longo de um fio. Digite um endereço depois de itda e sua IA usa o site." width="100%"></p>

<p align="center"><strong>Digite um endereço depois de itda e sua IA usa o site.</strong></p>

<p align="center"><a href="README.md">English</a> | <a href="README.ko.md">한국어</a> | <a href="README.ja.md">日本語</a> | <a href="README.zh-Hans.md">简体中文</a> | <a href="README.zh-Hant.md">繁體中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.de.md">Deutsch</a> | <b>Português (Brasil)</b> | <a href="README.vi.md">Tiếng Việt</a> | <a href="README.id.md">Bahasa Indonesia</a> | <a href="README.ru.md">Русский</a></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/docs/">Documentação</a>
</p>

Dê um endereço ao itda. Ele aprende o que o site pode fazer e experimenta. Dá ao
seu agente de IA uma lista curta de ações em palavras simples. Leituras
simplesmente funcionam. Nada muda no site até você dizer sim. Roda no seu
próprio computador, e funciona até na intranet antiga atrás de um login.

É para os sites que sua empresa mantém e os serviços em que você tem conta, não
para a web inteira.

<p align="center"><img src="assets/setup.gif" alt="Uma primeira execução: o itda pede um endereço, news.ycombinator.com é colado, ele encontra uma lista de 30 registros, um formulário e um feed, tenta as duas leituras, salva o site e conecta o Claude Desktop depois de um só Enter" width="860"></p>

<details>
<summary>A mesma execução como texto</summary>

```console
$ itda news.ycombinator.com
1/5  Olhando news.ycombinator.com...
    A página abre sem login.
    Encontrado: uma lista de 30 registros, um formulário, um feed.
2/5  O que os agentes vão poder fazer:
    leitura   home         List what Hacker News shows  (entradas: p)
    leitura   feed         Read the RSS feed
3/5  Tentando cada leitura uma vez...
    ✓ home         30 registros
    ✓ feed         ok
4/5  Salvo como 'news-ycombinator'.
    Conectar Claude Desktop? [Y/n]
5/5  Adicionando o itda ao Claude Desktop...
Adicionou o itda ao Claude Desktop (~/Library/Application Support/Claude/claude_desktop_config.json).
Abra o Claude Desktop e o itda estará lá.

Pronto. Peça ao seu agente algo sobre 'news-ycombinator', por exemplo "use news-ycombinator para List what Hacker News shows".
```

</details>

Peça ao seu agente: *"use news-ycombinator para listar o que o Hacker News
mostra"*. Um site atrás de um formulário de login pede, em vez disso, seu nome de
usuário e senha. O itda preenche o formulário do próprio site, como itda, e
guarda a sessão no seu computador. Se isso não bastar (login único, um código de
uso único), seu próprio navegador abre. Você entra e usa o site como sempre
enquanto o itda observa.

<p align="center"><img src="assets/flow.svg" alt="Grave um site uma vez no seu navegador; o itda rascunha comandos a partir dele. O agente os chama; uma alteração espera a aprovação de uma pessoa." width="860"></p>

> Status: inicial, e usado contra sites reais. O runtime, a CLI, o servidor MCP,
> as aprovações (num terminal ou dentro do app do agente), o log de auditoria,
> os rascunhos a partir de documentos OpenAPI, gravações do navegador ou páginas
> baixadas, chaves de API, login pelo navegador e os passos prévios de CSRF
> funcionam hoje. Pacotes de site assinados vêm a seguir. Veja
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Por quê

Agentes que controlam um navegador releem cada página e gastam milhares de
tokens em cada captura de tela ou snapshot de acessibilidade. Clicam no que a
página manda e quebram em silêncio quando o layout muda. O itda move esse
trabalho para um passo de instalação:

| | Agente que controla um navegador | itda |
|---|---|---|
| O que o agente vê | Páginas inteiras, a cada passo | O resultado tipado de uma ação, limitado a 24 KB |
| Uma página alterada ou deslogada | Lida como dados | Detectada pela resposta declarada (`response_mismatch`, `auth_required`) |
| Alterações no site | O que o modelo decidir | Aprovação de uso único para entradas exatas, concedida por uma pessoa num terminal ou no app do agente |
| Texto na página dizendo "ignore as instruções anteriores" | No prompt | Marcado como dado não confiável, nunca uma ferramenta |
| Credenciais | Digitadas na página pelo modelo | No chaveiro do sistema; o agente só vê ids de conta |
| Ao ser bloqueado | Tentar de novo, trocar IP, imitar um navegador | Parar, avisar você, esperar você retomar |
| O que aconteceu | Histórico do chat | Log de auditoria encadeado por hash que você pode verificar |

## O que ele pode e não pode fazer

Cada linha aqui é verdade sobre o código de hoje. `itda setup` diz com qual caso
se deparou; `itda draft page` ou `itda record` dizem o que deixaram de fora.

**Funciona hoje**

- Páginas renderizadas no servidor: tabelas, listas e cartões repetidos viram
  registros, e uma página que é uma coisa só vira uma leitura dos seus campos.
  Formulários de busca viram leituras, formulários de envio viram alterações, e
  os feeds vêm junto.
- Páginas que embutem seus dados para seus scripts (`__NEXT_DATA__`,
  `ld+json`, um estado do Apollo ou pré-carregado): uma leitura por lista
  contida nelas.
- Páginas desenhadas por scripts que chamam uma API que responde JSON, um
  portal em React ou Vue, digamos: `itda setup` abre seu navegador na página e
  observa o que ela carrega sozinha. Rascunha a partir dessas chamadas. Não
  clica nem digita.
- Formulários que querem um token CSRF: um passo prévio lê um novo da página do
  formulário logo antes de cada envio aprovado.
- Páginas que são uma casca em volta de frames: o conteúdo fica num `<iframe>`
  do mesmo site, muitas vezes atrás do próprio login. O itda lê cada frame como
  uma página do site e entra primeiro quando o frame pede.
- Sessões por cookie: um login pelo formulário do próprio site com seu nome de
  usuário e senha, ou seu próprio navegador para login único e códigos de uso
  único. Uma sessão carregada no endereço (`?sid=...`) funciona do mesmo jeito.
- Uma descrição OpenAPI ou Swagger que o site publica, e um endpoint GraphQL
  que responde à introspecção: uma leitura por operação ou campo de consulta.
- Sites que rodam GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki,
  WordPress, Gitea ou Forgejo: a própria API deles, a partir de um modelo
  pronto.
- Qualquer coisa que você gravou usando, no seu navegador ou como arquivo HAR.

**Funciona depois de gravar**

- Uma página que não mostra nada numa busca simples e não carrega nada sozinha.
  Só busca dados depois que você clica, pesquisa ou entra. `itda record` observa
  você usá-la e rascunha a partir das chamadas que viu.

**Não funciona**

- WebSocket. O itda fala HTTP puro, e uma gravação não guarda tráfego de
  socket.
- APIs que assinam cada requisição (uma assinatura ou nonce calculado por
  chamada). O itda guarda um token fixo ou chave de API no chaveiro. Não
  reproduz um valor que muda a cada chamada, então o site recusa a requisição.
- Páginas montadas de fragmentos HTML (htmx e parecidos). Não por desenho: o
  itda ainda pode ler um fragmento que contenha uma tabela inteira, mas não o
  que o script faz com ele.
- Conteúdo desenhado num canvas ou numa imagem. O itda lê texto em HTML, JSON e
  XML; não olha pixels.
- Sites atrás de um desafio ou de um gerenciador de bots (Cloudflare, Akamai,
  um captcha). O itda se identifica e nunca imita um navegador. Pausa o site até
  você retomá-lo. Veja [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- Qualquer coisa que sua conta não pode ver. O itda não tem conta própria; usa
  a sua.

## Quando o site muda

`itda check <site>` roda cada leitura que não precisa de entrada e nunca envia
uma alteração. Termina com um só veredito: `ok`, `signed_out`, `site_changed`,
`blocked` ou `unreachable`. O código de saída o acompanha (11 para
`site_changed`), então uma tarefa agendada pode avisar você antes que um agente
tropece nisso. Por MCP, `itda_check` devolve o mesmo.

Quando o veredito é `site_changed`, `itda refresh <site>` rascunha o site de novo
a partir do que foi usado para configurá-lo: suas páginas, a descrição da API, o
endpoint GraphQL, ou a forma guardada da gravação, com todos os valores em
branco. Mostra o que mudaria em cada ação e não grava nada até você dizer sim. O
itda mantém as ações que você editou à mão e atualiza as que o novo rascunho
alterou. Só descarta uma ação que as fontes não rascunham mais se a leitura dela
também falhou, e tenta cada ação nova uma vez, adicionando-a se responder. O
itda retira as aprovações que esperavam pelo site e verifica o site de novo logo
depois. Uma definição instalada com `itda add` não tem fontes e não pode ser
atualizada; edite-a à mão.

Essa é a troca. Um agente de navegador se adapta a cada execução, e por isso é
lento e não determinístico. O itda é uma requisição e a mesma resposta toda vez.
Quebra em voz alta quando o site muda.

## Ao lado dos agentes de navegador

Playwright MCP, browser-use e uso do computador controlam um navegador a cada
tarefa. O itda não os substitui: uma página que você vai usar uma vez, ou um
site da lista acima que o itda não cobre, é deles. O itda é para o site que um
agente vai chamar de novo e de novo. Grave uma vez num navegador, revise as
ações e depois execute-as por HTTP com uma resposta tipada, uma aprovação de uso
único para cada alteração e um log. Use os dois: um agente de navegador para
explorar, o itda para as chamadas que rodam todo dia.

## Início rápido

```sh
# macOS e Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

Baixe um arquivo em
[Releases](https://github.com/brnyxx/itda-cli/releases), ou compile do código
fonte com `cargo install --git https://github.com/brnyxx/itda-cli itda-cli`.

Claude Desktop, sem terminal: baixe o pacote para
[macOS (Apple silicon)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-aarch64-apple-darwin.mcpb),
[macOS (Intel)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-apple-darwin.mcpb) ou
[Windows](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-pc-windows-msvc.mcpb) e abra o arquivo. O Claude
Desktop instala o itda com suas ferramentas, e "use the-site-you-use.example"
configura um site de dentro da conversa. Aprovar uma alteração ainda acontece
num terminal com `itda approve`, então para alterações instale também o comando.

Digite `itda` e o endereço de um site que você usa. Ele encontra os apps de
agente no seu computador e os conecta depois de um só Enter. Ele fala doze idiomas: `LANG`
escolhe um e `ITDA_LANG` tem prioridade. Um `itda` sozinho pede o endereço;
`itda setup <address>` aceita opções para scripts.

O itda reconhece sites que rodam software conhecido. Quando a página diz que é
um GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki, WordPress ou Gitea
(também Forgejo), e uma requisição à API própria desse software confirma, o
agente recebe a API real desse software a partir de um modelo pronto
([`sites/software/`](sites/software/)), com o login do jeito que esse software
faz. O que a própria página mostra se junta a isso. `itda setup --as jira` força
um modelo; `--no-template` lê a página como qualquer outra.

Uma página desenhada por scripts mostra ao itda só um título, então o itda abre
seu navegador nela por alguns segundos. Observa o que a página carrega sozinha e
rascunha a partir dessas chamadas. Nada é clicado nem digitado, e a janela fecha
sozinha. Só uma página que continua não mostrando nada, ou que precisa de login,
pede que você use o site no seu navegador enquanto o itda observa.

Com um agente conectado, você também pode simplesmente pedir a ele: "configure
news.ycombinator.com para mim". Ele chama `itda_setup`. O itda olha o site,
rascunha e experimenta do mesmo jeito. Suas ações aparecem como ferramentas na
mesma conversa. Uma página com scripts abre brevemente numa janela do navegador
na sua tela. Quando uma leitura pede login, `itda_login` abre seu próprio
navegador; o agente só vê o id da conta.

Pule o terminal e entregue isto ao Claude Code, Cursor ou Codex, com o endereço
de um site que você usa:

```text
Instale o itda e configure the-site-you-use.example para mim.
1. Instale: curl -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
   (no Windows: irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex)
2. Execute: itda setup https://the-site-you-use.example --client claude-code -y
   (use cursor, codex ou claude-desktop se for o que você é)
3. Leia https://github.com/brnyxx/itda-cli/blob/main/AGENTS.md e depois use o site pelo itda.
   Leituras simplesmente funcionam. Qualquer coisa que altere o site espera minha aprovação.
   Se uma leitura pedir login, me diga para executar: itda login <site> --browser
```

As peças são comandos por si só. Para páginas que você abre sem login, dê ao
itda os endereços delas. Tabelas, listas e cartões repetidos viram registros.
Uma página que é uma coisa só (um pacote, um artigo, um produto) vira uma
leitura de seu título, corpo e valores rotulados. Uma lista cujos links
compartilham uma forma ganha uma leitura de um registro pelo id. Formulários de
busca viram leituras; formulários de envio viram alterações. Os feeds vêm junto.
Um endpoint GraphQL que a página nomeia é perguntado uma vez sobre o que
oferece, uma leitura por campo de consulta. Quando o site publica uma descrição
de API, as leituras dela entram no rascunho:

```sh
itda draft page https://example.com/board --install   # mostra o que encontrou na página
itda call example.get-board --page 2 --json
itda check example                                     # a página ainda corresponde?
```

Para um site atrás de um formulário de login, dê ao `itda setup` a página e seu
nome de usuário. Ele envia o formulário do próprio site uma vez, como itda, com
a senha que você digita. Guarda a sessão no chaveiro do sistema e rascunha a
partir da página como você a vê logado. A definição lembra o formulário, então
`itda login board` entra de novo quando a sessão expira:

```sh
itda setup https://intranet.example/board --name board --user me   # pede a senha
itda call board.list-notices --json                 # leituras rodam na hora
itda call board.create-notice --title "Server maintenance"   # alterações esperam sua aprovação
itda login board --user me                          # quando a sessão expira
```

Para login único ou um código de uso único, grave você mesmo usando o site. O
itda abre seu próprio navegador numa janela nova. Observa o que as páginas pedem
ao servidor enquanto você entra, pesquisa e envia um formulário; nunca clica nem
digita:

```sh
itda record https://intranet.example/board --name board --install   # aperte Enter quando terminar
                                                    # depois diga sim para manter a sessão ou o token que ele usou
itda mcp install claude-desktop                     # ou deixe seu agente chamá-las
```

`examples/lab` roda Gitea, Redmine, WordPress, MediaWiki, uma pequena intranet
antiga e três casos mais feios atrás de seus logins: um quadro dentro de um
iframe, uma tabela que um script desenha a partir de um XHR, e uma sessão
carregada na URL. Uma suíte de testes percorre cada um da configuração até uma
alteração aprovada.

A gravação fica na memória e só a forma de cada chamada chega ao rascunho:
nenhum cabeçalho, cookie ou valor que você digitou. Um arquivo HAR salvo das
ferramentas de desenvolvedor do navegador funciona do mesmo jeito com
`itda draft har`.

APIs públicas não precisam de gravação. Estes sites prontos de
[sites/](sites/README.md) vêm embutidos e não precisam de conta:

<p><a href="sites/github.yaml"><img src="assets/brands/github.svg" width="36" height="36" alt="GitHub"></a> <a href="sites/npm.yaml"><img src="assets/brands/npm.svg" width="36" height="36" alt="npm"></a> <a href="sites/crates-io.yaml"><img src="assets/brands/crates-io.svg" width="36" height="36" alt="crates.io"></a> <a href="sites/pypi.yaml"><img src="assets/brands/pypi.svg" width="36" height="36" alt="PyPI"></a></p>

```sh
itda add github      # ou npm, crates-io, pypi
itda call github.repo --owner rust-lang --repo rust
```

Experimente o sandbox incluído, uma intranet falsa que só escuta em 127.0.0.1:
siga [examples/sandbox](examples/sandbox/README.md). Uma definição de site é um
único arquivo YAML. `itda add spec.yaml` a instala depois de mostrar para onde
envia requisições e quais ações alteram o site. `itda doctor` imprime a
localização do diretório de dados. [docs/SPEC.md](docs/SPEC.md) explica como
escrever uma definição.

```sh
itda setup <url>                         # página ou gravação -> ações nomeadas -> testadas -> agente, com perguntas
itda draft page <url>... [--install]     # rascunhe um site a partir das páginas que você usa
itda record <url> [--install]            # ou a partir de usá-lo no seu navegador, atrás de um login
itda draft har <file> / openapi <file>   # ou a partir de uma gravação salva ou de uma descrição de API
itda add spec.yaml / remove <site>       # instale ou remova um site (também uma URL ou github)
itda ls                                  # sites instalados e suas ações
itda describe <site>                     # suas entradas e um comando para copiar de cada uma
itda call <site>.<action> --input value  # rode uma; --json para agentes
itda login <site> --user me              # entre pelo formulário do próprio site (senha pelo stdin ou pergunta)
echo "$KEY" | itda login <site> --secret api-key   # ou guarde uma chave de API no chaveiro do sistema
itda login <site> --browser              # ou entre no seu próprio navegador, uma vez (selada no diretório de dados)
itda accounts / logout <site>            # veja ou remova logins salvos
itda approvals / approve <id> / reject <id>
itda audit [--verify]                    # o que os agentes fizeram
itda check [<site>|--all]                # cada leitura ainda corresponde ao site?
itda refresh <site>                      # um site mudou: rascunhe de novo, mantenha o que você decidiu
itda resume <site>                       # depois que um site recusou ou uma chave expirou
itda feedback                            # relate um problema: abre um issue novo, já preenchido
```

## Use a partir de um agente

```sh
itda mcp install claude-desktop   # ou cursor, vscode, codex
itda mcp install claude-code      # imprime o comando `claude mcp add`
# adicione --approve-in-client para aprovar alterações no formulário do próprio cliente
# itda mcp --site <name> serve um só site, para a configuração própria de um projeto
```

O instalador se funde à configuração do cliente e mantém seus outros servidores.
Faz backup do arquivo antigo como `.bak`. `--dry-run` mostra a mudança antes.
`itda mcp` serve cada site instalado:

- Até 30 ações aparecem como uma ferramenta cada (`sandbox.list-notices`), com
  esquemas de entrada e dicas de somente leitura, destrutiva e idempotente.
- Acima disso, três ferramentas estáveis (`itda_search`, `itda_describe`,
  `itda_call`) mantêm a lista de ferramentas curta e os caches de prompt
  válidos. `--mode` sobrepõe isso.
- Os resultados são o mesmo envelope JSON de `itda call --json`, marcados como
  não confiáveis.
- Uma alteração devolve `approval_required` com um id. Nenhuma ferramenta
  aprova; o agente tem que pedir a você.

Agentes que só têm um shell podem chamar `itda call … --json` e se ramificar
pelo código de saída:

| Saída | Significado |
|---|---|
| 0 | ok |
| 2 | entrada inválida ou ação desconhecida |
| 10 | entre de novo (`itda login`) |
| 11 | a página não corresponde mais à sua definição |
| 12 | o site recusou; pausado até `itda resume` |
| 13 | limitado por frequência |
| 14 | o site recusou esta requisição específica como não permitida; entrar de novo não vai ajudar |
| 20 | precisa de aprovação, ou o usuário ainda não respondeu (ou as alterações estão desligadas) |
| 21 | aprovação usada, recusada, expirada ou retirada porque as entradas mudaram |
| 22 | o itda enviou uma alteração mas o resultado é desconhecido; verifique o site |
| 30 | erro de rede |

## Princípios

- **Local primeiro.** As definições rodam no seu dispositivo. As credenciais
  nunca saem dele.
- **Somente leitura por padrão.** Uma ação é uma leitura só quando seu método,
  nome, caminho, entradas e documento GraphQL concordam nisso. Tudo o mais é
  uma alteração.
- **Pessoas aprovam alterações.** Cada aprovação cobre uma requisição com
  entradas, conta, definição e credencial exatas. Funciona uma vez e expira.
- **Bloqueios são informados, não contornados.** O itda se identifica e nunca
  imita um navegador. Pausa um site que o recusa. Veja
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- **Ações verificadas.** Uma definição declara como é uma resposta correta,
  então "deslogado" e "o site mudou" são erros, não dados.

## Estrutura

Os crates dependem só dos crates abaixo deles; `cargo xtask check-layers` garante
isso.

| Camada | Crate | Propósito |
|---|---|---|
| 0 | `itda-types` | Contrato de erros, verificações de URL (Public Suffix List), redação |
| 1 | `itda-html`, `itda-store` | Extração de HTML sem scripts; o armazenamento SQLite local |
| 2 | `itda-spec` | Formato de definição, construção de requisições, verificação de respostas |
| 3 | `itda-policy` | Classificação leitura/escrita, verificações de segurança de definições, aprovações |
| 4 | `itda-net` | HTTP limitado, cadência, detecção de bloqueio, proteção contra endereços privados |
| 5 | `itda-vault` | Contas e segredos no chaveiro do sistema ou no ambiente |
| 7 | `itda-runtime` | Roda ações: credenciais, aprovações, modelagem, log de auditoria |
| 9 | `itda-setup` | Os passos de configuração, login e verificação, para a CLI e o servidor MCP igualmente |
| 10 | `itda-mcp` | Servidor MCP sobre stdio |
| 11 | `apps/cli` | O comando `itda` |

## Desenvolvimento

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check-layers
```

`rust-toolchain.toml` fixa o Rust 1.98 (edição 2024). Os testes só falam com
servidores locais que eles mesmos iniciam. As regras de commit e pull request
estão em [CONTRIBUTING.md](CONTRIBUTING.md).

## Segurança

[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) lista do que o itda se defende e
do que não. Relate vulnerabilidades como descrito em [SECURITY.md](SECURITY.md).

## Créditos e licença

Licenciado sob [Apache-2.0](LICENSE). Partes deste projeto derivam do
[Hycli](https://github.com/Hybirdss/Hycli) (Apache-2.0); veja [NOTICE](NOTICE).
Cada arquivo derivado diz isso em seu cabeçalho e descreve o que mudou. Este
projeto não é afiliado ao Hycli e não usa seu nome nem suas marcas.
