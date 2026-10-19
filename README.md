# rag-mobil 🏰

Générateur d'histoires Playmobil basé sur du **RAG** (Retrieval-Augmented Generation) :
à partir d'un numéro de set, `rag-mobil` récupère l'inventaire exact de la boîte sur
[PlaymoDB](https://playmodb.org) puis demande à Claude d'écrire une histoire qu'un enfant
peut rejouer **avec ce qu'il a vraiment dans sa boîte** : mêmes figurines, mêmes objets,
mêmes quantités.

```
numéro de set ──► Retrieve ──► Augment ──► Generate ──► histoire
                  PlaymoDB     contexte    Claude
                  (scraping)   + prompt    (streaming)
```

## Installation

```bash
cargo build --release
export ANTHROPIC_API_KEY=sk-ant-...
```

## Utilisation

```bash
# Histoire en français pour un enfant de 7 ans (par défaut)
rag-mobil 3666

# Personnalisée
rag-mobil 5135 --age 5 --words 400 --theme "une tempête et un trésor perdu"
rag-mobil 3666 --language English --effort high

# Sans appel au LLM (pas besoin de clé API)
rag-mobil 3666 --inventory-json   # inventaire récupéré, en JSON
rag-mobil 3666 --dry-run          # prompt augmenté complet
```

L'histoire s'affiche sur `stdout` au fil de la génération ; les messages de progression
vont sur `stderr` (`rag-mobil 3666 > histoire.md` fonctionne).

| Option | Défaut | Rôle |
|---|---|---|
| `-l, --language` | `français` | Langue de l'histoire |
| `-a, --age` | `7` | Âge du lecteur |
| `-w, --words` | `600` | Longueur visée |
| `-t, --theme` | — | Souhait libre |
| `--include-all` | off | Garde aussi les notices papier dans le contexte |
| `--model` / `RAG_MOBIL_MODEL` | `claude-opus-5-5` | Modèle Claude |
| `--effort` | `medium` | `low`, `medium`, `high`, `xhigh`, `max` |
| `--playmodb-url` / `PLAYMODB_BASE_URL` | `https://playmodb.org/cgi-bin` | Miroir PlaymoDB |

`ANTHROPIC_BASE_URL` permet de pointer vers un autre endpoint compatible.

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
│   └── parser.rs        parsing HTML pur, testé sur fixtures
├── augment/             A — augmentation
│   ├── context.rs       inventaire → contexte structuré (<set>, <personnages>, <pieces>)
│   └── prompt.rs        prompt système (règles de fidélité) + message utilisateur
└── generator/           G — génération
    ├── mod.rs           trait StoryGenerator
    ├── anthropic.rs     API Messages de Claude (HTTP brut + SSE)
    └── sse.rs           décodeur Server-Sent Events
```

### Retrieve — PlaymoDB

Deux pages sont récupérées en parallèle puis fusionnées :

- `showinv.pl?setnum=N` : nom, thème, date de sortie et **catégorie** de chaque pièce
  (`Klicky` = figurines, `Animal`, `Furniture`, `Vehicle`…). Les deux mises en page de
  PlaymoDB sont gérées (anciens sets en liste de liens, sets récents en blocs `lineup`).
- `listinv.pl?setnum=N` : tableau texte avec les **quantités** et les sous-éléments
  (perruque, chapeau… rattachés à une figurine).

Les pièces de la section « Extra parts not included in inventory » (numéros alternatifs
ou pièces de rechange) sont écartées pour que le modèle ne compte pas deux fois le même
objet. Pour les sets que PlaymoDB n'a pas encore dénombrés, la quantité est inconnue (`?`).

Le scraper s'identifie avec un User-Agent explicite et ne fait que deux requêtes par
set. PlaymoDB est un site maintenu bénévolement par des fans : merci de ne pas l'utiliser
pour du scraping massif.

### Augment — contexte et prompt

L'inventaire devient un bloc pseudo-XML : les figurines d'abord (ce sont les
personnages), puis les pièces par catégorie, avec quantités et couleurs. Le prompt
système fixe les règles de fidélité : personnages issus de la boîte uniquement, objets
présents dans l'inventaire, quantités respectées, pièces techniques utilisées seulement
pour décrire le décor. L'histoire se termine par une section « Dans ta boîte » listant
les éléments utilisés.

### Generate — Claude

Il n'existe pas de SDK Anthropic officiel pour Rust : `generator/anthropic.rs` appelle
directement `POST /v1/messages` en streaming SSE, avec :

- `thinking: {type: "adaptive"}` et `output_config.effort` (réglable avec `--effort`) ;
- `fallbacks: "default"` (beta `server-side-fallback-2026-07-01`) : si un classifieur de
  sécurité décline la requête, l'API la relance côté serveur sur un modèle de repli ;
- gestion de `stop_reason: "refusal"` (le texte partiel est alors jeté) et de
  `max_tokens` (avertissement de troncature).

Un autre LLM peut être branché en implémentant le trait `StoryGenerator`, et une autre
source de données (cache local, autre base) via `InventorySource`.

## Développement

```bash
cargo test     # tests unitaires hors-ligne (fixtures HTML dans tests/fixtures/)
cargo clippy --all-targets
cargo fmt
```

## Avertissement

Projet non officiel, sans lien avec geobra Brandstätter Stiftung & Co. KG.
PLAYMOBIL® est une marque déposée de geobra Brandstätter Stiftung & Co. KG.
Les données d'inventaire proviennent de [PlaymoDB](https://playmodb.org).
