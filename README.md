# rag-mobil 🏰

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
# Histoire en français pour un enfant de 7 ans (par défaut)
rag-mobil 3666

# Plusieurs boîtes : une seule histoire qui mélange leur contenu
rag-mobil 3799 70145

# Personnalisée
rag-mobil 5135 --age 5 --words 400 --theme "une tempête et un trésor perdu"
rag-mobil 3666 --language English --effort high

# Rédaction en un seul appel, affichée au fil de l'écriture (plus rapide)
rag-mobil 3666 --method direct

# Autres fournisseurs
rag-mobil 3666 -m gpt-5.4-mini
rag-mobil 3666 -m ollama::llama3.2        # local, sans clé API
RAG_MOBIL_OPENAI_BASE_URL=http://localhost:8080/v1 rag-mobil 3666 -m openai::ministral-3-3b  # llama.cpp, vLLM…

# Sans appel au LLM (pas besoin de clé API)
rag-mobil 3666 --inventory-json   # inventaire récupéré, en JSON
rag-mobil 3666 --dry-run          # premier prompt envoyé au LLM
```

L'histoire s'affiche sur `stdout` (en Markdown) ; les étapes et messages de progression
vont sur `stderr` (`rag-mobil 3666 > histoire.md` fonctionne).

| Option | Défaut | Rôle |
|---|---|---|
| `-l, --language` / `RAG_MOBIL_LANGUAGE` | `français` | Langue de l'histoire |
| `-a, --age` / `RAG_MOBIL_AGE` | `7` | Âge du lecteur |
| `-w, --words` / `RAG_MOBIL_WORDS` | `600` | Longueur visée |
| `-t, --theme` | — | Souhait libre |
| `--citations` / `RAG_MOBIL_CITATIONS` | off | Cite chaque pièce nommée par sa référence (voir ci-dessous) |
| `--method` | `planned` | `planned` (plan, rédaction, relecture) ou `direct` (un appel) |
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
dans l'histoire comme dans « Dans ta boîte » : `[réf. 30 06 368]`, ou
`[réf. 30 06 198, 30 06 199]` pour un objet en plusieurs pièces. Le format est stable : un
affichage les retire (`\s*\[réf\.[^\]]*\]`) ou fait de chaque référence un lien vers la pièce.
Dans la méthode `planned`, la section « Dans ta boîte » est écrite par le code, donc ses
citations sont toujours exactes ; dans le texte, c'est le LLM qui les recopie.

## Architecture

```
src/
├── main.rs              CLI (clap)
├── lib.rs
├── error.rs             RagError (thiserror), un variant par étape
├── model.rs             SetNumber, Part, SetInventory
├── pipeline.rs          RagPipeline<S: InventorySource, G: StoryGenerator>
├── retriever/           R — récupération
│   ├── mod.rs           trait InventorySource
│   ├── playmodb.rs      client HTTP (2 pages en parallèle)
│   ├── parser.rs        parsing HTML pur, testé sur fixtures
│   ├── cache.rs         cache disque, un fichier JSON par set
│   ├── json_file.rs     --inventory-file
│   ├── http_json.rs     source HTTP au format v1 (--inventory-url)
│   └── fallback.rs      une source, puis une autre pour les sets inconnus
├── augment/             A — augmentation
│   ├── context.rs       inventaire → contexte structuré (<set>, <personnages>, <pieces>)
│   └── prompt.rs        prompt système (règles de fidélité) + message utilisateur
├── generator/           G — génération
│   ├── mod.rs           trait StoryGenerator (texte en streaming, JSON structuré)
│   └── llm.rs           générateur multi-fournisseurs (genai) + repli en cas de refus
└── writer/              rédaction
    ├── mod.rs           méthodes direct / planned, Observer (progression)
    ├── plan.rs          plan, schémas JSON, vérification du plan contre l'inventaire
    ├── prompts.rs       prompts plan / rédaction / relecture / correction
    └── planned.rs       Plan → Rédaction → Relecture (adapté de Re3/DOC)
```

### Sources d'inventaire

Par défaut, les inventaires viennent de PlaymoDB (voir ci-dessous). Toute autre base peut
les fournir, au [format d'inventaire v1](docs/format-inventaire.md) (schéma JSON :
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
objet. Pour les sets que PlaymoDB n'a pas encore dénombrés, la quantité est inconnue (`?`).

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
