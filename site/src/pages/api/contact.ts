/**
 * The contact form's one server route.
 *
 * **This is the only code on the site that runs anywhere but the visitor's
 * machine, and it exists for one reason:** Resend authenticates with a secret,
 * and a secret shipped to the browser is a secret every visitor can send mail
 * with. So the key stays here, the browser posts to this origin, and the call
 * to Resend is made from the server.
 *
 * That also keeps the page's own promise intact. The visitor's browser opens
 * exactly one connection, to the site it is already on: `connect-src 'self'`
 * on /contact, no third-party form endpoint in the network panel, and the
 * `zero external requests` gate still passes because no page references an
 * external origin.
 *
 * Every other route on the site is still prerendered to a file.
 */
export const prerender = false;

import type { APIRoute } from 'astro';

/** Where the mail goes. */
const TO = import.meta.env.CONTACT_TO || 'blasiofabrizio23@gmail.com';
/**
 * Who it comes from. Resend requires a verified domain here; until
 * openconvert.dev is verified in the Resend dashboard this must stay
 * `onboarding@resend.dev`, which Resend allows and which can only deliver to
 * the account owner's own address.
 */
const FROM = import.meta.env.CONTACT_FROM || 'OpenConvert <onboarding@resend.dev>';

const TOPICS = new Set([
  'A question about OpenConvert',
  'Something converted wrong',
  'Enterprise or integration',
  'Press',
  'Something else',
]);

const json = (status: number, body: Record<string, unknown>) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
  });

/** Trim, cap, and drop the control characters that let a value forge headers. */
const clean = (v: FormDataEntryValue | null, max: number) =>
  String(v ?? '')
    // Control characters, named by codepoint rather than typed literally: a
    // newline or a NUL smuggled into `subject` or `reply_to` is how a form
    // field forges a mail header.
    .replace(/[\u0000-\u001F\u007F]/g, ' ')
    .trim()
    .slice(0, max);

const escapeHtml = (s: string) =>
  s.replace(/[&<>"']/g, (c) =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c] as string,
  );

export const POST: APIRoute = async ({ request }) => {
  const key = import.meta.env.RESEND_API_KEY;
  if (!key) {
    // Said plainly rather than as a 500. An unconfigured deployment is an
    // operator mistake, and the visitor needs the address, not a stack trace.
    return json(503, {
      ok: false,
      error: `Mail is not configured on this deployment. Write to ${TO} directly.`,
    });
  }

  let form: FormData;
  try {
    form = await request.formData();
  } catch {
    return json(400, { ok: false, error: 'That form could not be read.' });
  }

  // A field no person can see and no person fills in. A bot that fills every
  // input gives itself away here, and nothing is sent.
  if (clean(form.get('website'), 200)) return json(200, { ok: true });

  const topicRaw = clean(form.get('topic'), 120);
  const topic = TOPICS.has(topicRaw) ? topicRaw : 'Something else';
  const name = clean(form.get('name'), 120);
  const org = clean(form.get('org'), 160);
  const replyTo = clean(form.get('email'), 200);
  const message = clean(form.get('message'), 8000);

  if (!message) return json(400, { ok: false, error: 'The message was empty.' });
  if (replyTo && !/^[^@\s]+@[^@\s.]+\.[^@\s]+$/.test(replyTo)) {
    return json(400, { ok: false, error: 'That email address does not look right.' });
  }

  const lines = [
    message,
    '',
    '—',
    name && `From: ${name}`,
    org && `Company: ${org}`,
    replyTo && `Reply to: ${replyTo}`,
  ].filter(Boolean) as string[];

  // The REST API directly, rather than the `resend` SDK: it is one POST, and a
  // dependency that exists to build one POST is a dependency to audit for
  // nothing.
  let res: Response;
  try {
    res = await fetch('https://api.resend.com/emails', {
      method: 'POST',
      headers: { authorization: `Bearer ${key}`, 'content-type': 'application/json' },
      body: JSON.stringify({
        from: FROM,
        to: [TO],
        subject: `${topic}${name ? ` — ${name}` : ''}`,
        text: lines.join('\n'),
        html: `<pre style="font:14px ui-monospace,monospace;white-space:pre-wrap">${escapeHtml(
          lines.join('\n'),
        )}</pre>`,
        // So hitting reply in the inbox answers the person who wrote in.
        ...(replyTo ? { reply_to: replyTo } : {}),
      }),
    });
  } catch {
    return json(502, { ok: false, error: `Could not reach the mail service. Write to ${TO} directly.` });
  }

  if (!res.ok) {
    // The upstream body can carry the key back in an error echo, so it is
    // logged server-side and never returned to the browser.
    console.error('resend rejected the message', res.status, await res.text().catch(() => ''));
    return json(502, { ok: false, error: `That did not send. Write to ${TO} directly.` });
  }

  return json(200, { ok: true });
};
