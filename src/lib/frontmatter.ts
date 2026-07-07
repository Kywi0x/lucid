// Propriétés de page stockées en frontmatter YAML (compatible Obsidian).
// ponytail: sous-ensemble Obsidian volontairement minimal — scalaires (string),
// listes inline `[a, b]` et listes en bloc `- item`. Pas de maps imbriquées ni
// de scalaires multilignes. Si un jour on en a besoin → passer à une vraie lib YAML.

export type PropValue = string | string[];
export interface Prop {
  key: string;
  value: PropValue;
}

const FM = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/;

function unquote(s: string): string {
  return s.replace(/^["']/, "").replace(/["']$/, "");
}

/** Sépare le frontmatter (propriétés) du corps markdown. */
export function parseFrontmatter(md: string): { props: Prop[]; body: string } {
  const m = FM.exec(md);
  if (!m) return { props: [], body: md };

  const props: Prop[] = [];
  const lines = m[1].split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    if (!lines[i].trim()) continue;
    const km = /^([^:]+):\s?(.*)$/.exec(lines[i]);
    if (!km) continue; // ligne mal formée ou item de liste déjà consommé
    const key = km[1].trim();
    const rest = km[2].trim();

    if (rest === "") {
      // liste en bloc : lignes « - item » qui suivent
      const items: string[] = [];
      while (i + 1 < lines.length && /^\s*-\s+/.test(lines[i + 1])) {
        items.push(unquote(lines[++i].replace(/^\s*-\s+/, "").trim()));
      }
      props.push({ key, value: items.length ? items : "" });
    } else if (rest.startsWith("[") && rest.endsWith("]")) {
      // liste inline [a, b, c]
      const items = rest.slice(1, -1).split(",").map((s) => unquote(s.trim())).filter(Boolean);
      props.push({ key, value: items });
    } else {
      props.push({ key, value: unquote(rest) });
    }
  }
  return { props, body: md.slice(m[0].length).replace(/^\r?\n+/, "") };
}

/** Recompose frontmatter + corps. Sans propriété → corps nu (pas de bloc `---`). */
export function serializeFrontmatter(props: Prop[], body: string): string {
  const clean = props.filter((p) => p.key.trim());
  if (clean.length === 0) return body;
  const lines = clean.map((p) =>
    Array.isArray(p.value) ? `${p.key}: [${p.value.join(", ")}]` : `${p.key}: ${p.value}`,
  );
  return `---\n${lines.join("\n")}\n---\n\n${body.replace(/^\n+/, "")}`;
}
