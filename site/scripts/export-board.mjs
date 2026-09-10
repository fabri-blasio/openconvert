/**
 * The community feature board, generated from GitHub Discussions.
 *
 *     GITHUB_TOKEN=… npm run export:board
 *
 * 13-SITE-REBUILD §3.3. Ideas and votes live in a GitHub Discussions category,
 * where 👍 and 👎 already are upvote and downvote, and identity, moderation and
 * spam control are already somebody else's solved problem. This script reads
 * them at build time and writes `src/data/board.json`.
 *
 * The point of doing it this way: `/community` ends up a static page with no
 * JavaScript and no runtime request, so `connect-src 'none'` stays true on
 * every page of this site — which is a claim we sell, and one that an in-page
 * vote button would cost us.
 *
 * The tier is a label on the discussion. A discussion with no tier label lands
 * in `considering`, which is the honest default: it is open, and nobody has
 * committed to it.
 */

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = join(HERE, '..', 'src', 'data', 'board.json');

const REPO = process.env.BOARD_REPO ?? 'openconvert/openconvert';
const CATEGORY = process.env.BOARD_CATEGORY ?? 'Ideas';
const TOKEN = process.env.GITHUB_TOKEN;
const [owner, name] = REPO.split('/');

/** Label → tier. The order here is the order the page renders. */
const TIERS = [
  { id: 'shipped', label: 'Shipped', blurb: 'In a release you can download.' },
  { id: 'building', label: 'Building', blurb: 'Someone is on it now.' },
  { id: 'next', label: 'Next up', blurb: 'Accepted, not started.' },
  { id: 'considering', label: 'Under consideration', blurb: 'Open. Most ideas live here.' },
  { id: 'declined', label: 'Not planned', blurb: 'With a reason, always.' },
];
const LABEL_TO_TIER = {
  shipped: 'shipped',
  building: 'building',
  'in progress': 'building',
  'next up': 'next',
  accepted: 'next',
  'not planned': 'declined',
  declined: 'declined',
  wontfix: 'declined',
};

const QUERY = `
  query($owner: String!, $name: String!, $cursor: String) {
    repository(owner: $owner, name: $name) {
      discussions(first: 100, after: $cursor, orderBy: { field: UPDATED_AT, direction: DESC }) {
        pageInfo { hasNextPage endCursor }
        nodes {
          number
          title
          url
          bodyText
          category { name }
          labels(first: 10) { nodes { name } }
          upvote: reactions(content: THUMBS_UP) { totalCount }
          downvote: reactions(content: THUMBS_DOWN) { totalCount }
        }
      }
    }
  }
`;

async function fetchAll() {
  const nodes = [];
  let cursor = null;
  for (;;) {
    const res = await fetch('https://api.github.com/graphql', {
      method: 'POST',
      headers: {
        Authorization: `bearer ${TOKEN}`,
        'Content-Type': 'application/json',
        'User-Agent': 'openconvert-site-board-export',
      },
      body: JSON.stringify({ query: QUERY, variables: { owner, name, cursor } }),
    });
    if (!res.ok) throw new Error(`GitHub answered ${res.status} ${res.statusText}`);
    const body = await res.json();
    if (body.errors) throw new Error(body.errors.map((e) => e.message).join('; '));

    const page = body.data.repository.discussions;
    nodes.push(...page.nodes);
    if (!page.pageInfo.hasNextPage) return nodes;
    cursor = page.pageInfo.endCursor;
  }
}

/** First sentence of the body, capped. The board is a list, not a reader. */
function summarise(text) {
  const first = (text ?? '').trim().split(/(?<=[.!?])\s/)[0] ?? '';
  return first.length > 180 ? `${first.slice(0, 177).trimEnd()}…` : first;
}

function tierOf(labels) {
  for (const l of labels) {
    const hit = LABEL_TO_TIER[l.toLowerCase()];
    if (hit) return hit;
  }
  return 'considering';
}

if (!TOKEN) {
  console.error(
    'export-board needs GITHUB_TOKEN (a token with public repo read is enough).\n' +
      'Without it there is nothing to write: an empty board that claims to be the ' +
      'real one is worse than yesterday\'s board, so this refuses rather than guessing.',
  );
  process.exit(1);
}

const discussions = (await fetchAll()).filter((d) => d.category?.name === CATEGORY);

const ideas = discussions
  .map((d) => {
    const labels = d.labels.nodes.map((l) => l.name);
    return {
      number: d.number,
      title: d.title,
      summary: summarise(d.bodyText),
      tier: tierOf(labels),
      up: d.upvote.totalCount,
      down: d.downvote.totalCount,
      url: d.url,
    };
  })
  .sort((a, b) => b.up - b.down - (a.up - a.down) || a.title.localeCompare(b.title));

const previous = JSON.parse(readFileSync(OUT, 'utf8'));

writeFileSync(
  OUT,
  `${JSON.stringify(
    {
      ...previous,
      generated_by: 'site/scripts/export-board.mjs',
      source: `https://github.com/${REPO}/discussions/categories/${CATEGORY.toLowerCase()}`,
      new_url: `https://github.com/${REPO}/discussions/new?category=${CATEGORY.toLowerCase()}`,
      built: new Date().toISOString().slice(0, 10),
      tiers: TIERS,
      ideas,
    },
    null,
    2,
  )}\n`,
);

const counts = TIERS.map((t) => `${t.label} ${ideas.filter((i) => i.tier === t.id).length}`);
console.log(`board.json — ${ideas.length} ideas · ${counts.join(' · ')}`);

const noReason = ideas.filter((i) => i.tier === 'declined' && !i.summary);
if (noReason.length) {
  console.warn(
    `these are marked "Not planned" with no reason in the body, and the page will say so: ` +
      noReason.map((i) => `#${i.number}`).join(', '),
  );
}
