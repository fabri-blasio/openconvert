/**
 * The contact form.
 *
 * It posts to `/api/contact` on this same origin, which is the only server
 * route the site has. That route holds the Resend key and makes the call —
 * because a mail-service key shipped to the browser is a key anyone can send
 * mail with, and this site is not going to be the one that does that.
 *
 * So the visitor's browser opens exactly one connection and it is to the site
 * they are already on. `connect-src 'self'` on this page, nothing third-party
 * in the network panel, and the address is still printed as a plain link for
 * anyone who would rather write directly.
 *
 * Without JavaScript the form still submits natively to the same endpoint,
 * which is why the markup keeps a real `action` and `method`.
 */
(function () {
  'use strict';

  var form = document.querySelector('[data-contact-form]');
  if (!form) return;

  var status = form.querySelector('[data-status]');
  var button = form.querySelector('button[type="submit"]');
  var label = button ? button.textContent : '';

  function say(msg, tone) {
    if (!status) return;
    status.hidden = false;
    status.textContent = msg;
    status.setAttribute('data-tone', tone);
  }

  form.addEventListener('submit', function (event) {
    event.preventDefault();
    if (button && button.disabled) return;

    if (button) {
      button.disabled = true;
      button.textContent = 'Sending…';
    }
    say('', 'working');

    fetch(form.getAttribute('action'), {
      method: 'POST',
      body: new FormData(form),
      headers: { accept: 'application/json' },
    })
      .then(function (res) {
        return res.json().then(function (body) {
          return { ok: res.ok && body && body.ok, body: body || {} };
        });
      })
      .then(function (r) {
        if (!r.ok) throw new Error(r.body.error || 'That did not send.');
        form.reset();
        say('Sent. We answer within two business days.', 'ok');
      })
      .catch(function (e) {
        say(e.message || 'That did not send. Write to the address on the right.', 'bad');
      })
      .then(function () {
        if (button) {
          button.disabled = false;
          button.textContent = label;
        }
      });
  });
})();
