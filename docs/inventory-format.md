# Inventory format v1

English · [Français](inventory-format.fr.md) · [Deutsch](inventory-format.de.md)

`rag-mobil` makes nothing up: the story is written from the exact inventory of the box. This
document describes the JSON format of that inventory, so you can plug in a database other
than [PlaymoDB](https://playmodb.org) (your own collection, another site, an inventory app…).

It's the same format everywhere:

- what an HTTP source returns (`--inventory-url`);
- what `--inventory-file` reads (one inventory, or an array of inventories);
- what `--inventory-json` writes;
- the cache files (`~/.cache/rag-mobil/playmodb/<set>.json`).

JSON schema (draft 2020-12): [`schema/inventory-v1.schema.json`](../schema/inventory-v1.schema.json).
A test (`tests/inventory_schema.rs`) checks that it stays in line with the code.

## Example

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
  "source_url": "https://example.org/sets/3666"
}
```

## Fields

"Required" means the key must be present. An optional field may be absent or `null`: both
mean "unknown". Unknown fields are ignored (you can add `"$schema"` or fields of your own).

### The set

| Field | Type | Required | Purpose |
|---|---|---|---|
| `set_number` | string | yes | Set number, **digits only** (1 to 8), as a string: `"3666"`, not `3666`. Must be the number requested. |
| `name` | string | yes | Set name, used in the prompt and the summary. |
| `theme` | string or `null` | no | Theme (product line): `Knights`, `Pirates`… |
| `focus` | string or `null` | no | Sub-theme. |
| `release_date` | string or `null` | no | Release date, free text (PlaymoDB writes `YYYY/MM`). |
| `parts` | array of pieces | yes | Contents of the box. **Must not be empty**: no pieces, no story. |
| `source_url` | string | yes | Where a human can check the inventory. Used in the prompt (`source="…"`). Any string is accepted. |

### A piece (`parts[]`)

| Field | Type | Required | Purpose |
|---|---|---|---|
| `number` | string | yes | Piece reference, **unique within the set**. The LLM cites it in its plan and code checks that it exists: two different pieces shouldn't share one. |
| `name` | string | yes | Piece name, as the LLM will read it (PlaymoDB's English is fine; the LLM translates). |
| `color` | string or `null` | no | Color, when it makes sense. |
| `quantity` | integer ≥ 0 or `null` | no | Number of copies in the box; `null` if unknown (shown as `?×`). The LLM must not use more. |
| `category` | string | yes | Category, free text (see below). |
| `components` | array of strings | yes | Sub-items of the piece (a figure's wig, hat, weapon…); `[]` if none. |
| `image` | string or `null` | no | Absolute URL (`http` or `https`) of an image of the piece. The prompt doesn't read it: it's for whoever displays the inventory (PlaymoDB provides one for each piece). |

## Categories

`category` is free, but two things depend on it:

- **figures**: `Klicky` (PlaymoDB's name) or `figure`, case-insensitive. They are the only
  pieces that can become characters; they open the context, in a `<characters>` block. A
  source other than PlaymoDB should use `figure`.
- **other pieces** are grouped by category, in alphabetical order, one
  `<pieces category="…">` block per category. Short, meaningful names help the LLM
  (`Animal`, `Furniture`, `Vehicle`, `Tool`, `Weapon`…); a catch-all category (`Other`) works
  too.

The `Paper` category (instructions and paperwork, case-insensitive) is **left out** of the
prompt, unless `--include-all` is given.

## Writing a source

### HTTP service (`--inventory-url`)

```bash
rag-mobil 3666 --inventory-url 'http://localhost:8096/api/inventories/{set}'
```

`{set}` is replaced by the set number; the URL must be `http` or `https`. For each set,
`rag-mobil` sends `GET <url>` with `Accept: application/json` and a `rag-mobil/<version>`
User-Agent, and waits 30 seconds at most. The service answers:

| Response | Effect |
|---|---|
| `200` and a v1 inventory | Used as is. |
| `404` | Unknown set: asked to PlaymoDB (unless `--no-playmodb`, then an error). |
| any other status | Error, with the status and the URL. No fallback: a broken source mustn't go unnoticed. |
| invalid JSON or missing required field | Error. |
| `set_number` other than the set requested | Error. |
| empty `parts` | Error ("empty inventory"): a service that doesn't have a set's pieces yet should answer `404`, to let PlaymoDB answer. |

There is no authentication: for a private service, keep it on the local network or behind a
proxy.

### File (`--inventory-file`)

One v1 inventory, or an array of v1 inventories for several sets. Handy for a fixed
collection or offline tests.

### In code

A source is a type implementing the `InventorySource` trait (`src/retriever/mod.rs`) that
answers `RagError::SetNotFound` for a set it doesn't know. `Fallback::new(a, b)` queries `a`,
then `b` only for the sets `a` doesn't know; `Cached::new(source, dir)` adds the disk cache.

## Versions

This document describes version 1. A compatible change (a new optional field) keeps the
version number. An incompatible change (a field renamed, removed or made required, a changed
meaning) will make a v2 format, with its own schema (`schema/inventory-v2.schema.json`).
