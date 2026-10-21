# rag-mobil 🏰

[English](README.md) · Français · [Deutsch](README.de.md)

[![CI](https://github.com/krisanselmo/rag-mobil/actions/workflows/ci.yml/badge.svg)](https://github.com/krisanselmo/rag-mobil/actions/workflows/ci.yml)

Générateur d'histoires Playmobil basé sur du **RAG** (Retrieval-Augmented Generation) :
à partir d'un ou plusieurs numéros de set, `rag-mobil` récupère l'inventaire exact des boîtes sur
[PlaymoDB](https://playmodb.org) (ou dans [une autre base](#sources-dinventaire)) puis demande à un LLM (Claude par défaut, ou OpenAI,
Gemini, Groq, un modèle local via Ollama…) d'écrire une histoire qu'un enfant
peut rejouer **avec ce qu'il a vraiment dans sa boîte** : mêmes figurines, mêmes objets,
mêmes quantités.

```
numéro de set ──► Retrieve ──► Augment ──► Rédaction ─────────────────► histoire
                  PlaymoDB     contexte    Plan → Rédaction → Relecture
                  ou autre     + prompt    (Re3/DOC), via un LLM (genai)
                  source
```

## Exemple

[**Pepper and the Pirate's Hat**](examples/3799-pirates-secret-island/) : une histoire en anglais
pour le set 3799 « Pirates Secret Island », écrite par Claude, avec son
[livret illustré (PDF)](examples/3799-pirates-secret-island/booklet.pdf). Chaque figurine et
chaque objet de l'histoire est dans la boîte.

## Installation

```bash
cargo build --release
```

Puis, selon le fournisseur choisi :

| Modèle (`-m`) | Fournisseur | Prérequis |
|---|---|---|
| `claude-opus-5-5` (défaut), `claude-…` | Anthropic | `ANTHROPIC_API_KEY` |
| `gpt-…` | OpenAI | `OPENAI_API_KEY` |
| `gemini-…` | Google Gemini | `GEMINI_API_KEY` |
| `ollama::<modèle>` (ex. `ollama::llama3.2`) | [Ollama](https://ollama.com), local et gratuit | `ollama pull <modèle>`, pas de clé |
| `openai::<modèle>` + `RAG_MOBIL_OPENAI_BASE_URL` | un serveur compatible OpenAI, local ou non (llama.cpp, vLLM, LM Studio…) | l'adresse du serveur ; pas de clé si `OPENAI_API_KEY` est absente |

D'autres fournisseurs sont pris en charge (Groq, DeepSeek, xAI, Cohere, OpenRouter, Bedrock…) :
voir la [liste de genai](https://github.com/jeremychone/rust-genai).

## Utilisation

```bash
# Histoire en anglais pour un enfant de 7 ans (par défaut)
rag-mobil 3666

# En français : --language français (ou RAG_MOBIL_LANGUAGE=français)
rag-mobil 3666 --language français

# Plusieurs boîtes : une seule histoire qui mélange leur contenu
rag-mobil 3799 70145

# Personnalisée
rag-mobil 5135 --age 5 --words 400 --theme "une tempête et un trésor perdu"
rag-mobil 3666 --language français --effort high

# Rédaction en un seul appel, affichée au fil de l'écriture (plus rapide)
rag-mobil 3666 --method direct

# Livret illustré, avec la mise en place de chaque scène : HTML, ou PDF à imprimer
rag-mobil 3799 --scene-setup --format html > livret.html
rag-mobil 3799 --scene-setup --format pdf > livret.pdf
rag-mobil 3799 --scene-setup --format pdf --layout compact > livret.pdf   # 2 pages au lieu de 7

# Histoire dont tu es le héros : paragraphes numérotés, choix, plusieurs fins
rag-mobil 3799 --interactive --format pdf > aventure.pdf

# Une saga : chaque histoire est la suite de la précédente, mêmes personnages
rag-mobil 3799 --saga pirates.json      # épisode 1
rag-mobil 3799 70145 --saga pirates.json  # épisode 2, avec une nouvelle boîte

# Le perroquet est la pièce préférée : il est au cœur de l'histoire
rag-mobil 3799 --star "30 06 368"

# La boîte réelle : le perroquet est perdu, 2 hallebardes sur 4 aussi
rag-mobil 3666 --missing "30 06 368" --missing "30 07 640:2"

# Autres fournisseurs
rag-mobil 3666 -m gpt-5.4-mini
rag-mobil 3666 -m ollama::llama3.2        # local, sans clé API
RAG_MOBIL_OPENAI_BASE_URL=http://localhost:8080/v1 rag-mobil 3666 -m openai::ministral-3-3b  # llama.cpp, vLLM…

# Sans appel au LLM (pas besoin de clé API)
rag-mobil 3666 --inventory-json   # inventaire récupéré, en JSON
rag-mobil 3666 --dry-run          # premier prompt envoyé au LLM
```

Les exemples ci-dessous qui citent du texte (« Dans ta boîte », « Mise en place »…) supposent `--language français` ; en anglais, ces titres deviennent « In your box », « Set the scene »…

L'histoire s'affiche sur `stdout` (en Markdown) ; les étapes et messages de progression
vont sur `stderr` (`rag-mobil 3666 > histoire.md` fonctionne).

| Option | Défaut | Rôle |
|---|---|---|
| `-l, --language` / `RAG_MOBIL_LANGUAGE` | `English` | Langue de l'histoire (`français` pour une histoire en français) |
| `-a, --age` / `RAG_MOBIL_AGE` | `7` | Âge du lecteur |
| `-w, --words` / `RAG_MOBIL_WORDS` | `600` | Longueur visée |
| `-t, --theme` | — | Souhait libre |
| `--citations` / `RAG_MOBIL_CITATIONS` | off | Cite chaque pièce nommée par sa référence (voir ci-dessous) |
| `--method` | `planned` | `planned` (plan, rédaction, relecture), `direct` (un appel) ou `interactive` |
| `--interactive` | off | Histoire dont tu es le héros (raccourci de `--method interactive`, voir ci-dessous) |
| `--saga` / `RAG_MOBIL_SAGA` | — | Fichier JSON des épisodes : l'histoire en est la suite, puis s'y ajoute (voir ci-dessous) |
| `--scene-setup` / `RAG_MOBIL_SCENE_SETUP` | off | Méthode `planned` : « Mise en place » avant chaque partie (voir ci-dessous) |
| `-f, --format` / `RAG_MOBIL_FORMAT` | `markdown` | `markdown`, `html` ou `pdf` (livret illustré, voir ci-dessous) |
| `--booklet FICHIER` (alias `--livret`) | — | Écrit aussi le livret (HTML, ou PDF si `.pdf`) : l'histoire reste en Markdown sur `stdout`, en un seul passage du LLM |
| `--layout` / `RAG_MOBIL_LAYOUT` | `booklet` | `--format html` ou `pdf` : `booklet` (livret) ou `compact` (économe en papier) |
| `--browser-arg` | — | `--format pdf` : argument passé au navigateur (ex. `--no-sandbox`) ; répétable |
| `--star REF` | — | Pièce préférée de l'enfant, au cœur de l'histoire ; répétable (voir ci-dessous) |
| `--missing REF[:N]` | — | Pièce absente de la boîte : toutes les copies, ou N ; répétable |
| `--missing-file` / `RAG_MOBIL_MISSING_FILE` | — | Pièces absentes, une par ligne (voir ci-dessous) |
| `--no-review` | off | Méthode `planned` : saute la relecture de fidélité |
| `--include-all` | off | Garde aussi les notices papier dans le contexte |
| `-m, --model` / `RAG_MOBIL_MODEL` | `claude-opus-5-5` | Modèle ; le fournisseur est déduit du nom |
| `--effort` | `medium` pour Claude, défaut du fournisseur sinon | `low`, `medium`, `high`, `xhigh`, `max` |
| `--fallback-model` | `claude-opus-4-8` | Modèle relancé une fois si Claude décline la requête |
| `--no-fallback` | off | Désactive cette nouvelle tentative |
| `--playmodb-url` / `PLAYMODB_BASE_URL` | `https://playmodb.org/cgi-bin` | Miroir PlaymoDB |
| `--cache-dir` / `RAG_MOBIL_CACHE_DIR` | `~/.cache/rag-mobil/playmodb` | Dossier du cache des inventaires |
| `--refresh` | off | Interroge PlaymoDB même si le cache est à jour |
| `--no-cache` | off | N'utilise pas le cache |
| `--inventory-file` | — | Inventaires lus dans un fichier JSON (sortie de `--inventory-json`) |
| `--inventory-url` / `RAG_MOBIL_INVENTORY_URL` | — | Source HTTP d'inventaires (`{set}` = numéro du set), PlaymoDB en repli |
| `--no-playmodb` | off | Avec `--inventory-url` : pas de repli sur PlaymoDB |
| `--replay` | — | Contourne le LLM : prompts et réponses dans des fichiers (voir Développement) |

Les variables d'environnement fixent les défauts d'un déploiement ; une option tapée l'emporte.

### Citations des pièces

Avec `--citations`, chaque figurine ou pièce nommée est suivie de sa référence d'inventaire,
dans l'histoire comme dans « Dans ta boîte » : `[ref. 30 06 368]`, ou
`[ref. 30 06 198, 30 06 199]` pour un objet en plusieurs pièces. Le format est stable : un
affichage les retire (`\s*\[ref\.[^\]]*\]`) ou fait de chaque référence un lien vers la pièce.
Dans la méthode `planned`, la section « Dans ta boîte » est écrite par le code, donc ses
citations sont toujours exactes ; dans le texte, c'est le LLM qui les recopie.

### Histoire dont tu es le héros

Avec `--interactive`, le plan n'est plus une suite d'étapes mais un graphe : 6 à 9
paragraphes, chacun terminé par 2 ou 3 choix qui renvoient à un autre paragraphe, et 2 à
4 fins. Comme pour le plan linéaire, chaque personnage et chaque objet est une
référence vérifiée contre la boîte ; le code vérifie aussi le graphe : chaque choix mène
à un paragraphe qui existe, aucun paragraphe n'est inatteignable, pas de boucle, au
moins 2 fins. En cas d'écart, le plan est redemandé une fois avec la liste des
erreurs ; un graphe encore cassé fait écrire l'histoire en un seul appel (non
interactive).

Les paragraphes sont numérotés dans l'ordre de lecture (en largeur depuis le début), et
chacun est rédigé avec, comme contexte, les paragraphes qui y mènent. La relecture voit
tous les paragraphes ; seuls ceux dont elle cite un passage sont réécrits.

```markdown
## 1

Coco le perroquet se posa sur le hamac de Tom…

**Que choisis-tu ?**

- Pépin propose de faire le tour de l'île en barque → **va au 2**
- Robinson veut d'abord fouiller la grotte → **va au 3**
```

En HTML et en PDF, les choix sont des liens vers les paragraphes, et un paragraphe
imprimé ne se coupe pas entre deux pages. `--scene-setup` ajoute la mise en place de
chaque paragraphe.

### Sagas

`--saga pirates.json` enchaîne les histoires : le fichier (créé s'il n'existe pas)
garde chaque épisode raconté — boîtes, titre, prémisse, personnages (nom, figurine,
portrait) et étapes — et le plan de l'épisode suivant les reçoit dans
`<previous_episodes>`, avec la consigne de ne pas répéter l'intrigue. Les personnages
déjà connus dont la figurine est dans la boîte sont proposés avec leur nom et leur
caractère ; les autres ne peuvent qu'être évoqués.

Le code vérifie la continuité, figurine par figurine : une figurine déjà connue garde son
nom (« la figurine 30 13 020 s'appelait « Pépin » : garde ce nom »), et un nom déjà
donné ne passe pas à une autre figurine. Le plan est redemandé une fois ; s'il reste des
écarts, les noms d'origine sont remis. On peut changer de boîte d'un épisode à l'autre :
seules les figurines présentes reviennent. L'épisode est ajouté au fichier une fois
l'histoire écrite, sauf si le plan a échoué (rédaction en un seul appel). `--saga`
demande la méthode `planned` ou `interactive` ; une histoire interactive est gardée
sans ses étapes, puisque son déroulé dépend du lecteur.

### Pièce préférée

`--star REF` met la pièce préférée de l'enfant au cœur de l'histoire : une figurine
devient le personnage principal, un objet joue un rôle central. La référence s'écrit
avec ou sans espaces, comme pour `--missing` ; elle doit être dans la boîte (après
retrait des pièces absentes), sinon le programme s'arrête avant tout appel au LLM.
L'option est répétable.

Avec les méthodes `planned` et `interactive`, le code vérifie le plan : une figurine
préférée doit jouer un personnage, présent dès la première étape et dans au moins la
moitié des étapes ; un objet préféré doit servir dans au moins 2 étapes. En cas
d'écart, le plan est redemandé une fois avec ces erreurs ; s'il en reste, le code
ajoute la pièce aux premières étapes qui ne l'ont pas. Une figurine qui ne joue aucun
personnage ne peut pas être ajoutée, faute de nom : l'histoire est écrite sans elle
au premier plan. Avec `--method direct`, seule la consigne est donnée au LLM.

```bash
rag-mobil 3666 --star "30 00 496"                        # le trompettiste en héros
rag-mobil 3799 --star 3006368 --star "30 13 020" --interactive
```

### Pièces perdues ou cassées

L'inventaire décrit la boîte neuve ; la vraie boîte a souvent perdu des pièces. `--missing`
les retire de l'inventaire avant tout le reste : le plan ne peut plus les choisir, la
relecture les signale, « Dans ta boîte » ne les cite pas.

```bash
rag-mobil 3666 --missing "30 06 368"          # toutes les copies
rag-mobil 3666 --missing "30 07 640:2"        # 2 hallebardes perdues sur 4
rag-mobil 3666 --missing-file ma-boite.txt    # la liste, gardée dans un fichier
```

```text
# ma-boite.txt : une pièce par ligne, au format de --missing
30 06 368        # le perroquet
30 07 640:2
```

La référence s'écrit avec ou sans espaces (`3007640`, `réf. 30 07 640`). Avec plusieurs
boîtes, les copies sont retirées de la première boîte qui les contient, puis des
suivantes. Une pièce dont la quantité est inconnue (`?`) n'est retirée que sans `:N`.
Chaque pièce est rapportée sur `stderr` (`retirée`, `1 restant(s)`, `absente des
boîtes, ignorée`). `--inventory-json` montre l'inventaire après retrait.

### Mise en place des scènes

Avec `--scene-setup` (méthode `planned`), chaque partie de l'histoire commence par ce
qu'il faut installer pour la jouer, tiré du plan validé et écrit **par le code**, donc
sans pièce inventée :

```markdown
> 🧱 **Mise en place**: Pépin (Pirate, red vest and ponytail); Rowboat, one-seater (dark brown) + Rowboat gear: seat, oarlocks, paddles (light brown), Lantern (silver/yellow)
```

Pour que chaque mise en place reste devant la bonne partie, la réécriture qui suit la
relecture reçoit les parties séparées par des lignes `* * *` et doit les garder. Si elle
les fusionne, l'histoire corrigée est gardée, sans mise en place (message sur `stderr`).

### Livret HTML et PDF

`--format html` écrit une page HTML autonome (styles inclus, thème clair ou sombre)
au lieu du Markdown :

- une couverture : titre, portraits des personnages, prémisse, boîtes utilisées ;
- une vignette de chaque pièce (champ `image` de l'inventaire, images de PlaymoDB) ;
- la mise en place de chaque scène en cartes illustrées, avec `--scene-setup` ;
- « Dans ta boîte » en cartes à cocher : personnages, objets, quantités, référence, boîte ;
- avec `--citations`, chaque `[ref. …]` du texte devient la vignette de la pièce.

Le HTML que contiendrait le texte du LLM est affiché comme du texte, et seules les
images en `http(s)` sont reprises. Les images sont chargées depuis PlaymoDB à
l'ouverture de la page. Avec `--format html` ou `pdf`, l'histoire s'affiche à la fin,
même avec `--method direct`.

Imprimée (ou enregistrée en PDF depuis le navigateur), la page devient un livret A4 :
couverture encadrée sur la première page, une page par scène avec une mise en place
compacte, texte en 14 pt justifié avec césure, « Dans ta boîte » sur une nouvelle page
avec une case à cocher par pièce pour rassembler les pièces avant de jouer, numéros de
page (sauf sur la couverture).

`--layout compact` garde le même contenu mais économise le papier : un en-tête court
au lieu de la couverture, l'histoire sur deux colonnes sans saut de page, des mises en
place et un « Dans ta boîte » en petites vignettes, juste après l'histoire. Pour le set
3799 avec `--scene-setup`, 2 pages au lieu de 7 (3 au lieu de 8 en interactif).

`--booklet livret.pdf` (ou `.html`) écrit le livret dans un fichier en plus de la sortie choisie par
`--format` : un service garde ainsi l'histoire en Markdown et son livret, pour un seul passage du LLM. Un
livret qui ne s'écrit pas (navigateur absent) est signalé sur `stderr` sans perdre l'histoire.

`--format pdf` fait cette impression directement : le livret est imprimé par un Chrome,
Chromium ou Edge sans interface, déjà installé (cherché dans le `PATH` et aux
emplacements habituels, ou indiqué par `RAG_MOBIL_BROWSER`). Le PDF s'écrit sur
`stdout`, à rediriger vers un fichier. Le navigateur est cherché avant tout appel au
LLM. Dans un conteneur lancé en root, Chromium demande `--browser-arg --no-sandbox`.

## Architecture

```
src/
├── main.rs              CLI (clap)
├── lib.rs
├── error.rs             RagError (thiserror), un variant par étape
├── model.rs             SetNumber, Part, SetInventory, pièces absentes
├── pipeline.rs          RagPipeline<S: InventorySource, G: StoryGenerator>
├── saga.rs              --saga : épisodes précédents, continuité des personnages
├── html.rs              --format html : livret illustré (pulldown-cmark)
├── pdf.rs               --format pdf : le livret imprimé par Chrome sans interface
├── retriever/           R — récupération
│   ├── mod.rs           trait InventorySource
│   ├── playmodb.rs      client HTTP (2 pages en parallèle)
│   ├── parser.rs        parsing HTML pur, testé sur fixtures
│   ├── cache.rs         cache disque, un fichier JSON par set
│   ├── json_file.rs     --inventory-file
│   ├── http_json.rs     source HTTP au format v1 (--inventory-url)
│   └── fallback.rs      une source, puis une autre pour les sets inconnus
├── augment/             A — augmentation
│   ├── context.rs       inventaire → contexte structuré (<set>, <characters>, <pieces>)
│   └── prompt.rs        prompt système (règles de fidélité) + message utilisateur
├── generator/           G — génération
│   ├── mod.rs           trait StoryGenerator (texte en streaming, JSON structuré)
│   └── llm.rs           générateur multi-fournisseurs (genai) + repli en cas de refus
└── writer/              rédaction
    ├── mod.rs           méthodes direct / planned, Observer (progression)
    ├── plan.rs          plan, vérification contre l'inventaire, mise en place, « Dans ta boîte »
    ├── interactive.rs   histoire dont tu es le héros : graphe de paragraphes, vérifié
    ├── prompts.rs       prompts plan / rédaction / relecture / correction
    └── planned.rs       Plan → Rédaction → Relecture (adapté de Re3/DOC)
```

### Sources d'inventaire

Par défaut, les inventaires viennent de PlaymoDB (voir ci-dessous). Toute autre base peut
les fournir, au [format d'inventaire v1](docs/inventory-format.fr.md) (schéma JSON :
[`schema/inventory-v1.schema.json`](schema/inventory-v1.schema.json)), le même que
celui de `--inventory-json` et du cache :

```bash
# Un service qui répond GET /api/inventories/<set> ; PlaymoDB pour les sets qu'il ne connaît pas
rag-mobil 3666 70145 --inventory-url 'http://localhost:8096/api/inventories/{set}'

# Ce service seulement
rag-mobil 3666 --inventory-url 'http://localhost:8096/api/inventories/{set}' --no-playmodb

# Un fichier
rag-mobil 3666 --inventory-file mes-boites.json
```

| Source | Quand | Repli |
|---|---|---|
| `--inventory-file` | un fichier, un set ou un tableau | aucun |
| `--inventory-url` / `RAG_MOBIL_INVENTORY_URL` | service HTTP, `{set}` remplacé par le numéro | PlaymoDB (avec cache) si le service répond `404`, sauf `--no-playmodb` |
| (rien) | PlaymoDB, avec cache sauf `--no-cache` | aucun |

Avec `--inventory-url`, seule une réponse `404` passe la main à PlaymoDB ; une autre
erreur (service en panne, JSON invalide, autre set que celui demandé, inventaire vide)
arrête le programme. `--cache-dir`, `--refresh` et `--no-cache` s'appliquent au repli
PlaymoDB. `--inventory-file` donné sur la ligne de commande l'emporte sur
`RAG_MOBIL_INVENTORY_URL`. La ligne `[rag-mobil] inventaires : 3666 (localhost:8096),
70145 (cache)` indique d'où vient chaque set.

Dans un inventaire, la catégorie `Klicky` (PlaymoDB) ou `figure` désigne une figurine,
c'est-à-dire un personnage possible ; les autres pièces sont regroupées par catégorie.

### Retrieve — PlaymoDB

Deux pages sont récupérées en parallèle puis fusionnées :

- `showinv.pl?setnum=N&pics=on` : nom, thème, date de sortie, **catégorie** et **image**
  de chaque pièce (`Klicky` = figurines, `Animal`, `Furniture`, `Vehicle`…). Les deux mises
  en page de PlaymoDB sont gérées (liste de liens, blocs `lineup` avec photos). L'image
  (champ `image`, adresse absolue) ne sert pas au prompt : elle est pour qui affiche l'inventaire.
- `listinv.pl?setnum=N` : tableau texte avec les **quantités** et les sous-éléments
  (perruque, chapeau… rattachés à une figurine).

Les pièces de la section « Extra parts not included in inventory » (numéros alternatifs
ou pièces de rechange) sont écartées pour que le modèle ne compte pas deux fois le même
objet. Pour les sets que PlaymoDB n'a pas encore dénombrés, la quantité est inconnue (`?`). Une pièce listée sans
nombre sous le même nom qu'une pièce comptée est un numéro alternatif, écarté lui aussi ; toute
autre pièce sans nombre est gardée, de quantité inconnue (le set 3693 ne compte que 7 de ses
17 pièces).

Le scraper s'identifie avec un User-Agent explicite et ne fait que deux requêtes par
set ; en cas d'erreur passagère (PlaymoDB répond souvent 508, limite de ressources de
son hébergement), il retente jusqu'à 3 fois, après 1, 3 puis 6 secondes. PlaymoDB est
un site maintenu bénévolement par des fans : merci de ne pas l'utiliser pour du scraping
massif.

**Cache** : chaque inventaire récupéré est enregistré sur le disque
(`~/.cache/rag-mobil/playmodb/<set>.json` sous Linux, `~/Library/Caches` sous macOS,
`%LOCALAPPDATA%` sous Windows) et réutilisé pendant 30 jours sans rappeler PlaymoDB.
`--refresh` force une nouvelle récupération, `--no-cache` désactive le cache. Une entrée
illisible est simplement récupérée à nouveau.

**Plusieurs sets** : `rag-mobil 3799 70145` récupère les inventaires l'un après l'autre
(doublons ignorés), sans limite de nombre. Le contexte contient un bloc `<set>` par
boîte, et le prompt précise que tout se rencontre dans la même histoire. Les personnages
peuvent venir de n'importe quelle boîte, et une référence présente dans plusieurs boîtes
cumule leurs exemplaires. « Dans ta boîte » indique pour chaque ligne dans quelle boîte
chercher (`(boîte 70145)`). Avec beaucoup de grosses boîtes, le prompt s'allonge :
c'est sans conséquence pour Claude, mais un petit modèle local peut saturer.

### Augment — contexte et prompt

L'inventaire devient un bloc pseudo-XML : les figurines d'abord (ce sont les
personnages), puis les pièces par catégorie, avec quantités et couleurs. Le prompt
système fixe les règles de fidélité : personnages issus de la boîte uniquement, objets
présents dans l'inventaire, quantités respectées, pièces techniques utilisées seulement
pour décrire le décor. L'histoire se termine par une section « Dans ta boîte » listant
les éléments utilisés.

### Generate — LLM via genai

La génération s'appuie sur la bibliothèque open source
[`genai`](https://github.com/jeremychone/rust-genai) (MIT/Apache-2.0), qui fournit un
client unique pour une vingtaine de fournisseurs, en streaming. `generator/llm.rs` y
ajoute ce qui est propre au projet :

- **Claude** : `max_tokens` à 64 000 et effort `medium` par défaut, que genai traduit en
  `thinking: {type: "adaptive"}` + `output_config.effort` ;
- **refus** : si le modèle décline la requête (`stop_reason: "refusal"` chez Claude, filtre
  de contenu ailleurs), le texte partiel est abandonné ; pour Claude, une seule nouvelle
  tentative est faite sur `--fallback-model` ;
- **troncature** : un arrêt sur `max_tokens` est signalé.

Les autres fournisseurs gardent leurs réglages par défaut (beaucoup de modèles locaux
refusent les paramètres de raisonnement) ; `--effort` permet de les forcer.

### Rédaction — méthode Re3/DOC

Par défaut, l'histoire n'est pas écrite d'un seul jet : la rédaction reprend la méthode
de deux générateurs d'histoires open source issus de la recherche (licence MIT),
[Re3](https://github.com/yangkevin2/emnlp22-re3-story-generation) (*Recursive
Reprompting and Revision*, EMNLP 2022) et
[DOC](https://github.com/yangkevin2/doc-story-generation) (*Detailed Outline
Control*, ACL 2023), adaptée à la contrainte d'inventaire :

1. **Plan** (module *Plan* de Re3) : le LLM produit en JSON structuré une prémisse, un
   décor, les personnages et 3 à 5 étapes. Chaque personnage désigne **la référence d'une
   figurine** de la boîte, chaque étape **les références des objets** utilisés.
2. **Vérification du plan, par le code** : chaque référence doit exister dans
   l'inventaire, un personnage doit être une figurine, une figurine ne joue pas plus de
   rôles que d'exemplaires dans la boîte. En cas d'écart, le plan est redemandé une fois
   avec la liste des erreurs ; s'il reste des écarts, les références invalides sont
   retirées. Cette étape remplace les modèles de classement entraînés de Re3. Si le
   plan reste inutilisable (JSON cassé, fréquent avec les petits modèles locaux),
   l'histoire est écrite en un seul appel, comme avec `--method direct`.
3. **Rédaction** (contrôle par plan détaillé de DOC) : une partie par étape, guidée par
   le plan, les personnages et objets de l'étape et le texte déjà écrit.
4. **Relecture** (module *Edit* de Re3) : un relecteur liste en JSON les écarts avec la
   boîte (objet absent, quantité dépassée, apparence contredite) ; s'il y en a, une
   seule réécriture les corrige.
5. **« Dans ta boîte »** : la liste finale des personnages et objets est générée **par
   le code** à partir du plan validé, elle ne cite donc que des pièces de la boîte. Les
   objets que PlaymoDB découpe en plusieurs lignes (corps et ailes du perroquet, coffre
   et serrure, barque et rames…) y sont regroupés.

Cela représente 5 à 9 appels au LLM au lieu d'un : l'histoire s'affiche à la fin, les
étapes sur `stderr`. `--method direct` revient à un seul appel en streaming.

Un autre LLM peut être branché en implémentant le trait `StoryGenerator`, une autre
source de données via `InventorySource` (ou sans code, avec `--inventory-url`), et le
déroulé suivi via le trait `Observer`.

## Développement

```bash
cargo test     # hors-ligne : fixtures HTML, faux serveur Claude, LLM scripté, bout en bout
cargo clippy --all-targets
cargo fmt
```

### Contourner le LLM (tests de bout en bout)

`--replay DOSSIER` remplace le LLM par des fichiers : chaque appel écrit son prompt
dans `DOSSIER/NN-text.prompt.txt` (ou `NN-json`) et lit la réponse dans
`DOSSIER/NN-text.answer.txt`. S'il manque une réponse, le programme s'arrête en
indiquant quel fichier écrire ; on relance et les réponses déjà écrites sont rejouées.
Tout le reste du pipeline tourne pour de vrai. Avec `--inventory-file`, aucun accès
réseau n'est nécessaire :

```bash
rag-mobil 3799 --inventory-json > 3799.json          # une fois
rag-mobil 3799 --inventory-file 3799.json --replay session/
# … écrire session/01-json.answer.txt (le plan), relancer, et ainsi de suite
```

Une session complète devient un test de bout en bout dans `tests/e2e/<sets>/`
(`inventory.json`, `answers/`, et les sorties attendues dans `expected/` : histoire
finale et chaque prompt). Deux exemples, aux réponses écrites par Claude :

- `tests/e2e/3799/` : « Pirates Secret Island », dont la relecture repère une clé
  absente de la boîte ;
- `tests/e2e/3799+70145/` : la même île et le « Diving Scooter » réunis, dont la
  relecture repère une canne à pêche et une lampe de plongée absentes des deux boîtes.

Après un changement voulu des prompts ou de la sortie :

```bash
UPDATE_GOLDEN=1 cargo test --test e2e_replay
```

## Avertissement

Projet non officiel, sans lien avec geobra Brandstätter Stiftung & Co. KG.
PLAYMOBIL® est une marque déposée de geobra Brandstätter Stiftung & Co. KG.
Par défaut, les données d'inventaire proviennent de [PlaymoDB](https://playmodb.org).

## Licence

[MIT](LICENSE). Les données d'inventaire et les images de PlaymoDB ne sont pas couvertes
par cette licence.
