<p align="center"><img src="assets/hero.png" alt="itda: eine Hand reicht eine kleine Website-Karte an einem Faden an einen KI-Agenten weiter. Einfach eine Adresse hinter itda tippen, und Ihre KI kann die Site benutzen." width="100%"></p>

<p align="center"><strong>Einfach eine Adresse hinter itda tippen, und Ihre KI kann die Site benutzen.</strong></p>

<p align="center"><a href="README.md">English</a> | <a href="README.ko.md">한국어</a> | <a href="README.ja.md">日本語</a> | <a href="README.zh-Hans.md">简体中文</a> | <a href="README.zh-Hant.md">繁體中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <b>Deutsch</b> | <a href="README.pt-BR.md">Português (Brasil)</a> | <a href="README.vi.md">Tiếng Việt</a> | <a href="README.id.md">Bahasa Indonesia</a> | <a href="README.ru.md">Русский</a></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="Lizenz: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/docs/">Doku</a>
</p>

Geben Sie itda eine Adresse. Es lernt, was die Site kann, und probiert es aus.
Es gibt Ihrem KI-Agenten eine kurze Liste von Aktionen in einfachen Worten.
Lesen funktioniert einfach. Auf der Site ändert sich nichts, bis Sie Ja sagen.
Es läuft auf Ihrem eigenen Rechner, und es funktioniert auch im alten Intranet
hinter einer Anmeldung.

Es ist für die Sites Ihrer Firma und die Dienste, bei denen Sie ein Konto haben,
nicht für das ganze Web.

<p align="center"><img src="assets/setup.gif" alt="Ein erster Lauf: itda fragt nach einer Adresse, news.ycombinator.com wird eingefügt, es findet eine Liste mit 30 Datensätzen, ein Formular und einen Feed, probiert beide Lesevorgänge aus, speichert die Site und verbindet Claude Desktop nach einem einzigen Enter" width="860"></p>

<details>
<summary>Derselbe Lauf als Text</summary>

```console
$ itda news.ycombinator.com
1/5  news.ycombinator.com wird angesehen...
    Die Seite öffnet sich ohne Anmeldung.
    Gefunden: eine Liste mit 30 Datensätzen, ein Formular, ein Feed.
2/5  Was Agenten tun können werden:
    lesen   home         List what Hacker News shows  (Eingaben: p)
    lesen   feed         Read the RSS feed
3/5  Jeder Lesevorgang wird einmal ausprobiert...
    ✓ home         30 Datensätze
    ✓ feed         ok
4/5  Gespeichert als 'news-ycombinator'.
    Claude Desktop verbinden? [Y/n]
5/5  itda wird zu Claude Desktop hinzugefügt...
itda zu Claude Desktop hinzugefügt (~/Library/Application Support/Claude/claude_desktop_config.json).
Öffnen Sie Claude Desktop, und itda ist da.

Fertig. Bitten Sie Ihren Agenten um etwas auf 'news-ycombinator', zum Beispiel "nutze news-ycombinator, um List what Hacker News shows".
```

</details>

Bitten Sie Ihren Agenten: *„nutze news-ycombinator, um aufzulisten, was Hacker
News zeigt“*. Eine Site hinter einem Anmeldeformular fragt stattdessen nach
Ihrem Benutzernamen und Passwort. itda füllt das Formular der Site für Sie aus und behält die Sitzung auf Ihrem Rechner. Reicht das nicht
(Single Sign-on, ein Einmalcode), öffnet sich Ihr eigener Browser. Sie melden
sich an und benutzen die Site wie immer, während itda zusieht.

<p align="center"><img src="assets/flow.svg" alt="Zeichnen Sie eine Website einmal in Ihrem Browser auf; itda entwirft daraus Befehle. Der Agent ruft sie auf; eine Änderung wartet auf die Freigabe eines Menschen." width="860"></p>

> Status: früh, und gegen echte Sites im Einsatz. Die Laufzeit, die CLI, der
> MCP-Server, Freigaben (im Terminal oder in der Agenten-App), das Audit-Log,
> Entwürfe aus OpenAPI-Dokumenten, Browser-Aufzeichnungen oder abgerufenen
> Seiten, API-Schlüssel, Browser-Anmeldung und CSRF-Vorab-Schritte funktionieren
> heute. Signierte Site-Pakete kommen als Nächstes. Siehe
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Warum

Agenten, die einen Browser steuern, lesen jede Seite neu und geben für jeden
Screenshot oder Accessibility-Snapshot Tausende Tokens aus. Sie klicken, was
die Seite ihnen sagt, und brechen still, wenn sich ein Layout ändert. itda
verlegt diese Arbeit in einen Installationsschritt:

| | Browser-steuernder Agent | itda |
|---|---|---|
| Was der Agent sieht | Ganze Seiten, bei jedem Schritt | Das typisierte Ergebnis einer Aktion, auf 24 KB begrenzt |
| Eine geänderte oder abgemeldete Seite | Als Daten gelesen | Anhand der erklärten Antwort erkannt (`response_mismatch`, `auth_required`) |
| Änderungen auf der Site | Was immer das Modell entscheidet | Einmalige Freigabe für genaue Eingaben, von einem Menschen im Terminal oder in der Agenten-App erteilt |
| Text auf der Seite, der „ignoriere vorherige Anweisungen“ sagt | Im Prompt | Als nicht vertrauenswürdige Daten markiert, nie ein Werkzeug |
| Zugangsdaten | Vom Modell in die Seite getippt | Im Schlüsselbund des Systems; der Agent sieht nur Konto-IDs |
| Blockiert werden | Erneut versuchen, IP wechseln, Browser imitieren | Den Lesevorgang einmal mehr in Ihrem eigenen installierten Browser versuchen, sich merken, was funktioniert hat, nichts anhalten |
| Was passiert ist | Chat-Verlauf | Hash-verkettetes Audit-Log, das Sie prüfen können |

## Was es kann und was nicht

Jede Zeile hier gilt für den Code von heute. `itda setup` sagt, welchen Fall
es angetroffen hat; `itda draft page` oder `itda record` sagen, was sie
weggelassen haben.

**Funktioniert heute**

- Serverseitig gezeichnete Seiten: Tabellen, Listen und wiederholte Karten
  werden zu Datensätzen, und eine Seite, die eine einzelne Sache ist, wird zu
  einem Lesevorgang ihrer Felder. Suchformulare werden zu Lesevorgängen,
  Eingabeformulare zu Änderungen, und Feeds kommen mit.
- Seiten, die ihre Daten für ihre Skripte einbetten (`__NEXT_DATA__`,
  `ld+json`, ein Apollo- oder vorgeladener Zustand): ein Lesevorgang pro Liste
  darin.
- Von Skripten gezeichnete Seiten, die eine API mit JSON-Antwort aufrufen, etwa
  ein React- oder Vue-Portal: `itda setup` öffnet Ihren Browser auf der Seite
  und sieht zu, was sie von selbst lädt. Es entwirft aus diesen Aufrufen. Es
  klickt und tippt nicht.
- Formulare, die ein CSRF-Token wollen: ein Vorab-Schritt liest direkt vor
  jedem freigegebenen Absenden ein frisches von der Formularseite.
- Seiten, die eine Hülle um Frames sind: der Inhalt sitzt in einem `<iframe>`
  derselben Site, oft hinter einer eigenen Anmeldung. itda liest jeden Frame als
  Seite der Site und meldet sich zuerst an, wenn der Frame es verlangt.
- Cookie-Sitzungen: eine Anmeldung über das Formular der Site mit Ihrem
  Benutzernamen und Passwort, oder Ihr eigener Browser für Single Sign-on und
  Einmalcodes. Eine Sitzung in der Adresse (`?sid=...`) funktioniert genauso.
- Eine OpenAPI- oder Swagger-Beschreibung, die die Site veröffentlicht, und ein
  GraphQL-Endpunkt, der auf Introspektion antwortet: ein Lesevorgang pro
  Operation oder Abfragefeld.
- Sites, die GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki,
  WordPress, Gitea oder Forgejo betreiben: deren eigene API, aus einer fertigen
  Vorlage.
- Dateien herein und heraus: ein Link, den die Seite als Anhang kennzeichnet,
  wird zu einem Lesevorgang, der die Datei auf Ihrem Computer speichert und
  ihren Pfad, ihre Größe und ihren Hash zurückgibt, nie die Bytes. Ein
  Formular mit einem Dateifeld wird zu einer Änderung, die eine von Ihnen
  benannte Datei sendet; Sie geben sie nach Name, Größe und Hash frei.
- Alles, was Sie selbst beim Benutzen aufgezeichnet haben, im Browser oder als
  HAR-Datei.
- Seiten hinter einer Challenge-Seite, wenn der Browser auf Ihrem Computer
  daran vorbeikommt. itda versucht den Lesevorgang einmal mehr in diesem
  Browser, verborgen, nur für diese eine Site und dieses Konto, und merkt sich,
  was funktioniert hat, sodass der nächste Aufruf dort beginnt. Der Browser
  bleibt zwischen CLI-Aufrufen warm und schließt sich nach fünf Minuten ohne
  Nutzung; ein geänderter Suchbegriff startet ihn nicht neu. Alles, was die
  Seite lädt, durchläuft dieselben Adress- und Leseprüfungen wie eine direkte
  Anfrage. Siehe [die Transportrichtlinie](docs/HONEST_CLIENT.md).

**Funktioniert nach dem Aufzeichnen**

- Eine Seite, die bei einem einfachen Abruf nichts zeigt und nichts von selbst
  lädt. Sie holt erst etwas, nachdem Sie klicken, suchen oder sich anmelden.
  `itda record` sieht zu, wie Sie sie benutzen, und entwirft aus den Aufrufen,
  die es gesehen hat.

**Funktioniert nicht**

- WebSocket. itda spricht einfaches HTTP, und eine Aufzeichnung behält keinen
  Socket-Verkehr.
- APIs, die jede Anfrage signieren (eine Signatur oder Nonce, die pro Aufruf
  berechnet wird). itda behält ein festes Token oder einen API-Schlüssel im
  Schlüsselbund. Es erzeugt keinen Wert nach, der sich mit jedem Aufruf ändert,
  also lehnt die Site die Anfrage ab.
- Seiten aus HTML-Fragmenten (htmx und Ähnliches). Nicht mit Absicht: itda kann
  ein Fragment mit einer ganzen Tabelle noch lesen, aber nicht, was das Skript
  damit macht.
- Inhalt, der auf ein Canvas oder in ein Bild gezeichnet wird. itda liest Text
  in HTML, JSON und XML; es sieht sich keine Pixel an.
- Eine Challenge, an der auch Ihr Browser nicht vorbeikommt, oder ein Captcha.
  itda löst keine Captchas. Der Lesevorgang schlägt fehl und sagt das; nichts
  wird pausiert. Siehe [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- Alles, was Ihr Konto nicht sehen kann. itda hat kein eigenes Konto; es
  benutzt Ihres.

## Wenn sich die Site ändert

`itda check <site>` führt jeden Lesevorgang aus, der keine Eingabe braucht, und
sendet nie eine Änderung. Es endet mit einem Urteil: `ok`, `signed_out`,
`site_changed`, `blocked` oder `unreachable`. Der Exit-Code folgt ihm (11 für
`site_changed`), damit ein geplanter Job Sie warnen kann, bevor ein Agent
darauf stößt. Über MCP gibt `itda_check` dasselbe zurück.
`itda fsck <site>` liest die lokale Aufzeichnung dessen, was itda versucht hat
(Formen und Urteile, nie Werte), und listet, was nie versucht wurde, was eine
Eingabe braucht und was ab und zu fehlschlägt, jeweils mit dem auszuführenden
Befehl.

Lautet das Urteil `site_changed`, entwirft `itda refresh <site>` die Site neu
aus dem, womit sie eingerichtet wurde: ihren Seiten, der API-Beschreibung, dem
GraphQL-Endpunkt oder der behaltenen Form der Aufzeichnung, mit jedem Wert
geleert. Es zeigt pro Aktion, was sich ändern würde, und schreibt nichts, bis
Sie Ja sagen. itda behält Aktionen, die Sie von Hand bearbeitet haben, und
aktualisiert Aktionen, die der neue Entwurf geändert hat. Eine Aktion, die die
Quellen nicht mehr entwerfen, lässt es nur fallen, wenn auch ihr Lesevorgang
fehlgeschlagen ist, und jede neue Aktion probiert es einmal aus und nimmt sie
auf, wenn sie antwortet. itda zieht Freigaben zurück, die auf die Site warten,
und prüft die Site direkt danach erneut. Eine mit `itda add` installierte
Definition hat keine Quellen und kann nicht neu entworfen werden; bearbeiten Sie
sie von Hand.

Das ist der Kompromiss. Ein Browser-Agent passt sich bei jedem Lauf an, und ist
dafür langsam und nicht deterministisch. itda ist eine Anfrage und jedes Mal
dieselbe Antwort; nach einer Challenge ist es ein Versuch mehr in Ihrem Browser,
und immer noch dieselbe Antwort. Es bricht laut, wenn sich die Site bewegt.
`docs/TRANSPORT_BENCHMARKS.md` hat die Zahlen, auch die eine, die itda noch
nicht erreicht.

## Neben Browser-Agenten

Playwright MCP, browser-use und Computer Use steuern bei jeder Aufgabe einen
Browser. itda ersetzt sie nicht: eine Seite, die Sie einmal benutzen, oder eine
Site aus der Liste oben, die itda nicht abdeckt, gehört ihnen. itda ist für die
Site, die ein Agent immer wieder aufrufen wird. Einmal im Browser aufzeichnen,
die Aktionen durchsehen, dann über HTTP ausführen, mit einer typisierten
Antwort, einer einmaligen Freigabe für jede Änderung und einem Log. Nutzen Sie
beides: einen Browser-Agenten zum Erkunden, itda für die Aufrufe, die jeden Tag
laufen.

## Schnellstart

```sh
# macOS und Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

Laden Sie ein Archiv von den
[Releases](https://github.com/brnyxx/itda-cli/releases) herunter, oder bauen
Sie aus dem Quellcode mit
`cargo install --git https://github.com/brnyxx/itda-cli itda-cli`.

Claude Desktop, ohne Terminal: Laden Sie das Bundle für
[macOS (Apple silicon)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-aarch64-apple-darwin.mcpb),
[macOS (Intel)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-apple-darwin.mcpb) oder
[Windows](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-pc-windows-msvc.mcpb) herunter und öffnen Sie die
Datei. Claude Desktop installiert itda mit seinen Werkzeugen, und „benutze
the-site-you-use.example“ richtet eine Site aus der Unterhaltung heraus
ein. Eine Änderung freizugeben passiert weiterhin im Terminal mit
`itda approve`, für Änderungen installieren Sie also auch den Befehl.

Tippen Sie `itda` und die Adresse einer Site, die Sie benutzen. Es findet die
Agenten-Apps auf Ihrem Rechner und verbindet sie nach einem einzigen Enter.
Es spricht zwölf Sprachen: `LANG` wählt eine, `ITDA_LANG` geht vor. Ein bloßes `itda` fragt nach der
Adresse; `itda setup <adresse>` nimmt Optionen für Skripte.

itda erkennt Sites, die bekannte Software betreiben. Wenn die Seite sagt, sie
sei ein GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki, WordPress oder
Gitea (auch Forgejo), und eine Anfrage an die eigene API dieser Software das
bestätigt, bekommt der Agent die echte API dieser Software aus einer fertigen
Vorlage ([`sites/software/`](sites/software/)), angemeldet so, wie diese
Software es macht. Was die Seite selbst zeigt, kommt dazu. `itda setup --as jira`
erzwingt eine Vorlage; `--no-template` liest die Seite wie jede andere.

Eine von Skripten gezeichnete Seite zeigt itda nur einen Titel, also öffnet
itda Ihren Browser für ein paar Sekunden darauf. Es sieht zu, was die Seite von
selbst lädt, und entwirft aus diesen Aufrufen. Nichts wird geklickt oder
getippt, und das Fenster schließt sich von selbst. Nur eine Seite, die immer
noch nichts zeigt oder eine Anmeldung braucht, bittet Sie, die Site in Ihrem
Browser zu benutzen, während itda zusieht.

Sobald ein Agent verbunden ist, können Sie ihn auch einfach bitten: „richte
news.ycombinator.com für mich ein“. Er ruft `itda_setup` auf. itda sieht sich
die Site an, entwirft und probiert sie auf dieselbe Weise aus. Ihre Aktionen
erscheinen als Werkzeuge in derselben Unterhaltung. Eine Skript-Seite öffnet
sich kurz in einem Browserfenster auf Ihrem Bildschirm. Wenn ein Lesevorgang
eine Anmeldung verlangt, öffnet `itda_login` Ihren eigenen Browser; der Agent
sieht nur die Konto-ID.

Lassen Sie das Terminal weg und geben Sie das Claude Code, Cursor oder Codex,
mit der Adresse einer Site, die Sie benutzen:

```text
Installiere itda und richte the-site-you-use.example für mich ein.
1. Installieren: curl -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
   (unter Windows: irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex)
2. Ausführen: itda setup https://the-site-you-use.example --client claude-code -y
   (nimm cursor, codex oder claude-desktop, wenn du das bist)
3. Lies https://github.com/brnyxx/itda-cli/blob/main/AGENTS.md und benutze die Site dann über itda.
   Lesen funktioniert einfach. Alles, was die Site ändert, wartet auf meine Freigabe.
   Wenn ein Lesevorgang eine Anmeldung verlangt, sag mir, dass ich das ausführen soll: itda login <site> --browser
```

Die Teile sind eigene Befehle. Für Seiten, die Sie ohne Anmeldung öffnen
können, geben Sie itda ihre Adressen. Tabellen, Listen und wiederholte Karten
werden zu Datensätzen. Eine Seite, die eine einzelne Sache ist (ein Paket, ein
Artikel, ein Produkt), wird zu einem Lesevorgang ihrer Überschrift, ihres
Textes und ihrer beschrifteten Werte. Eine Liste, deren Links eine Form teilen,
bekommt einen Lesevorgang für einen Datensatz nach seiner ID. Suchformulare
werden zu Lesevorgängen; Eingabeformulare zu Änderungen. Feeds kommen mit. Ein
GraphQL-Endpunkt, den die Seite nennt, wird einmal gefragt, was er anbietet,
ein Lesevorgang pro Abfragefeld. Veröffentlicht die Site eine API-Beschreibung,
kommen deren Lesevorgänge zum Entwurf dazu:

```sh
itda draft page https://example.com/board --install   # zeigt, was es auf der Seite gefunden hat
itda call example.get-board --page 2 --json
itda check example                                     # passt die Seite noch?
```

Für eine Site hinter einem Anmeldeformular geben Sie `itda setup` die Seite und
Ihren Benutzernamen. Es sendet das Formular der Site einmal, mit dem Passwort, das Sie eintippen. Es behält die Sitzung im
Schlüsselbund des Systems und entwirft aus der Seite, wie Sie sie angemeldet
sehen. Die Definition merkt sich das Formular, also meldet `itda login board`
sich erneut an, wenn die Sitzung abläuft:

```sh
itda setup https://intranet.example/board --name board --user me   # fragt nach dem Passwort
itda call board.list-notices --json                 # Lesevorgänge laufen sofort
itda call board.create-notice --title "Server maintenance"   # Änderungen warten auf Ihre Freigabe
itda login board --user me                          # wenn die Sitzung abläuft
```

Für Single Sign-on oder einen Einmalcode zeichnen Sie stattdessen auf, wie Sie
die Site benutzen. itda öffnet Ihren eigenen Browser in einem frischen Fenster.
Es sieht zu, was die Seiten vom Server verlangen, während Sie sich anmelden,
suchen und ein Formular absenden; es klickt und tippt nie:

```sh
itda record https://intranet.example/board --name board --install   # drücken Sie Enter, wenn Sie fertig sind
                                                    # dann sagen Sie Ja, um die benutzte Sitzung oder das Token zu behalten
itda mcp install claude-desktop                     # oder lassen Sie Ihren Agenten sie aufrufen
```

`examples/lab` betreibt Gitea, Redmine, WordPress, MediaWiki, ein kleines altes
Intranet und drei hässlichere Fälle hinter ihren Anmeldungen: ein Board in einem
iframe, eine Tabelle, die ein Skript aus einem XHR zeichnet, und eine Sitzung in
der URL. Eine Testsuite geht jeden von der Einrichtung bis zu einer
freigegebenen Änderung durch.

Die Aufzeichnung bleibt im Speicher, und nur die Form jedes Aufrufs erreicht den
Entwurf: kein Header, Cookie oder Wert, den Sie getippt haben. Eine HAR-Datei
aus den Entwicklerwerkzeugen des Browsers funktioniert genauso mit
`itda draft har`.

Öffentliche APIs brauchen keine Aufzeichnung. Diese fertigen Sites aus
[sites/](sites/README.md) sind eingebaut und brauchen kein Konto:

<p><a href="sites/github.yaml"><img src="assets/brands/github.svg" width="36" height="36" alt="GitHub"></a> <a href="sites/npm.yaml"><img src="assets/brands/npm.svg" width="36" height="36" alt="npm"></a> <a href="sites/crates-io.yaml"><img src="assets/brands/crates-io.svg" width="36" height="36" alt="crates.io"></a> <a href="sites/pypi.yaml"><img src="assets/brands/pypi.svg" width="36" height="36" alt="PyPI"></a></p>

```sh
itda add github      # oder npm, crates-io, pypi
itda call github.repo --owner rust-lang --repo rust
```

Probieren Sie die mitgelieferte Sandbox, ein falsches Intranet, das nur auf
127.0.0.1 lauscht: folgen Sie [examples/sandbox](examples/sandbox/README.md).
Eine Site-Definition ist eine YAML-Datei. `itda add spec.yaml` installiert sie,
nachdem es gezeigt hat, wohin sie Anfragen sendet und welche Aktionen die Site
ändern. `itda doctor` gibt den Ort des Datenverzeichnisses aus.
[docs/SPEC.md](docs/SPEC.md) erklärt, wie man eine Definition schreibt.

```sh
itda setup <url>                         # Seite oder Aufzeichnung -> benannte Aktionen -> ausprobiert -> Agent, mit Fragen
itda draft page <url>... [--install]     # eine Site aus den Seiten entwerfen, die Sie benutzen
itda record <url> [--install]            # oder daraus, wie Sie sie im Browser benutzen, hinter einer Anmeldung
itda draft har <file> / openapi <file>   # oder aus einer gespeicherten Aufzeichnung oder einer API-Beschreibung
itda add spec.yaml / remove <site>       # eine Site installieren oder entfernen (auch eine URL oder github)
itda ls                                  # installierte Sites und ihre Aktionen
itda describe <site>                     # ihre Eingaben und ein Befehl zum Kopieren für jede
itda call <site>.<action> --input value  # eine ausführen; --json für Agenten
itda login <site> --user me              # über das Formular der Site anmelden (Passwort über stdin oder eine Frage)
echo "$KEY" | itda login <site> --secret api-key   # oder einen API-Schlüssel im Schlüsselbund des Systems ablegen
itda login <site> --browser              # oder im eigenen Browser anmelden, einmal (versiegelt im Datenverzeichnis)
itda accounts / logout <site>            # gespeicherte Anmeldungen ansehen oder entfernen
itda approvals / approve <id> / reject <id>
itda audit [--verify]                    # was Agenten getan haben
itda check [<site>|--all]                # passt jeder Lesevorgang noch zur Site?
itda refresh <site>                      # eine Site hat sich geändert: neu entwerfen, Ihre Entscheidungen behalten
itda resume <site>                       # nach einer Pause im direkten HTTP-Modus oder einem abgelaufenen Schlüssel
itda feedback                            # ein Problem melden: öffnet ein neues Issue, ausgefüllt
```

## Aus einem Agenten benutzen

```sh
itda mcp install claude-desktop   # oder cursor, vscode, codex
itda mcp install claude-code      # gibt den Befehl `claude mcp add` aus
# mit --approve-in-client Änderungen im eigenen Formular des Clients freigeben
# itda mcp --site <name> serviert nur eine Site, für die eigene Konfiguration eines Projekts
```

Der Installer fügt sich in die Konfiguration des Clients ein und behält Ihre
anderen Server. Er sichert die alte Datei als `.bak`. `--dry-run` zeigt die
Änderung zuerst. `itda mcp` serviert jede installierte Site:

- Bis zu 30 Aktionen erscheinen als je ein Werkzeug (`sandbox.list-notices`),
  mit Eingabeschemas und Hinweisen zu nur lesen, destruktiv und idempotent.
- Darüber halten drei feste Werkzeuge (`itda_search`, `itda_describe`,
  `itda_call`) die Werkzeugliste kurz und die Prompt-Caches gültig. `--mode`
  überschreibt das.
- Ergebnisse sind derselbe JSON-Umschlag wie bei `itda call --json`, als nicht
  vertrauenswürdig markiert.
- Eine Änderung gibt `approval_required` mit einer ID zurück. Kein Werkzeug
  gibt frei; der Agent muss Sie fragen.

Agenten, die nur eine Shell haben, können `itda call … --json` aufrufen und
nach dem Exit-Code verzweigen:

| Exit | Bedeutung |
|---|---|
| 0 | ok |
| 2 | falsche Eingabe oder unbekannte Aktion |
| 10 | erneut anmelden (`itda login`) |
| 11 | die Seite passt nicht mehr zu ihrer Definition |
| 12 | die Site hat diesen Lesevorgang abgelehnt, auch im Browser; `paused: false` heißt, es gibt nichts fortzusetzen |
| 13 | Rate begrenzt |
| 14 | die Site hat diese eine Anfrage als nicht erlaubt abgelehnt; erneut anmelden hilft nicht |
| 20 | braucht Freigabe, oder der Benutzer hat noch nicht geantwortet (oder Änderungen sind aus) |
| 21 | Freigabe benutzt, abgelehnt, abgelaufen oder zurückgezogen, weil sich die Eingaben geändert haben |
| 22 | itda hat eine Änderung gesendet, aber ihr Ergebnis ist unbekannt; mit `outcome_check` sagt der Lesevorgang, den die Änderung nennt, ob sie angekommen ist |
| 30 | Netzwerkfehler |

## Grundsätze

- **Lokal zuerst.** Definitionen laufen auf Ihrem Gerät. Zugangsdaten verlassen
  es nie.
- **Standardmäßig nur lesen.** Eine Aktion ist nur dann ein Lesevorgang, wenn
  Methode, Name, Pfad, Eingaben und GraphQL-Dokument alle übereinstimmen. Alles
  andere ist eine Änderung.
- **Menschen geben Änderungen frei.** Jede Freigabe gilt für eine Anfrage mit
  genauen Eingaben, Konto, Definition und Zugangsdaten. Sie funktioniert einmal
  und läuft ab.
- **Eine Challenge bekommt einen Versuch mehr, in Ihrem Browser.** Ein
  Lesevorgang, den die Site mit einer Challenge-Seite beantwortet, läuft einmal
  mehr im Browser, der auf Ihrem Computer installiert ist; itda merkt sich, was
  funktioniert hat. Eine Änderung wird einmal gesendet und nie über den
  Browser. `ITDA_TRANSPORT=http` bleibt bei direktem HTTP. Siehe
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- **Geprüfte Aktionen.** Eine Definition erklärt, wie eine richtige Antwort
  aussieht, sodass „abgemeldet“ und „die Site hat sich geändert“ Fehler sind,
  keine Daten.

## Aufbau

Crates hängen nur von Crates unter ihnen ab; `cargo xtask check-layers` setzt
das durch.

| Schicht | Crate | Zweck |
|---|---|---|
| 0 | `itda-types` | Fehlervertrag, URL-Prüfungen (Public Suffix List), Schwärzung |
| 1 | `itda-html`, `itda-store` | Skriptfreie HTML-Extraktion; der lokale SQLite-Speicher |
| 2 | `itda-spec` | Definitionsformat, Aufbau der Anfragen, Prüfung der Antworten |
| 3 | `itda-policy` | Lese-/Schreib-Einstufung, Sicherheitsprüfungen für Definitionen, Freigaben |
| 4 | `itda-browser` | Anmeldung, Aufzeichnung, isolierter Browser-Lesetransport |
| 5 | `itda-net` | Adaptiver Transport, Strategie-Cache, Taktung, Schutz vor privaten Adressen |
| 6 | `itda-vault` | Konten und Geheimnisse im Schlüsselbund des Systems oder in der Umgebung |
| 7 | `itda-runtime` | Führt Aktionen aus: Zugangsdaten, Freigaben, Formung, Audit-Log |
| 9 | `itda-setup` | Die Schritte für Einrichtung, Anmeldung und Prüfung, für CLI und MCP-Server gleichermaßen |
| 10 | `itda-mcp` | MCP-Server über stdio |
| 11 | `apps/cli` | Der Befehl `itda` |

## Entwickeln

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check-layers
```

`rust-toolchain.toml` legt Rust 1.98 fest (Edition 2024). Tests sprechen nur
mit lokalen Servern, die sie selbst starten. Die Regeln für Commits und Pull
Requests stehen in [CONTRIBUTING.md](CONTRIBUTING.md).

## Sicherheit

[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) listet auf, wogegen itda sich
verteidigt und wogegen nicht. Melden Sie Schwachstellen so, wie
[SECURITY.md](SECURITY.md) es beschreibt.

## Danksagung und Lizenz

Lizenziert unter [Apache-2.0](LICENSE). Teile dieses Projekts sind von
[Hycli](https://github.com/Hybirdss/Hycli) (Apache-2.0) abgeleitet; siehe
[NOTICE](NOTICE). Jede abgeleitete Datei sagt das in ihrem Kopf und beschreibt,
was sich geändert hat. Dieses Projekt steht in keiner Verbindung zu Hycli und
verwendet weder dessen Namen noch dessen Marken.
