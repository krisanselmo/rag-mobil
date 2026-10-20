# Format d'inventaire v1

[English](inventory-format.md) · Français

`rag-mobil` n'invente rien : l'histoire est écrite à partir de l'inventaire exact de la
boîte. Ce document décrit le format JSON de cet inventaire, pour brancher une autre base
que [PlaymoDB](https://playmodb.org) (sa propre collection, un autre site, une
application d'inventaire…).

C'est le même format partout :

- ce que renvoie une source HTTP (`--inventory-url`) ;
- ce que lit `--inventory-file` (un inventaire, ou un tableau d'inventaires) ;
- ce qu'écrit `--inventory-json` ;
- les fichiers du cache (`~/.cache/rag-mobil/playmodb/<set>.json`).

Schéma JSON (draft 2020-12) : [`schema/inventory-v1.schema.json`](../schema/inventory-v1.schema.json).
Un test (`tests/inventory_schema.rs`) vérifie qu'il reste conforme au code.

## Exemple

```json
{
  "set_number": "3666",
  "name": "Kings Large Castle",
  "theme": "Knights",
  "focus": null,
  "release_date": "1993/09",
  "parts": [
    {
      "number": "30 00 4950",
      "name": "Castle Trumpeter",
      "color": null,
      "quantity": 1,
      "category": "figure",
      "components": ["Wig (hair), classic male - brown", "Trumpet"]
    },
    {
      "number": "30 21 3500",
      "name": "Halberd",
      "color": "silver",
      "quantity": 4,
      "category": "Tool",
      "components": []
    },
    {
      "number": "30 61 0850",
      "name": "Throne",
      "color": "red",
      "quantity": null,
      "category": "Furniture",
      "components": []
    }
  ],
  "source_url": "https://exemple.org/sets/3666"
}
```

## Champs

« Obligatoire » veut dire que la clé doit être présente. Un champ facultatif peut être
absent ou valoir `null` : les deux veulent dire « inconnu ». Les champs inconnus sont
ignorés (on peut ajouter `"$schema"` ou ses propres champs).

### Le set

| Champ | Type | Obligatoire | Rôle |
|---|---|---|---|
| `set_number` | texte | oui | Numéro du set, **chiffres uniquement** (1 à 8), en texte : `"3666"`, pas `3666`. Doit être le numéro demandé. |
| `name` | texte | oui | Nom du set, repris dans le prompt et le résumé. |
| `theme` | texte ou `null` | non | Thème (gamme) : `Knights`, `Pirates`… |
| `focus` | texte ou `null` | non | Sous-thème. |
| `release_date` | texte ou `null` | non | Date de sortie, en texte libre (PlaymoDB écrit `AAAA/MM`). |
| `parts` | tableau de pièces | oui | Contenu de la boîte. **Ne doit pas être vide** : sans pièce, pas d'histoire. |
| `source_url` | texte | oui | Où un humain peut vérifier l'inventaire. Repris dans le prompt (`source="…"`). Toute chaîne est acceptée. |

### Une pièce (`parts[]`)

| Champ | Type | Obligatoire | Rôle |
|---|---|---|---|
| `number` | texte | oui | Référence de la pièce, **unique dans le set**. Le LLM la cite dans son plan et le code vérifie qu'elle existe : deux pièces différentes ne devraient pas partager la même. |
| `name` | texte | oui | Nom de la pièce, tel que le LLM le lira (l'anglais de PlaymoDB convient, le LLM traduit). |
| `color` | texte ou `null` | non | Couleur, si elle a un sens. |
| `quantity` | entier ≥ 0 ou `null` | non | Nombre d'exemplaires dans la boîte ; `null` si on ne sait pas (affiché `?×`). Le LLM ne doit pas en utiliser plus. |
| `category` | texte | oui | Catégorie, en texte libre (voir plus bas). |
| `components` | tableau de textes | oui | Sous-éléments de la pièce (perruque, chapeau, arme d'une figurine…) ; `[]` s'il n'y en a pas. |
| `image` | texte ou `null` | non | Adresse absolue (`http` ou `https`) d'une image de la pièce. Le prompt ne la lit pas : elle sert à qui affiche l'inventaire (PlaymoDB la donne pour chaque pièce). |

## Catégories

`category` est libre, mais deux choses en dépendent :

- **les figurines** : `Klicky` (le nom PlaymoDB) ou `figure`, sans tenir compte de la
  casse. Ce sont les seules pièces qui peuvent devenir des personnages ; elles ouvrent
  le contexte, dans un bloc `<characters>`. Une source qui n'est pas PlaymoDB devrait
  utiliser `figure`.
- **les autres pièces** sont regroupées par catégorie, dans l'ordre alphabétique, un bloc
  `<pieces category="…">` par catégorie. Des noms courts et parlants aident le LLM
  (`Animal`, `Furniture`, `Vehicle`, `Tool`, `Weapon`…) ; une catégorie fourre-tout
  (`Other`) convient aussi.

La catégorie `Paper` (notices et papiers, sans tenir compte de la casse) est **écartée**
du prompt, sauf avec `--include-all`.

## Écrire une source

### Service HTTP (`--inventory-url`)

```bash
rag-mobil 3666 --inventory-url 'http://localhost:8096/api/inventories/{set}'
```

`{set}` est remplacé par le numéro du set ; l'adresse doit être en `http` ou `https`.
Pour chaque set, `rag-mobil` envoie `GET <adresse>` avec `Accept: application/json`,
un User-Agent `rag-mobil/<version>`, et attend 30 secondes au plus. Le service répond :

| Réponse | Effet |
|---|---|
| `200` et un inventaire v1 | Utilisé tel quel. |
| `404` | Set inconnu : demandé à PlaymoDB (sauf `--no-playmodb`, alors erreur). |
| autre code | Erreur, avec le code et l'adresse. Pas de repli : une source en panne ne doit pas passer inaperçue. |
| JSON invalide ou champ obligatoire manquant | Erreur. |
| `set_number` différent du set demandé | Erreur. |
| `parts` vide | Erreur (« inventaire vide ») : un service qui n'a pas encore les pièces d'un set doit répondre `404`, pour laisser PlaymoDB répondre. |

Il n'y a pas d'authentification : pour un service privé, le mettre sur le réseau local
ou derrière un proxy.

### Fichier (`--inventory-file`)

Un inventaire v1, ou un tableau d'inventaires v1 pour plusieurs sets. Pratique pour une
collection figée ou des tests sans réseau.

### Dans le code

Une source est un type qui implémente le trait `InventorySource`
(`src/retriever/mod.rs`) et répond `RagError::SetNotFound` pour un set qu'elle ne connaît
pas. `Fallback::new(a, b)` interroge `a`, puis `b` seulement pour les sets que `a` ne
connaît pas ; `Cached::new(source, dossier)` ajoute le cache disque.

## Versions

Ce document décrit la version 1. Une évolution compatible (nouveau champ facultatif)
garde le numéro de version. Une évolution incompatible (champ renommé, retiré ou rendu
obligatoire, sens changé) donnera un format v2, avec son propre schéma
(`schema/inventory-v2.schema.json`).
