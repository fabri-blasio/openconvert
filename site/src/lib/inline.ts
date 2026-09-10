/* ============================================================================
   Inline markdown → HTML, for the fragments lifted verbatim out of the design
   record. Deliberately tiny: bold, code and links, and nothing else.

   It escapes first and marks up second, so a `<script>` typed into the threat
   model renders as the text `<script>` on the page. The source is a file in our
   own repository rather than user input, but a renderer that is only safe
   because of where its input comes from stops being safe the day someone points
   it somewhere else.
   ========================================================================== */

const escapeHtml = (s: string): string =>
  s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');

/** Bold and links, applied to text that is already escaped and outside code. */
function marksOnly(s: string): string {
  return s
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_m, text: string, href: string) =>
      // Links into the design record (03-ARCHITECTURE.md#…, the private validation record)
      // have no page on this site, so they keep their label and lose their href.
      // A dead link on the security page is worse than no link.
      /^https?:\/\//.test(href)
        ? `<a href="${href}" rel="noopener">${text}</a>`
        : `<span class="ref">${text}</span>`,
    )
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
}

/**
 * Render `**bold**`, `` `code` `` and `[text](href)`.
 *
 * Split on backticks rather than substituting placeholders: an odd number of
 * segments means the spans alternate text/code/text, and nothing inside a code
 * span is ever scanned for the other two patterns.
 */
export function inlineMd(src: string): string {
  return escapeHtml(src)
    .split('`')
    .map((seg, i) => (i % 2 === 1 ? `<code>${seg}</code>` : marksOnly(seg)))
    .join('');
}
