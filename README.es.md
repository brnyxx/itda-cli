<p align="center"><img src="assets/hero.png" alt="itda: una mano pasa una pequeña tarjeta de sitio web a un agente de IA a lo largo de un hilo. Escribe una dirección después de itda y tu IA usa el sitio." width="100%"></p>

<p align="center"><strong>Escribe una dirección después de itda y tu IA usa el sitio.</strong></p>

<p align="center"><a href="README.md">English</a> | <a href="README.ko.md">한국어</a> | <a href="README.ja.md">日本語</a> | <a href="README.zh-Hans.md">简体中文</a> | <a href="README.zh-Hant.md">繁體中文</a> | <b>Español</b> | <a href="README.fr.md">Français</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pt-BR.md">Português (Brasil)</a> | <a href="README.vi.md">Tiếng Việt</a> | <a href="README.id.md">Bahasa Indonesia</a> | <a href="README.ru.md">Русский</a></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/docs/">Documentación</a>
</p>

Dale a itda una dirección. Aprende lo que el sitio puede hacer y lo prueba. Le
da a tu agente de IA una lista corta de acciones en palabras sencillas. Las
lecturas simplemente funcionan. Nada cambia en el sitio hasta que dices sí.
Funciona en tu propio ordenador, y también en la vieja intranet detrás de un
inicio de sesión.

Es para los sitios que gestiona tu empresa y los servicios donde tienes
cuenta, no para toda la web.

<p align="center"><img src="assets/setup.gif" alt="Una primera ejecución: itda pide una dirección, se pega news.ycombinator.com, encuentra una lista de 30 registros, un formulario y un feed, prueba ambas lecturas, guarda el sitio y conecta Claude Desktop tras un solo Enter" width="860"></p>

<details>
<summary>La misma ejecución como texto</summary>

```console
$ itda news.ycombinator.com
1/5  Mirando news.ycombinator.com...
    La página se abre sin iniciar sesión.
    Encontrado: una lista de 30 registros, un formulario, un feed.
2/5  Lo que los agentes podrán hacer:
    lectura home         List what Hacker News shows  (entradas: p)
    lectura feed         Read the RSS feed
3/5  Probando cada lectura una vez...
    ✓ home         30 registros
    ✓ feed         ok
4/5  Guardado como 'news-ycombinator'.
    ¿Conectar Claude Desktop? [Y/n]
5/5  Añadiendo itda a Claude Desktop...
Se añadió itda a Claude Desktop (~/Library/Application Support/Claude/claude_desktop_config.json).
Abre Claude Desktop y ahí estará itda.

Listo. Pide a tu agente algo sobre 'news-ycombinator', por ejemplo "usa news-ycombinator para List what Hacker News shows".
```

</details>

Pídele a tu agente: *"usa news-ycombinator para listar lo que muestra Hacker
News"*. Un sitio detrás de un formulario de inicio de sesión te pide en cambio
tu nombre de usuario y contraseña. itda rellena el formulario del propio sitio
como itda y guarda la sesión en tu ordenador. Si eso no basta (inicio de sesión
único, un código de un solo uso), se abre tu propio navegador. Inicias sesión y
usas el sitio como siempre mientras itda observa.

<p align="center"><img src="assets/flow.svg" alt="Graba un sitio web una vez en tu navegador; itda redacta comandos a partir de ello. El agente los llama; un cambio espera la aprobación de una persona." width="860"></p>

> Estado: temprano, y usado contra sitios reales. El runtime, la CLI, el
> servidor MCP, las aprobaciones (en una terminal o dentro de la aplicación del
> agente), el registro de auditoría, los borradores a partir de documentos
> OpenAPI, grabaciones del navegador o páginas descargadas, las claves de API,
> el inicio de sesión en el navegador y los pasos previos de CSRF funcionan
> hoy. Lo siguiente son los paquetes de sitio firmados. Consulta
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Por qué

Los agentes que manejan un navegador releen cada página y gastan miles de
tokens en cada captura de pantalla o instantánea de accesibilidad. Hacen clic
en lo que la página les dice y fallan en silencio cuando cambia el diseño. itda
traslada ese trabajo a un paso de instalación:

| | Agente que maneja un navegador | itda |
|---|---|---|
| Lo que ve el agente | Páginas enteras, en cada paso | El resultado tipado de una acción, limitado a 24 KB |
| Una página cambiada o con la sesión cerrada | Se lee como datos | Se detecta a partir de la respuesta declarada (`response_mismatch`, `auth_required`) |
| Cambios en el sitio | Lo que decida el modelo | Aprobación de un solo uso para entradas exactas, concedida por una persona en una terminal o en la aplicación del agente |
| Texto en la página que dice "ignora las instrucciones anteriores" | En el prompt | Marcado como datos no confiables, nunca como herramienta |
| Credenciales | El modelo las escribe en la página | En el llavero del sistema; el agente solo ve ids de cuenta |
| Cuando lo bloquean | Reintentar, rotar, imitar un navegador | Intentar la lectura una vez más en tu propio navegador instalado, recordar lo que funcionó, no pausar nada |
| Lo que pasó | Historial del chat | Registro de auditoría encadenado por hash que puedes verificar |

## Qué puede y qué no puede hacer

Cada línea de aquí es cierta del código de hoy. `itda setup` dice con qué caso
se encontró; `itda draft page` o `itda record` dicen qué dejaron fuera.

**Funciona hoy**

- Páginas dibujadas en el servidor: las tablas, listas y tarjetas repetidas se
  convierten en registros, y una página que es una sola cosa se convierte en
  una lectura de sus campos. Los formularios de búsqueda se convierten en
  lecturas, los de envío en cambios, y los feeds vienen de paso.
- Páginas que incrustan sus datos para sus scripts (`__NEXT_DATA__`,
  `ld+json`, un estado de Apollo o precargado): una lectura por cada lista que
  contienen.
- Páginas dibujadas por scripts que llaman a una API que responde JSON, un
  portal en React o Vue, digamos: `itda setup` abre tu navegador en la página y
  observa lo que carga por sí sola. Redacta el borrador a partir de esas
  llamadas. No hace clic ni escribe.
- Formularios que quieren un token CSRF: un paso previo lee uno nuevo de la
  página del formulario justo antes de cada envío aprobado.
- Páginas que son una envoltura alrededor de marcos: el contenido está en un
  `<iframe>` del mismo sitio, a menudo detrás de su propio inicio de sesión.
  itda lee cada marco como una página del sitio e inicia sesión primero cuando
  el marco lo pide.
- Sesiones por cookie: un inicio de sesión a través del formulario del propio
  sitio con tu nombre de usuario y contraseña, o tu propio navegador para el
  inicio de sesión único y los códigos de un solo uso. Una sesión que viaja en
  la dirección (`?sid=...`) funciona igual.
- Una descripción OpenAPI o Swagger que publique el sitio, y un endpoint
  GraphQL que responda a la introspección: una lectura por operación o campo
  de consulta.
- Sitios que ejecutan GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki,
  WordPress, Gitea o Forgejo: su propia API, desde una plantilla ya hecha.
- Archivos de entrada y de salida: un enlace que la página marca como adjunto
  se convierte en una lectura que guarda el archivo en tu ordenador y devuelve
  su ruta, tamaño y hash, nunca los bytes. Un formulario con un campo de
  archivo se convierte en un cambio que envía un archivo que tú indicas; lo
  apruebas por nombre, tamaño y hash.
- Cualquier cosa que te hayas grabado usando, en tu navegador o como archivo
  HAR.
- Páginas detrás de una página de desafío, cuando el navegador de tu
  ordenador lo supera. itda intenta la lectura una vez más dentro de ese
  navegador, oculto, reservado para ese sitio y esa cuenta, y recuerda lo que
  funcionó, así que la siguiente llamada empieza ahí. El navegador sigue
  caliente entre llamadas de la CLI y se cierra tras cinco minutos sin uso;
  cambiar un término de búsqueda no lo reinicia. Todo lo que carga la página
  pasa las mismas comprobaciones de dirección y de lectura que una petición
  directa. Consulta [la política de transporte](docs/HONEST_CLIENT.md).

**Funciona después de grabarlo**

- Una página que no muestra nada con una descarga simple y no carga nada por sí
  sola. Solo descarga datos después de que haces clic, buscas o inicias sesión.
  `itda record` te observa usarla y redacta el borrador a partir de las
  llamadas que vio.

**No funciona**

- WebSocket. itda habla HTTP plano, y una grabación no conserva el tráfico de
  sockets.
- APIs que firman cada petición (una firma o un nonce calculado por llamada).
  itda guarda un token fijo o una clave de API en el llavero. No reproduce un
  valor que cambia con cada llamada, así que el sitio rechaza la petición.
- Páginas armadas con fragmentos de HTML (htmx y similares). No por diseño:
  itda puede leer igualmente un fragmento que contenga una tabla entera, pero
  no lo que el script hace con él.
- Contenido dibujado en un canvas o en una imagen. itda lee texto en HTML, JSON
  y XML; no mira píxeles.
- Un desafío que tu navegador tampoco supera, o un captcha. itda no resuelve
  captchas. La lectura falla y lo dice; nada queda en pausa. Consulta
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- Cualquier cosa que tu cuenta no pueda ver. itda no tiene cuenta propia; usa
  la tuya.

## Cuando el sitio cambia

`itda check <site>` ejecuta cada lectura que no necesita entradas y nunca envía
un cambio. Termina con un solo veredicto: `ok`, `signed_out`, `site_changed`,
`blocked` o `unreachable`. El código de salida lo sigue (11 para
`site_changed`), así que una tarea programada puede avisarte antes de que un
agente se tope con ello. Por MCP, `itda_check` devuelve lo mismo.

Cuando el veredicto es `site_changed`, `itda refresh <site>` redacta el sitio de
nuevo a partir de aquello con lo que se configuró: sus páginas, la descripción
de la API, el endpoint GraphQL, o la forma conservada de la grabación, con todos
los valores en blanco. Muestra lo que cambiaría en cada acción y no escribe nada
hasta que dices sí. itda conserva las acciones que editaste a mano y actualiza
las que el nuevo borrador cambió. Quita una acción que las fuentes ya no
producen solo si su lectura también falló, y prueba cada acción nueva una vez,
añadiéndola si responde. itda retira las aprobaciones que esperaban al sitio y
vuelve a comprobar el sitio justo después. Una definición instalada con
`itda add` no tiene fuentes y no se puede refrescar; edítala a mano.

Ese es el compromiso. Un agente de navegador se adapta en cada ejecución, y por
eso es lento y no determinista. itda es una petición y la misma respuesta cada
vez; tras un desafío es un intento más en tu navegador, y sigue siendo la
misma respuesta. Falla a lo grande cuando el sitio se mueve.
`docs/TRANSPORT_BENCHMARKS.md` tiene las cifras, incluida la que itda aún no
cumple.

## Junto a los agentes de navegador

Playwright MCP, browser-use y el uso del ordenador manejan un navegador en cada
tarea. itda no los reemplaza: una página que usarás una sola vez, o un sitio de
la lista de arriba que itda no cubre, es cosa suya. itda es para el sitio que un
agente llamará una y otra vez. Grábalo una vez en un navegador, revisa las
acciones y luego ejecútalas por HTTP con una respuesta tipada, una aprobación de
un solo uso para cada cambio y un registro. Usa ambos: un agente de navegador
para explorar, itda para las llamadas que se ejecutan cada día.

## Guía rápida

```sh
# macOS y Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

Descarga un archivo desde
[Releases](https://github.com/brnyxx/itda-cli/releases), o compila desde el
código fuente con `cargo install --git https://github.com/brnyxx/itda-cli itda-cli`.

Claude Desktop, sin terminal: descarga el paquete para
[macOS (Apple silicon)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-aarch64-apple-darwin.mcpb),
[macOS (Intel)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-apple-darwin.mcpb) o
[Windows](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-pc-windows-msvc.mcpb) y abre el archivo. Claude
Desktop instala itda con sus herramientas, y "usa the-site-you-use.example"
configura un sitio desde dentro de la conversación. Aprobar un cambio sigue
ocurriendo en una terminal con `itda approve`, así que para los cambios instala
también el comando.

Escribe `itda` y la dirección de un sitio que uses. Encuentra las aplicaciones
de agente en tu ordenador y las conecta tras un solo Enter. Habla doce idiomas: `LANG`
elige uno e `ITDA_LANG` lo sobrescribe. Un `itda` a solas pide la dirección; `itda setup <address>`
acepta opciones para scripts.

itda reconoce los sitios que ejecutan software conocido. Cuando la página dice
que es un GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki, WordPress o
Gitea (también Forgejo), y una petición a la API propia de ese software lo
confirma, el agente recibe la API real de ese software desde una plantilla ya
hecha ([`sites/software/`](sites/software/)), con el inicio de sesión tal como
lo hace ese software. Lo que muestra la propia página se le suma.
`itda setup --as jira` fuerza una plantilla; `--no-template` lee la página como
cualquier otra.

Una página dibujada por scripts solo le muestra a itda un título, así que itda
abre tu navegador en ella durante unos segundos. Observa lo que la página carga
por sí sola y redacta el borrador a partir de esas llamadas. No se hace clic ni
se escribe nada, y la ventana se cierra sola. Solo una página que sigue sin
mostrar nada, o que necesita iniciar sesión, te pide que uses el sitio en tu
navegador mientras itda observa.

Una vez conectado un agente, también puedes pedírselo directamente: "configura
news.ycombinator.com para mí". Llama a `itda_setup`. itda mira el sitio, redacta
el borrador y lo prueba de la misma manera. Sus acciones aparecen como
herramientas en la misma conversación. Una página con scripts se abre
brevemente en una ventana del navegador en tu pantalla. Cuando una lectura dice
que hay que iniciar sesión, `itda_login` abre tu propio navegador; el agente
solo ve el id de la cuenta.

Sáltate la terminal y dale esto a Claude Code, Cursor o Codex, con la dirección
de un sitio que uses:

```text
Instala itda y configura the-site-you-use.example para mí.
1. Instálalo: curl -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
   (en Windows: irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex)
2. Ejecuta: itda setup https://the-site-you-use.example --client claude-code -y
   (usa cursor, codex o claude-desktop si eso es lo que eres)
3. Lee https://github.com/brnyxx/itda-cli/blob/main/AGENTS.md y luego usa el sitio a través de itda.
   Las lecturas simplemente funcionan. Todo lo que cambie el sitio espera mi aprobación.
   Si una lectura dice que hay que iniciar sesión, dime que ejecute: itda login <site> --browser
```

Las piezas son comandos por sí mismas. Para las páginas que puedes abrir sin
iniciar sesión, dale a itda sus direcciones. Las tablas, listas y tarjetas
repetidas se convierten en registros. Una página que es una sola cosa (un
paquete, un artículo, un producto) se convierte en una lectura de su titular,
cuerpo y valores etiquetados. Una lista cuyos enlaces comparten una misma forma
recibe una lectura de un registro por su id. Los formularios de búsqueda se
convierten en lecturas; los de envío, en cambios. Los feeds vienen de paso. A un
endpoint GraphQL que la página nombra se le pregunta una vez qué ofrece, una
lectura por campo de consulta. Cuando el sitio publica una descripción de API,
sus lecturas se suman al borrador:

```sh
itda draft page https://example.com/board --install   # muestra lo que encontró en la página
itda call example.get-board --page 2 --json
itda check example                                     # ¿la página sigue coincidiendo?
```

Para un sitio detrás de un formulario de inicio de sesión, dale a `itda setup`
la página y tu nombre de usuario. Envía el formulario del propio sitio una vez,
como itda, con la contraseña que escribes. Guarda la sesión en el llavero del
sistema y redacta el borrador a partir de la página tal como la ves con la
sesión iniciada. La definición recuerda el formulario, así que `itda login board`
inicia sesión de nuevo cuando la sesión caduca:

```sh
itda setup https://intranet.example/board --name board --user me   # pide la contraseña
itda call board.list-notices --json                 # las lecturas se ejecutan al momento
itda call board.create-notice --title "Server maintenance"   # los cambios esperan tu aprobación
itda login board --user me                          # cuando la sesión caduca
```

Para el inicio de sesión único o un código de un solo uso, grábate usando el
sitio en su lugar. itda abre tu propio navegador en una ventana nueva. Observa lo
que las páginas le piden al servidor mientras inicias sesión, buscas y envías un
formulario; nunca hace clic ni escribe:

```sh
itda record https://intranet.example/board --name board --install   # pulsa Enter cuando termines
                                                    # luego di sí para conservar la sesión o el token que usó
itda mcp install claude-desktop                     # o deja que tu agente las llame
```

`examples/lab` ejecuta Gitea, Redmine, WordPress, MediaWiki, una pequeña
intranet antigua y tres casos más feos detrás de sus inicios de sesión: un
tablón dentro de un iframe, una tabla que un script dibuja a partir de un XHR, y
una sesión que viaja en la URL. Una suite de pruebas recorre cada uno desde la
configuración hasta un cambio aprobado.

La grabación se queda en memoria y solo la forma de cada llamada llega al
borrador: ningún encabezado, cookie o valor que hayas escrito. Un archivo HAR
guardado desde las herramientas de desarrollo del navegador funciona igual con
`itda draft har`.

Las APIs públicas no necesitan grabación. Estos sitios ya hechos de
[sites/](sites/README.md) vienen incluidos y no necesitan cuenta:

<p><a href="sites/github.yaml"><img src="assets/brands/github.svg" width="36" height="36" alt="GitHub"></a> <a href="sites/npm.yaml"><img src="assets/brands/npm.svg" width="36" height="36" alt="npm"></a> <a href="sites/crates-io.yaml"><img src="assets/brands/crates-io.svg" width="36" height="36" alt="crates.io"></a> <a href="sites/pypi.yaml"><img src="assets/brands/pypi.svg" width="36" height="36" alt="PyPI"></a></p>

```sh
itda add github      # o npm, crates-io, pypi
itda call github.repo --owner rust-lang --repo rust
```

Prueba el sandbox incluido, una intranet falsa que solo escucha en 127.0.0.1:
sigue [examples/sandbox](examples/sandbox/README.md). Una definición de sitio es
un solo archivo YAML. `itda add spec.yaml` la instala después de mostrar a dónde
envía peticiones y qué acciones cambian el sitio. `itda doctor` imprime la
ubicación del directorio de datos. [docs/SPEC.md](docs/SPEC.md) explica cómo
escribir una definición.

```sh
itda setup <url>                         # página o grabación -> acciones con nombre -> probadas -> agente, con preguntas
itda draft page <url>... [--install]     # redacta un sitio a partir de las páginas que usas
itda record <url> [--install]            # o a partir de usarlo en tu navegador, detrás de un inicio de sesión
itda draft har <file> / openapi <file>   # o a partir de una grabación guardada o una descripción de API
itda add spec.yaml / remove <site>       # instala o quita un sitio (también una URL o github)
itda ls                                  # sitios instalados y sus acciones
itda describe <site>                     # sus entradas y un comando para copiar de cada una
itda call <site>.<action> --input value  # ejecuta una; --json para agentes
itda login <site> --user me              # inicia sesión a través del formulario del propio sitio (contraseña por stdin o pregunta)
echo "$KEY" | itda login <site> --secret api-key   # o guarda una clave de API en el llavero del sistema
itda login <site> --browser              # o inicia sesión en tu propio navegador, una vez (sellada bajo el directorio de datos)
itda accounts / logout <site>            # ve o quita los inicios de sesión guardados
itda approvals / approve <id> / reject <id>
itda audit [--verify]                    # qué hicieron los agentes
itda check [<site>|--all]                # ¿cada lectura sigue coincidiendo con el sitio?
itda refresh <site>                      # un sitio cambió: redáctalo de nuevo, conserva lo que decidiste
itda resume <site>                       # después de una pausa en modo HTTP directo, o de una clave caducada
itda feedback                            # informa de un problema: abre un issue nuevo, ya rellenado
```

## Úsalo desde un agente

```sh
itda mcp install claude-desktop   # o cursor, vscode, codex
itda mcp install claude-code      # imprime el comando `claude mcp add`
# añade --approve-in-client para aprobar cambios en el formulario propio del cliente
# itda mcp --site <name> sirve un solo sitio, para la configuración propia de un proyecto
```

El instalador se fusiona con la configuración del cliente y conserva tus otros
servidores. Hace una copia del archivo anterior como `.bak`. `--dry-run` muestra
el cambio primero. `itda mcp` sirve cada sitio instalado:

- Hasta 30 acciones aparecen como una herramienta cada una
  (`sandbox.list-notices`), con esquemas de entrada y pistas de solo lectura,
  destructiva e idempotente.
- Por encima de eso, tres herramientas estables (`itda_search`, `itda_describe`,
  `itda_call`) mantienen la lista de herramientas corta y las cachés de prompt
  válidas. `--mode` lo anula.
- Los resultados son el mismo sobre JSON que `itda call --json`, marcados como
  no confiables.
- Un cambio devuelve `approval_required` con un id. Ninguna herramienta aprueba;
  el agente tiene que pedírtelo.

Los agentes que solo tienen un shell pueden llamar a `itda call … --json` y
ramificar según el código de salida:

| Salida | Significado |
|---|---|
| 0 | ok |
| 2 | entrada incorrecta o acción desconocida |
| 10 | inicia sesión de nuevo (`itda login`) |
| 11 | la página ya no coincide con su definición |
| 12 | el sitio rechazó esta lectura, también en el navegador; `paused: false` significa que no hay nada que reanudar |
| 13 | limitado por frecuencia |
| 14 | el sitio rechazó esta petición concreta como no permitida; iniciar sesión de nuevo no ayudará |
| 20 | necesita aprobación, o el usuario aún no ha respondido (o los cambios están desactivados) |
| 21 | aprobación usada, rechazada, caducada o retirada porque las entradas cambiaron |
| 22 | itda envió un cambio pero su resultado es desconocido; con `outcome_check`, la lectura que el cambio nombra dice si se aplicó |
| 30 | error de red |

## Principios

- **Primero local.** Las definiciones se ejecutan en tu dispositivo. Las
  credenciales nunca salen de él.
- **Solo lectura por defecto.** Una acción es una lectura solo cuando su método,
  nombre, ruta, entradas y documento GraphQL coinciden en ello. Todo lo demás es
  un cambio.
- **Las personas aprueban los cambios.** Cada aprobación cubre una petición con
  entradas, cuenta, definición y credencial exactas. Funciona una vez y caduca.
- **Un desafío recibe un intento más, en tu navegador.** Una lectura que el
  sitio responde con una página de desafío se ejecuta una vez más dentro del
  navegador instalado en tu ordenador; itda recuerda lo que funcionó. Un cambio
  se envía una vez y nunca a través del navegador. `ITDA_TRANSPORT=http`
  mantiene solo HTTP directo. Consulta
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- **Acciones verificadas.** Una definición declara cómo es una respuesta
  correcta, así que "sesión cerrada" y "el sitio cambió" son errores, no datos.

## Estructura

Los crates dependen solo de los crates que están debajo; `cargo xtask check-layers`
lo hace cumplir.

| Capa | Crate | Propósito |
|---|---|---|
| 0 | `itda-types` | Contrato de errores, comprobaciones de URL (Public Suffix List), redacción |
| 1 | `itda-html`, `itda-store` | Extracción de HTML sin scripts; el almacén SQLite local |
| 2 | `itda-spec` | Formato de definición, construcción de peticiones, verificación de respuestas |
| 3 | `itda-policy` | Clasificación lectura/escritura, comprobaciones de seguridad de definiciones, aprobaciones |
| 4 | `itda-browser` | Inicio de sesión, grabación, transporte de lectura aislado en el navegador |
| 5 | `itda-net` | Transporte adaptativo, caché de estrategias, ritmo, protección de direcciones privadas |
| 6 | `itda-vault` | Cuentas y secretos en el llavero del sistema o el entorno |
| 7 | `itda-runtime` | Ejecuta acciones: credenciales, aprobaciones, moldeado, registro de auditoría |
| 9 | `itda-setup` | Los pasos de configuración, inicio de sesión y comprobación, para la CLI y el servidor MCP por igual |
| 10 | `itda-mcp` | Servidor MCP sobre stdio |
| 11 | `apps/cli` | El comando `itda` |

## Desarrollo

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check-layers
```

`rust-toolchain.toml` fija Rust 1.98 (edición 2024). Las pruebas solo hablan
con servidores locales que ellas mismas inician. Las reglas de commits y pull
requests están en [CONTRIBUTING.md](CONTRIBUTING.md).

## Seguridad

[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) enumera de qué se defiende itda y
de qué no. Informa de vulnerabilidades como se describe en
[SECURITY.md](SECURITY.md).

## Créditos y licencia

Con licencia [Apache-2.0](LICENSE). Partes de este proyecto derivan de
[Hycli](https://github.com/Hybirdss/Hycli) (Apache-2.0); consulta [NOTICE](NOTICE).
Cada archivo derivado lo indica en su encabezado y describe qué cambió. Este
proyecto no está afiliado a Hycli y no usa su nombre ni sus marcas.
