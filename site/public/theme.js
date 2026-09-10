/**
 * Light, dark, or whatever the system says — the site's copy of the app's own
 * three-state theme control (`apps/desktop/src/lib/theme.ts`).
 *
 * **This is the only script on every page, and it is loaded blocking in the
 * head on purpose.** A deferred theme script paints the wrong palette first and
 * corrects it after layout, which is a white flash on every navigation for
 * anyone browsing dark. Six hundred bytes read synchronously is the cheaper
 * trade, and it is the reason `<html>` already carries the right attribute
 * before the first paint.
 *
 * `data-theme` is always `light` or `dark` — never absent, never `system`. The
 * preference is resolved here and stamped, so `tokens.css` carries exactly one
 * dark palette instead of one for the media query and one for the toggle.
 * That is the same decision, and the same reasoning, as the desktop app.
 *
 * localStorage holds one enum and nothing else. It is not a cookie, it is
 * never read by a server, and nothing about a visitor or a file goes into it.
 */
(function () {
  'use strict';

  var KEY = 'openconvert.theme';
  var ORDER = ['system', 'light', 'dark'];

  function stored() {
    try {
      var v = localStorage.getItem(KEY);
      return v === 'light' || v === 'dark' ? v : 'system';
    } catch (e) {
      // Private mode, blocked storage. A theme is not worth failing over.
      return 'system';
    }
  }

  function effective(pref) {
    if (pref !== 'system') return pref;
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches
      ? 'dark'
      : 'light';
  }

  var current = stored();

  function apply(pref) {
    current = pref;
    document.documentElement.setAttribute('data-theme', effective(pref));
    document.documentElement.setAttribute('data-theme-pref', pref);
  }

  apply(current);

  // Only "system" follows the OS, and it has to follow it live: a page left
  // open across sunset should not need reloading.
  if (window.matchMedia) {
    var q = window.matchMedia('(prefers-color-scheme: dark)');
    var onChange = function () {
      if (current === 'system') apply('system');
    };
    if (q.addEventListener) q.addEventListener('change', onChange);
    else if (q.addListener) q.addListener(onChange);
  }

  // The button exists in the markup already; this only gives it its behaviour,
  // so the header is the same size and shape before this file arrives.
  document.addEventListener('click', function (event) {
    var hit = event.target.closest && event.target.closest('[data-theme-toggle]');
    if (!hit) return;
    var next = ORDER[(ORDER.indexOf(current) + 1) % ORDER.length];
    try {
      if (next === 'system') localStorage.removeItem(KEY);
      else localStorage.setItem(KEY, next);
    } catch (e) {
      // Not storable. The choice still applies to this page.
    }
    apply(next);
    hit.setAttribute('aria-label', 'Theme: ' + next + '. Click to change.');
  });
})();
