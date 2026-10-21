# rag-mobil 🏰

[English](README.md) · [Français](README.fr.md) · Deutsch

[![CI](https://github.com/krisanselmo/rag-mobil/actions/workflows/ci.yml/badge.svg)](https://github.com/krisanselmo/rag-mobil/actions/workflows/ci.yml)

Ein Playmobil-Geschichtengenerator auf Basis von **RAG** (Retrieval-Augmented Generation): Aus
einer oder mehreren Setnummern holt `rag-mobil` das genaue Inventar der Packungen von
[PlaymoDB](https://playmodb.org) (oder aus [einer anderen Datenbank](#inventarquellen)) und
lässt dann ein LLM (standardmäßig Claude, oder OpenAI, Gemini, Groq, ein lokales Modell über
Ollama…) eine Geschichte schreiben, die ein Kind **mit dem nachspielen kann, was wirklich in
seiner Packung steckt**: dieselben Figuren, dieselben Gegenstände, dieselben Mengen.

```
set number ──► Retrieve ──► Augment ──► Writing ───────────────────► story
               PlaymoDB     context     Plan → Draft → Edit
               or another   + prompt    (Re3/DOC), through an LLM (genai)
               source
```

## Beispiel

[**Pepper and the Pirate's Hat**](examples/3799-pirates-secret-island/): eine englische
Geschichte zum Set 3799 „Pirates Secret Island“, geschrieben von Claude, mit dem
[illustrierten Heft (PDF)](examples/3799-pirates-secret-island/booklet.pdf). Jede Figur und
jeder Gegenstand der Geschichte ist in der Schachtel.

## Installation

```bash
cargo build --release
```

Danach, je nach Anbieter:

| Modell (`-m`) | Anbieter | Voraussetzungen |
|---|---|---|
| `claude-opus-5-5` (Standard), `claude-…` | Anthropic | `ANTHROPIC_API_KEY` |
| `gpt-…` | OpenAI | `OPENAI_API_KEY` |
| `gemini-…` | Google Gemini | `GEMINI_API_KEY` |
| `ollama::<modell>` (z. B. `ollama::llama3.2`) | [Ollama](https://ollama.com), lokal und kostenlos | `ollama pull <modell>`, kein Schlüssel |
| `openai::<modell>` + `RAG_MOBIL_OPENAI_BASE_URL` | ein OpenAI-kompatibler Server, lokal oder entfernt (llama.cpp, vLLM, LM Studio…) | die Adresse des Servers; kein Schlüssel, wenn `OPENAI_API_KEY` nicht gesetzt ist |

Weitere Anbieter werden unterstützt (Groq, DeepSeek, xAI, Cohere, OpenRouter, Bedrock…):
siehe die [Liste von genai](https://github.com/jeremychone/rust-genai).

## Verwendung

```bash
# A story in English for a 7-year-old (the default)
rag-mobil 3666

# Several boxes: one story mixing their contents
rag-mobil 3799 70145

# Customized
rag-mobil 5135 --age 5 --words 400 --theme "a storm and a lost treasure"
rag-mobil 3666 --language Deutsch --effort high

# Written in a single call, streamed as it is written (faster)
rag-mobil 3666 --method direct

# Illustrated booklet, with how to set up each scene: HTML, or PDF to print
rag-mobil 3799 --scene-setup --format html > booklet.html
rag-mobil 3799 --scene-setup --format pdf > booklet.pdf
rag-mobil 3799 --scene-setup --format pdf --layout compact > booklet.pdf   # 2 pages instead of 7

# Choose-your-own-adventure: numbered paragraphs, choices, several endings
rag-mobil 3799 --interactive --format pdf > adventure.pdf

# A saga: each story follows on from the previous one, with the same characters
rag-mobil 3799 --saga pirates.json        # episode 1
rag-mobil 3799 70145 --saga pirates.json  # episode 2, with a new box

# The parrot is the favorite piece: it is at the heart of the story
rag-mobil 3799 --star "30 06 368"

# The real box: the parrot is lost, and so are 2 of the 4 halberds
rag-mobil 3666 --missing "30 06 368" --missing "30 07 640:2"

# Other providers
rag-mobil 3666 -m gpt-5.4-mini
rag-mobil 3666 -m ollama::llama3.2        # local, no API key
RAG_MOBIL_OPENAI_BASE_URL=http://localhost:8080/v1 rag-mobil 3666 -m openai::ministral-3-3b  # llama.cpp, vLLM…

# No LLM call (no API key needed)
rag-mobil 3666 --inventory-json   # the fetched inventory, as JSON
rag-mobil 3666 --dry-run          # the first prompt sent to the LLM
```

Geschichten auf Deutsch bekommst du mit `--language Deutsch`. Auch die Überschriften, die der
Code selbst schreibt, sind dann auf Deutsch („In deiner Schachtel“, „Aufbauen“, das Heft…); die
Beispiele unten zeigen die englische Ausgabe („In your box“, „Set the scene“…).

Die Geschichte wird auf `stdout` ausgegeben (als Markdown); Schritte und Fortschrittsmeldungen
gehen nach `stderr` (`rag-mobil 3666 > story.md` funktioniert).

| Option | Standard | Zweck |
|---|---|---|
| `-l, --language` / `RAG_MOBIL_LANGUAGE` | `English` | Sprache der Geschichte (z. B. `Deutsch`, `français`) |
| `-a, --age` / `RAG_MOBIL_AGE` | `7` | Alter des Kindes |
| `-w, --words` / `RAG_MOBIL_WORDS` | `600` | Ziellänge |
| `-t, --theme` | — | Freier Wunsch |
| `--citations` / `RAG_MOBIL_CITATIONS` | aus | Jedes genannte Teil mit seiner Referenz zitieren (siehe unten) |
| `--method` | `planned` | `planned` (Plan, Entwurf, Überarbeitung), `direct` (ein Aufruf) oder `interactive` |
| `--interactive` | aus | Abenteuer zum Selbstwählen (Kurzform für `--method interactive`, siehe unten) |
| `--saga` / `RAG_MOBIL_SAGA` | — | JSON-Datei mit Folgen: Die Geschichte knüpft daran an und wird dann hinzugefügt (siehe unten) |
| `--scene-setup` / `RAG_MOBIL_SCENE_SETUP` | aus | Methode `planned`: „Set the scene“ vor jedem Teil (siehe unten) |
| `-f, --format` / `RAG_MOBIL_FORMAT` | `markdown` | `markdown`, `html` oder `pdf` (illustriertes Heft, siehe unten) |
| `--booklet FILE` (Alias `--livret`) | — | Schreibt zusätzlich das Heft (HTML, oder PDF bei `.pdf`): Die Geschichte bleibt als Markdown auf `stdout`, mit einem einzigen LLM-Durchlauf |
| `--layout` / `RAG_MOBIL_LAYOUT` | `booklet` | `--format html` oder `pdf`: `booklet` oder `compact` (spart Papier) |
| `--browser-arg` | — | `--format pdf`: Argument für den Browser (z. B. `--no-sandbox`); wiederholbar |
| `--star REF` | — | Das Lieblingsteil des Kindes, im Mittelpunkt der Geschichte; wiederholbar (siehe unten) |
| `--missing REF[:N]` | — | Ein Teil, das in der Packung fehlt: alle Exemplare oder N; wiederholbar |
| `--missing-file` / `RAG_MOBIL_MISSING_FILE` | — | Fehlende Teile, eines pro Zeile (siehe unten) |
| `--reviews N` / `RAG_MOBIL_REVIEWS` | `1` | Methoden `planned` und `interactive`: Runden der Treue-Prüfung (0 bis 3), jede gefolgt von einer Neufassung; sie enden bei der ersten Prüfung ohne Befund |
| `--no-review` | aus | Wie `--reviews 0`: die Treue-Prüfung überspringen |
| `--include-all` | aus | Auch Papieranleitungen im Kontext behalten |
| `-m, --model` / `RAG_MOBIL_MODEL` | `claude-opus-5-5` | Modell; der Anbieter ergibt sich aus dem Namen |
| `--effort` | `medium` bei Claude, sonst Standard des Anbieters | `low`, `medium`, `high`, `xhigh`, `max` |
| `--fallback-model` | `claude-opus-4-8` | Modell für einen erneuten Versuch, wenn Claude die Anfrage ablehnt |
| `--no-fallback` | aus | Schaltet diesen erneuten Versuch ab |
| `--playmodb-url` / `PLAYMODB_BASE_URL` | `https://playmodb.org/cgi-bin` | PlaymoDB-Spiegel |
| `--cache-dir` / `RAG_MOBIL_CACHE_DIR` | `~/.cache/rag-mobil/playmodb` | Cache-Verzeichnis der Inventare |
| `--refresh` | aus | PlaymoDB abfragen, auch wenn der Cache noch frisch ist |
| `--no-cache` | aus | Den Cache nicht verwenden |
| `--inventory-file` | — | Inventare aus einer JSON-Datei lesen (Ausgabe von `--inventory-json`) |
| `--inventory-url` / `RAG_MOBIL_INVENTORY_URL` | — | HTTP-Inventarquelle (`{set}` = Setnummer), PlaymoDB als Ausweichquelle |
| `--no-playmodb` | aus | Mit `--inventory-url`: kein Ausweichen auf PlaymoDB |
| `--replay` | — | Das LLM umgehen: Prompts und Antworten in Dateien (siehe Entwicklung) |

Umgebungsvariablen legen die Standardwerte einer Installation fest; eine Option auf der
Kommandozeile hat Vorrang.

### Teile zitieren

Mit `--citations` folgt auf jede genannte Figur und jedes genannte Teil seine Inventarreferenz,
sowohl in der Geschichte als auch in „In your box“: `[ref. 30 06 368]`, oder
`[ref. 30 06 198, 30 06 199]` für einen Gegenstand aus mehreren Teilen. Das Format ist stabil:
Ein Frontend kann die Zitate entfernen (`\s*\[ref\.[^\]]*\]`) oder jede Referenz in einen Link
zum Teil verwandeln. Mit der Methode `planned` schreibt der Code den Abschnitt „In your box“,
dessen Zitate sind also immer exakt; im Text der Geschichte übernimmt das LLM sie, und **der
Code prüft sie** (Methoden `planned` und `interactive`): Eine Referenz, die keine Packung
enthält, kommt zu den Befunden der Prüfung, damit die Neufassung sie korrigiert; ist sie nach
der letzten Neufassung (oder mit `--no-review`) noch da, wird sie aus dem Text entfernt
(Meldung auf `stderr`). Die Geschichte zitiert nie ein Teil, das nicht in der Packung ist.

### Abenteuer zum Selbstwählen

Mit `--interactive` ist der Plan keine Abfolge von Schritten mehr, sondern ein Graph: 6 bis 9
Abschnitte, die jeweils mit 2 oder 3 Wahlmöglichkeiten enden, die zu einem anderen Abschnitt
führen, und 2 bis 4 Enden. Wie beim linearen Plan ist jede Figur und jeder Gegenstand eine
Referenz, die mit der Packung abgeglichen wird; außerdem prüft der Code den Graphen: Jede Wahl
führt zu einem existierenden Abschnitt, kein Abschnitt ist unerreichbar, es gibt keine
Schleifen und mindestens 2 Enden. Stimmt etwas nicht, wird der Plan noch einmal mit der
Fehlerliste angefordert; ist der Graph dann immer noch fehlerhaft, wird die Geschichte in einem
einzigen Aufruf geschrieben (nicht interaktiv).

Die Abschnitte werden in Lesereihenfolge nummeriert (Breitensuche ab dem Anfang), und jeder wird
mit den Abschnitten, die zu ihm führen, als Kontext geschrieben. Die Prüfung sieht alle
Abschnitte; nur die, die sie zitiert, werden neu geschrieben.

```markdown
## 1

Coco the parrot landed on Tom's hammock…

**What do you choose?**

- Pip suggests rowing around the island → **go to 2**
- Robinson wants to search the cave first → **go to 3**
```

In HTML und PDF sind die Wahlmöglichkeiten Links zu den Abschnitten, und ein gedruckter
Abschnitt wird nie auf zwei Seiten verteilt. `--scene-setup` fügt den Spielaufbau jedes
Abschnitts hinzu.

### Sagas

`--saga pirates.json` verknüpft Geschichten miteinander: Die Datei (wird angelegt, falls es sie
nicht gibt) speichert jede erzählte Folge — Packungen, Titel, Prämisse, Figuren (Name, Figur,
Porträt) und Schritte —, und der Plan der nächsten Folge bekommt sie in `<previous_episodes>`,
mit der Anweisung, die Handlung nicht zu wiederholen. Bekannte Figuren, deren Figur in der
Packung ist, werden mit Namen und Persönlichkeit angeboten; die anderen dürfen nur erwähnt
werden.

Der Code prüft die Kontinuität Figur für Figur: Eine bekannte Figur behält ihren Namen („figure
30 13 020 was called "Pip": keep that name“), und ein bereits vergebener Name wandert nicht zu
einer anderen Figur. Der Plan wird noch einmal angefordert; bleiben Abweichungen, werden die
ursprünglichen Namen wiederhergestellt. Die Packungen können sich von Folge zu Folge ändern: Nur
die vorhandenen Figuren kehren zurück. Die Folge wird zur Datei hinzugefügt, sobald die
Geschichte geschrieben ist, außer wenn der Plan gescheitert ist (Schreiben in einem Aufruf).
`--saga` setzt die Methode `planned` oder `interactive` voraus; eine interaktive Geschichte wird
ohne ihre Schritte gespeichert, da ihr Verlauf vom Leser abhängt.

### Lieblingsteil

`--star REF` stellt das Lieblingsteil des Kindes in den Mittelpunkt der Geschichte: Eine Figur
wird zur Hauptfigur, ein Gegenstand spielt eine zentrale Rolle. Die Referenz kann mit oder ohne
Leerzeichen geschrieben werden, wie bei `--missing`; sie muss in der Packung sein (nachdem
fehlende Teile entfernt wurden), sonst bricht das Programm vor jedem LLM-Aufruf ab. Die Option
ist wiederholbar.

Mit den Methoden `planned` und `interactive` prüft der Code den Plan: Eine Lieblingsfigur muss
eine Figur der Geschichte spielen, ab dem ersten Schritt und in mindestens der Hälfte der
Schritte dabei sein; ein Lieblingsgegenstand muss in mindestens 2 Schritten vorkommen. Stimmt
etwas nicht, wird der Plan noch einmal mit diesen Fehlern angefordert; bleiben welche übrig,
fügt der Code das Teil in die ersten Schritte ein, in denen es fehlt. Eine Figur, die keine
Rolle spielt, kann mangels Namen nicht hinzugefügt werden: Die Geschichte wird dann ohne sie im
Rampenlicht geschrieben. Mit `--method direct` bekommt das LLM nur die Anweisung.

```bash
rag-mobil 3666 --star "30 00 496"                        # the trumpeter as the hero
rag-mobil 3799 --star 3006368 --star "30 13 020" --interactive
```

### Verlorene oder kaputte Teile

Das Inventar beschreibt die Packung im Neuzustand; der echten Packung fehlen oft ein paar Teile.
`--missing` entfernt sie vor allem anderen aus dem Inventar: Der Plan kann sie nicht mehr
auswählen, die Prüfung meldet sie, und „In your box“ führt sie nicht auf.

```bash
rag-mobil 3666 --missing "30 06 368"          # every copy
rag-mobil 3666 --missing "30 07 640:2"        # 2 of the 4 halberds lost
rag-mobil 3666 --missing-file my-box.txt      # the list, kept in a file
```

```text
# my-box.txt: one piece per line, in the --missing format
30 06 368        # the parrot
30 07 640:2
```

Die Referenz kann mit oder ohne Leerzeichen geschrieben werden (`3007640`, `ref. 30 07 640`).
Bei mehreren Packungen werden die Exemplare zuerst aus der ersten Packung entfernt, die sie
enthält, dann aus den folgenden. Ein Teil mit unbekannter Menge (`?`) wird nur ohne `:N`
entfernt. Jedes Teil wird auf `stderr` gemeldet (entfernt, wie viele übrig sind, oder nicht in
den Packungen und ignoriert). `--inventory-json` zeigt das Inventar nach dem Entfernen.

### Spielaufbau

Mit `--scene-setup` (Methode `planned`) beginnt jeder Teil der Geschichte mit dem, was du zum
Nachspielen aufbauen musst. Es stammt aus dem geprüften Plan und wird **vom Code** geschrieben,
es wird also nie ein Teil erfunden:

```markdown
> 🧱 **Set the scene**: Pip (Pirate, red vest and ponytail); Rowboat, one-seater (dark brown) + Rowboat gear: seat, oarlocks, paddles (light brown), Lantern (silver/yellow)
```

Damit jeder Spielaufbau vor dem richtigen Teil steht, bekommt die Überarbeitung nach der Prüfung
die Teile durch `* * *`-Zeilen getrennt und muss sie beibehalten. Fügt sie Teile zusammen, wird
die überarbeitete Geschichte ohne Spielaufbau behalten (mit einer Meldung auf `stderr`).

### HTML- und PDF-Heft

`--format html` schreibt statt Markdown eine eigenständige HTML-Seite (Stile eingebettet, helles
oder dunkles Design):

- ein Titelblatt: Titel, Porträts der Figuren, Prämisse, verwendete Packungen;
- ein Vorschaubild jedes Teils (das Feld `image` des Inventars, Bilder von PlaymoDB);
- der Spielaufbau jeder Szene als illustrierte Karten, mit `--scene-setup`;
- „In your box“ als Abhak-Karten: Figuren, Gegenstände, Mengen, Referenz, Packung;
- mit `--citations` wird jedes `[ref. …]` im Text zum Vorschaubild des Teils.

HTML, das das LLM in den Text schreiben könnte, wird als Text angezeigt, und nur
`http(s)`-Bilder werden übernommen. Die Bilder werden beim Öffnen der Seite von PlaymoDB
geladen. Mit `--format html` oder `pdf` erscheint die Geschichte am Ende, auch mit
`--method direct`.

Gedruckt (oder im Browser als PDF gespeichert) wird die Seite zu einem A4-Heft: ein gerahmtes
Titelblatt auf der ersten Seite, eine Seite pro Szene mit kompaktem Spielaufbau, 14-pt-Text im
Blocksatz mit Silbentrennung, „In your box“ auf einer neuen Seite mit einem Kästchen pro Teil,
um die Teile vor dem Spielen zusammenzusuchen, und Seitenzahlen (außer auf dem Titelblatt).

`--layout compact` behält denselben Inhalt, spart aber Papier: ein kurzer Kopf statt des
Titelblatts, die Geschichte in zwei Spalten ohne Seitenumbrüche, und Spielaufbau und „In your
box“ mit kleinen Vorschaubildern direkt nach der Geschichte. Für Set 3799 mit `--scene-setup`
sind das 2 statt 7 Seiten (3 statt 8 in der interaktiven Variante).

`--booklet booklet.pdf` (oder `.html`) schreibt das Heft zusätzlich zur mit `--format`
gewählten Ausgabe in eine Datei: Ein Dienst kann so aus einem einzigen LLM-Durchlauf sowohl die
Markdown-Geschichte als auch ihr Heft behalten. Ein Heft, das nicht geschrieben werden kann
(kein Browser), wird auf `stderr` gemeldet, ohne dass die Geschichte verloren geht.

`--format pdf` druckt selbst: Das Heft wird von einem bereits installierten Headless-Chrome,
-Chromium oder -Edge gedruckt (gesucht in `PATH` und an den üblichen Orten, oder angegeben über
`RAG_MOBIL_BROWSER`). Das PDF wird auf `stdout` geschrieben und in eine Datei umgeleitet. Der
Browser wird vor jedem LLM-Aufruf gesucht. In einem Container, der als root läuft, braucht
Chromium `--browser-arg --no-sandbox`.

## Architektur

```
src/
├── main.rs              CLI (clap)
├── lib.rs
├── error.rs             RagError (thiserror), one variant per step
├── model.rs             SetNumber, Part, SetInventory, missing pieces
├── pipeline.rs          RagPipeline<S: InventorySource, G: StoryGenerator>
├── saga.rs              --saga: previous episodes, character continuity
├── html.rs              --format html: illustrated booklet (pulldown-cmark)
├── pdf.rs               --format pdf: the booklet printed by headless Chrome
├── retriever/           R — retrieval
│   ├── mod.rs           InventorySource trait
│   ├── playmodb.rs      HTTP client (2 pages in parallel)
│   ├── parser.rs        pure HTML parsing, tested on fixtures
│   ├── cache.rs         disk cache, one JSON file per set
│   ├── json_file.rs     --inventory-file
│   ├── http_json.rs     HTTP source in format v1 (--inventory-url)
│   └── fallback.rs      one source, then another for unknown sets
├── augment/             A — augmentation
│   ├── context.rs       inventory → structured context (<set>, <characters>, <pieces>)
│   └── prompt.rs        system prompt (faithfulness rules) + user message
├── generator/           G — generation
│   ├── mod.rs           StoryGenerator trait (streamed text, structured JSON)
│   └── llm.rs           multi-provider generator (genai) + fallback on refusal
└── writer/              writing
    ├── mod.rs           direct / planned methods, Observer (progress)
    ├── plan.rs          plan, check against the inventory, scene setup, "In your box"
    ├── citations.rs     --citations: im Text zitierte Referenzen, gegen die Packungen geprüft
    ├── interactive.rs   choose-your-own-adventure: graph of paragraphs, checked
    ├── prompts.rs       plan / draft / review / revision prompts
    └── planned.rs       Plan → Draft → Edit (adapted from Re3/DOC)
```

### Inventarquellen

Standardmäßig kommen die Inventare von PlaymoDB (siehe unten). Jede andere Datenbank kann sie
liefern, im [Inventarformat v1](docs/inventory-format.de.md) (JSON-Schema:
[`schema/inventory-v1.schema.json`](schema/inventory-v1.schema.json)), demselben Format wie
`--inventory-json` und der Cache:

```bash
# A service answering GET /api/inventories/<set>; PlaymoDB for the sets it doesn't know
rag-mobil 3666 70145 --inventory-url 'http://localhost:8096/api/inventories/{set}'

# That service only
rag-mobil 3666 --inventory-url 'http://localhost:8096/api/inventories/{set}' --no-playmodb

# A file
rag-mobil 3666 --inventory-file my-boxes.json
```

| Quelle | Wann | Ausweichquelle |
|---|---|---|
| `--inventory-file` | eine Datei, ein Set oder ein Array | keine |
| `--inventory-url` / `RAG_MOBIL_INVENTORY_URL` | HTTP-Dienst, `{set}` wird durch die Nummer ersetzt | PlaymoDB (mit Cache), wenn der Dienst `404` antwortet, außer mit `--no-playmodb` |
| (nichts) | PlaymoDB, mit Cache außer bei `--no-cache` | keine |

Mit `--inventory-url` übergibt nur eine `404`-Antwort an PlaymoDB; jeder andere Fehler (Dienst
nicht erreichbar, ungültiges JSON, ein anderes Set als das angefragte, leeres Inventar) beendet
das Programm. `--cache-dir`, `--refresh` und `--no-cache` gelten für das Ausweichen auf
PlaymoDB. Ein auf der Kommandozeile angegebenes `--inventory-file` hat Vorrang vor
`RAG_MOBIL_INVENTORY_URL`. Eine `[rag-mobil]`-Zeile auf `stderr` sagt, woher jedes Set kam
(z. B. `3666 (localhost:8096), 70145 (cache)`).

In einem Inventar kennzeichnet die Kategorie `Klicky` (PlaymoDB) oder `figure` eine Figur, also
eine mögliche Rolle in der Geschichte; die anderen Teile werden nach Kategorie gruppiert.

### Retrieve — PlaymoDB

Zwei Seiten werden parallel geholt und dann zusammengeführt:

- `showinv.pl?setnum=N&pics=on`: Name, Thema, Erscheinungsdatum, **Kategorie** und **Bild**
  jedes Teils (`Klicky` = Figuren, `Animal`, `Furniture`, `Vehicle`…). Beide Layouts von
  PlaymoDB werden unterstützt (Linkliste, `lineup`-Blöcke mit Bildern). Das Bild (das Feld
  `image`, eine absolute URL) wird im Prompt nicht verwendet: Es ist für alle gedacht, die das
  Inventar anzeigen.
- `listinv.pl?setnum=N`: eine Texttabelle mit den **Mengen** und Unterteilen (Perücke, Hut…,
  die zu einer Figur gehören).

Teile aus dem Abschnitt „Extra parts not included in inventory“ (alternative Nummern oder
Ersatzteile) werden verworfen, damit das Modell denselben Gegenstand nicht doppelt zählt. Bei
Sets, die PlaymoDB noch nicht gezählt hat, ist die Menge unbekannt (`?`). Ein Teil ohne Anzahl, das denselben
Namen trägt wie ein gezähltes, ist eine alternative Nummer und wird ebenfalls verworfen; jedes
andere Teil ohne Anzahl bleibt erhalten, mit unbekannter Menge (Set 3693 zählt nur 7 seiner
17 Teile).

Der Scraper gibt sich mit einem eindeutigen User-Agent zu erkennen und stellt nur zwei Anfragen
pro Set; bei einem vorübergehenden Fehler (PlaymoDB antwortet oft mit 508, dem Ressourcenlimit
seines Hosters) versucht er es bis zu 3-mal erneut, nach 1, 3 und dann 6 Sekunden. PlaymoDB wird
von ehrenamtlichen Fans betrieben: Bitte nutze es nicht für massenhaftes Scraping.

**Cache**: Jedes geholte Inventar wird auf der Festplatte gespeichert
(`~/.cache/rag-mobil/playmodb/<set>.json` unter Linux, `~/Library/Caches` unter macOS,
`%LOCALAPPDATA%` unter Windows) und 30 Tage lang wiederverwendet, ohne PlaymoDB erneut
abzufragen. `--refresh` erzwingt einen neuen Abruf, `--no-cache` schaltet den Cache ab. Ein
unlesbarer Eintrag wird einfach neu geholt.

**Mehrere Sets**: `rag-mobil 3799 70145` holt die Inventare nacheinander (Duplikate werden
ignoriert), ohne Begrenzung ihrer Anzahl. Der Kontext enthält einen `<set>`-Block pro Packung,
und der Prompt sagt, dass alles in derselben Geschichte zusammenkommt. Figuren können aus jeder
Packung stammen, und die Exemplare einer Referenz, die in mehreren Packungen vorkommt, werden
addiert. „In your box“ gibt für jede Zeile an, in welcher Packung du suchen musst
(`(box 70145)`). Mit vielen großen Packungen wächst der Prompt: kein Problem für Claude, aber ein
kleines lokales Modell kann daran ersticken.

### Augment — Kontext und Prompt

Das Inventar wird zu einem Pseudo-XML-Block: zuerst die Figuren (sie sind die Rollen der
Geschichte), dann die Teile nach Kategorie, mit Mengen und Farben. Der System-Prompt legt die
Treue-Regeln fest: Figuren nur aus der Packung, Gegenstände nur aus dem Inventar, Mengen werden
eingehalten, technische Teile dienen nur zur Beschreibung der Kulisse. Die Geschichte endet mit
einem Abschnitt „In your box“, der die verwendeten Elemente aufführt.

### Generate — LLM über genai

Die Generierung stützt sich auf die Open-Source-Bibliothek
[`genai`](https://github.com/jeremychone/rust-genai) (MIT/Apache-2.0), die einen einheitlichen
Streaming-Client für rund zwanzig Anbieter bietet. `generator/llm.rs` ergänzt, was für dieses
Projekt spezifisch ist:

- **Claude**: `max_tokens` auf 64.000 und standardmäßig `medium` effort, was genai in
  `thinking: {type: "adaptive"}` + `output_config.effort` übersetzt;
- **Ablehnungen**: Lehnt das Modell die Anfrage ab (`stop_reason: "refusal"` bei Claude, ein
  Inhaltsfilter anderswo), wird der Teiltext verworfen; bei Claude folgt ein einziger erneuter
  Versuch mit `--fallback-model`;
- **Abschneiden**: Ein Abbruch wegen `max_tokens` wird gemeldet;
- **Prompt-Cache**: Die Entwurfsaufrufe (einer pro Teil, einer pro Absatz im interaktiven
  Modus) beginnen alle mit demselben System-Prompt, Inventar und Plan. Bei Claude wird dieser
  Anfang mit `cache_control` (5 Minuten) markiert: Der erste Aufruf schreibt ihn, die folgenden
  lesen ihn zu einem Bruchteil des Eingabepreises und schneller wieder. Die anderen Anbieter
  erhalten den Prompt unverändert (OpenAI und Gemini cachen wiederholte Anfänge von selbst);
- **Tokens**: Die Zusammenfassung auf `stderr` summiert über alle Aufrufe die Eingabe-Tokens
  (davon die aus dem Cache gelesenen) und die erzeugten Tokens.

Die anderen Anbieter behalten ihre Standardeinstellungen (viele lokale Modelle lehnen
Reasoning-Parameter ab); `--effort` erzwingt sie.

### Schreiben — die Re3/DOC-Methode

Standardmäßig wird die Geschichte nicht in einem Zug geschrieben: Das Schreiben folgt der
Methode zweier Open-Source-Forschungsgeneratoren für Geschichten (MIT-Lizenz),
[Re3](https://github.com/yangkevin2/emnlp22-re3-story-generation) (*Recursive Reprompting and
Revision*, EMNLP 2022) und [DOC](https://github.com/yangkevin2/doc-story-generation)
(*Detailed Outline Control*, ACL 2023), angepasst an die Vorgabe des Inventars:

1. **Plan** (Modul *Plan* von Re3): Das LLM erstellt als strukturiertes JSON eine Prämisse,
   eine Kulisse, die Figuren und 3 bis 5 Schritte. Jede Figur nennt **die Referenz einer Figur**
   aus der Packung, jeder Schritt **die Referenzen der verwendeten Gegenstände**.
2. **Prüfung des Plans durch den Code**: Jede Referenz muss im Inventar existieren, eine Rolle
   muss eine Figur sein, und eine Figur spielt nicht mehr Rollen, als Exemplare in der Packung
   sind. Stimmt etwas nicht, wird der Plan noch einmal mit der Fehlerliste angefordert; bleiben
   Abweichungen, werden die ungültigen Referenzen entfernt. Dieser Schritt ersetzt die
   trainierten Ranking-Modelle von Re3. Ist der Plan immer noch unbrauchbar (kaputtes JSON,
   häufig bei kleinen lokalen Modellen), wird die Geschichte in einem einzigen Aufruf
   geschrieben, wie mit `--method direct`.
3. **Entwurf** (detaillierte Gliederungssteuerung von DOC): ein Teil pro Schritt, gelenkt vom
   Plan, den Figuren und Gegenständen des Schritts und dem bisher geschriebenen Text.
4. **Überarbeitung** (Modul *Edit* von Re3): Ein Prüfer listet als JSON die Abweichungen von der
   Packung auf (fehlender Gegenstand, überschrittene Menge, widersprochenes Aussehen), und mit
   `--citations` ergänzt der Code die zitierten Referenzen, die die Packung nicht enthält; gibt
   es welche, korrigiert sie eine Neufassung. `--reviews 2` (oder 3) prüft die neu gefasste
   Geschichte erneut, solange Befunde bleiben.
5. **„In your box“**: Die endgültige Liste der Figuren und Gegenstände wird **vom Code** aus dem
   geprüften Plan erzeugt und nennt daher nur Teile aus der Packung. Gegenstände, die PlaymoDB
   auf mehrere Zeilen aufteilt (Körper und Flügel des Papageis, Truhe und Schloss, Ruderboot und
   Ruder…), werden zusammengefasst.

Das sind 5 bis 9 LLM-Aufrufe statt einem (2 mehr pro zusätzlicher Prüfrunde): Die Geschichte erscheint am Ende, die Schritte auf
`stderr`. `--method direct` kehrt zu einem einzigen gestreamten Aufruf zurück.

Ein anderes LLM lässt sich anbinden, indem man den Trait `StoryGenerator` implementiert, eine
andere Datenquelle über `InventorySource` (oder ganz ohne Code über `--inventory-url`), und der
Ablauf lässt sich über den Trait `Observer` verfolgen.

## Entwicklung

```bash
cargo test     # offline: HTML fixtures, fake Claude server, scripted LLM, end-to-end
cargo clippy --all-targets
cargo fmt
```

### Das LLM umgehen (End-to-End-Tests)

`--replay DIR` ersetzt das LLM durch Dateien: Jeder Aufruf schreibt seinen Prompt nach
`DIR/NN-text.prompt.txt` (oder `NN-json`) und liest die Antwort aus `DIR/NN-text.answer.txt`.
Fehlt eine Antwort, hält das Programm an und sagt, welche Datei du schreiben musst; starte es
erneut, und die bereits geschriebenen Antworten werden wieder abgespielt. Der Rest der Pipeline
läuft echt. Mit `--inventory-file` ist kein Netzwerkzugriff nötig:

```bash
rag-mobil 3799 --inventory-json > 3799.json          # once
rag-mobil 3799 --inventory-file 3799.json --replay session/
# … write session/01-json.answer.txt (the plan), run again, and so on
```

Eine vollständige Sitzung wird zu einem End-to-End-Test in `tests/e2e/<sets>/`
(`inventory.json`, `answers/` und die erwarteten Ausgaben in `expected/`: fertige Geschichte und
jeder Prompt). Zwei Beispiele, mit von Claude geschriebenen Antworten:

- `tests/e2e/3799/`: „Pirates Secret Island“, deren Prüfung einen Schlüssel entdeckt, der nicht
  in der Packung ist;
- `tests/e2e/3799+70145/`: dieselbe Insel zusammen mit dem „Diving Scooter“, deren Prüfung eine
  Angel und eine Taucherlampe entdeckt, die in beiden Packungen fehlen.

Nach einer gewollten Änderung der Prompts oder der Ausgabe:

```bash
UPDATE_GOLDEN=1 cargo test --test e2e_replay
```

## Hinweis

Inoffizielles Projekt, nicht mit geobra Brandstätter Stiftung & Co. KG verbunden.
PLAYMOBIL® ist eine eingetragene Marke der geobra Brandstätter Stiftung & Co. KG.
Die Inventardaten stammen standardmäßig von [PlaymoDB](https://playmodb.org).

## Lizenz

[MIT](LICENSE). Die Inventardaten und Bilder von PlaymoDB fallen nicht unter diese Lizenz.
