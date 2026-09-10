/* ============================================================================
   A reader for the ONE TOML shape this repo's licence surfaces use: a file of
   `[[table]]` entries whose values are strings, booleans or integers.

   models.toml and engines.toml are both exactly that, and both are the source
   of truth for a page and for a CI gate. Pulling a general TOML parser into the
   site's dependency tree to read thirty lines of `key = "value"` would add a
   supply-chain surface to a site whose argument is that it has none.

   Anything it does not understand it refuses, loudly, at build time. A licence
   table that silently parses to nothing is the failure that matters here — the
   /models page would render empty and look correct.
   ========================================================================== */

export type Row = Record<string, string | boolean | number>;

export function parseTables(src: string, table: string): Row[] {
  const rows: Row[] = [];
  let cur: Row | null = null;

  src.split(/\r?\n/).forEach((raw, i) => {
    const line = raw.trim();
    if (line === '' || line.startsWith('#')) return;

    if (line === `[[${table}]]`) {
      cur = {};
      rows.push(cur);
      return;
    }
    // Another table's header ends the current entry rather than leaking into it.
    if (/^\[+[^\]]+\]+$/.test(line)) {
      cur = null;
      return;
    }
    if (cur === null) return;

    const m = line.match(/^([A-Za-z_][A-Za-z0-9_-]*)\s*=\s*(.+?)\s*(?:#.*)?$/);
    if (!m) throw new Error(`${table}: cannot parse line ${i + 1}: ${raw}`);

    const [, key, rawValue] = m;
    let value: string | boolean | number;
    if (/^".*"$/.test(rawValue)) value = rawValue.slice(1, -1);
    else if (rawValue === 'true' || rawValue === 'false') value = rawValue === 'true';
    else if (/^-?\d+$/.test(rawValue)) value = Number(rawValue);
    else throw new Error(`${table}: unsupported value on line ${i + 1}: ${raw}`);

    cur[key] = value;
  });

  return rows;
}
