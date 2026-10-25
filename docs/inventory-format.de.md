# Inventarformat v1

[English](inventory-format.md) · [Français](inventory-format.fr.md) · Deutsch

`rag-mobil` erfindet nichts: Die Geschichte wird aus dem genauen Inventar der Packung
geschrieben. Dieses Dokument beschreibt das JSON-Format dieses Inventars, damit du eine andere
Datenbank als [PlaymoDB](https://playmodb.org) anbinden kannst (deine eigene Sammlung, eine
andere Website, eine Inventar-App…).

Es ist überall dasselbe Format:

- was eine HTTP-Quelle zurückgibt (`--inventory-url`);
- was `--inventory-file` liest (ein Inventar oder ein Array von Inventaren);
- was `--inventory-json` schreibt;
- die Cache-Dateien (`~/.cache/rag-mobil/playmodb/<set>.json`).

JSON-Schema (Draft 2020-12): [`schema/inventory-v1.schema.json`](../schema/inventory-v1.schema.json).
Ein Test (`tests/inventory_schema.rs`) stellt sicher, dass es mit dem Code übereinstimmt.

## Beispiel

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

## Felder

„Pflicht“ heißt, dass der Schlüssel vorhanden sein muss. Ein optionales Feld darf fehlen oder
`null` sein: Beides bedeutet „unbekannt“. Unbekannte Felder werden ignoriert (du kannst
`"$schema"` oder eigene Felder hinzufügen).

### Das Set

| Feld | Typ | Pflicht | Zweck |
|---|---|---|---|
| `set_number` | String | ja | Setnummer, 1 bis 8 **Ziffern**, bei einer Neuauflage gefolgt von einem Kleinbuchstaben (`"3258a"`), als String: `"3666", nicht `3666`. Muss die angefragte Nummer sein. |
| `name` | String | ja | Name des Sets, verwendet im Prompt und in der Zusammenfassung. |
| `theme` | String oder `null` | nein | Thema (Produktlinie): `Knights`, `Pirates`… |
| `focus` | String oder `null` | nein | Unterthema. |
| `release_date` | String oder `null` | nein | Erscheinungsdatum, freier Text (PlaymoDB schreibt `YYYY/MM`). |
| `parts` | Array von Teilen | ja | Inhalt der Packung. **Darf nicht leer sein**: keine Teile, keine Geschichte. |
| `source_url` | String | ja | Wo ein Mensch das Inventar überprüfen kann. Wird im Prompt verwendet (`source="…"`). Jeder String wird akzeptiert. |

### Ein Teil (`parts[]`)

| Feld | Typ | Pflicht | Zweck |
|---|---|---|---|
| `number` | String | ja | Referenz des Teils, **eindeutig innerhalb des Sets**. Das LLM zitiert sie in seinem Plan und der Code prüft, dass sie existiert: Zwei verschiedene Teile sollten sich keine teilen. |
| `name` | String | ja | Name des Teils, so wie das LLM ihn liest (das Englisch von PlaymoDB passt; das LLM übersetzt). |
| `color` | String oder `null` | nein | Farbe, wenn sie sinnvoll ist. |
| `quantity` | Ganzzahl ≥ 0 oder `null` | nein | Anzahl der Exemplare in der Packung; `null`, wenn unbekannt (angezeigt als `?×`). Das LLM darf nicht mehr verwenden. |
| `category` | String | ja | Kategorie, freier Text (siehe unten). |
| `components` | Array von Strings | ja | Unterteile des Teils (Perücke, Hut, Waffe einer Figur…); `[]`, wenn keine. |
| `image` | String oder `null` | nein | Absolute URL (`http` oder `https`) eines Bilds des Teils. Der Prompt liest es nicht: Es ist für alle gedacht, die das Inventar anzeigen (PlaymoDB liefert eines für jedes Teil). |

## Kategorien

`category` ist frei, aber zwei Dinge hängen davon ab:

- **Figuren**: `Klicky` (der Name bei PlaymoDB) oder `figure`, ohne Beachtung der
  Groß-/Kleinschreibung. Nur sie können zu Rollen der Geschichte werden; sie stehen am Anfang
  des Kontexts, in einem `<characters>`-Block. Eine andere Quelle als PlaymoDB sollte `figure`
  verwenden.
- **Die anderen Teile** werden nach Kategorie gruppiert, in alphabetischer Reihenfolge, ein
  `<pieces category="…">`-Block pro Kategorie. Kurze, aussagekräftige Namen helfen dem LLM
  (`Animal`, `Furniture`, `Vehicle`, `Tool`, `Weapon`…); eine Sammelkategorie (`Other`)
  funktioniert auch.

Die Kategorie `Paper` (Anleitungen und Papierkram, ohne Beachtung der Groß-/Kleinschreibung)
wird aus dem Prompt **weggelassen**, außer mit `--include-all`.

## Eine Quelle schreiben

### HTTP-Dienst (`--inventory-url`)

```bash
rag-mobil 3666 --inventory-url 'http://localhost:8096/api/inventories/{set}'
```

`{set}` wird durch die Setnummer ersetzt; die URL muss `http` oder `https` sein. Für jedes Set
sendet `rag-mobil` `GET <url>` mit `Accept: application/json` und einem User-Agent
`rag-mobil/<version>` und wartet höchstens 30 Sekunden. Der Dienst antwortet:

| Antwort | Wirkung |
|---|---|
| `200` und ein v1-Inventar | Wird so verwendet. |
| `404` | Unbekanntes Set: Es wird bei PlaymoDB angefragt (außer mit `--no-playmodb`, dann ein Fehler). |
| jeder andere Status | Fehler, mit Status und URL. Keine Ausweichquelle: Eine kaputte Quelle darf nicht unbemerkt bleiben. |
| ungültiges JSON oder fehlendes Pflichtfeld | Fehler. |
| `set_number` anders als das angefragte Set | Fehler. |
| leeres `parts` | Fehler („empty inventory“): Ein Dienst, der die Teile eines Sets noch nicht hat, sollte `404` antworten, damit PlaymoDB übernehmen kann. |

Es gibt keine Authentifizierung: Halte einen privaten Dienst im lokalen Netzwerk oder hinter
einem Proxy.

### Datei (`--inventory-file`)

Ein v1-Inventar oder ein Array von v1-Inventaren für mehrere Sets. Praktisch für eine feste
Sammlung oder Offline-Tests.

### Im Code

Eine Quelle ist ein Typ, der den Trait `InventorySource` (`src/retriever/mod.rs`) implementiert
und für ein Set, das er nicht kennt, `RagError::SetNotFound` zurückgibt. `Fallback::new(a, b)`
fragt `a` ab, dann `b` nur für die Sets, die `a` nicht kennt; `Cached::new(source, dir)` fügt
den Festplatten-Cache hinzu.

## Versionen

Dieses Dokument beschreibt Version 1. Eine kompatible Änderung (ein neues optionales Feld)
behält die Versionsnummer. Eine inkompatible Änderung (ein Feld umbenannt, entfernt oder zur
Pflicht gemacht, eine geänderte Bedeutung) führt zu einem Format v2 mit eigenem Schema
(`schema/inventory-v2.schema.json`).
