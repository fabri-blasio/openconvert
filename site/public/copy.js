/**
 * Copy-to-clipboard for the install commands on /download.
 *
 * The whole content of that page is lines you are meant to paste into a
 * terminal, so a copy button is the page's primary action rather than a
 * flourish. It is the second script on this site and it is deliberately the
 * whole of it: no framework, no bundler, no dependency, one listener.
 *
 * The button is rendered by the page, not created here, so the layout is the
 * same before this file arrives and there is nothing to reflow. If the script
 * never loads the button is inert — which is why it also carries a `title`
 * naming the command, and why the command itself stays selectable text.
 */
(function () {
  'use strict';

  var DONE_MS = 1400;

  function label(button, state) {
    button.setAttribute('data-state', state);
    var live = button.querySelector('[data-label]');
    if (live) live.textContent = state === 'done' ? 'Copied' : state === 'failed' ? 'Press Ctrl+C' : 'Copy';
  }

  function copy(button) {
    var target = document.getElementById(button.getAttribute('data-copy'));
    if (!target) return;
    var text = (target.textContent || '').trim();

    // `navigator.clipboard` needs a secure context. On plain http — a local
    // preview, an intranet mirror — it is simply absent, so the fallback
    // selects the text and tells the reader to use their own shortcut rather
    // than failing silently.
    if (!navigator.clipboard || !navigator.clipboard.writeText) {
      var range = document.createRange();
      range.selectNodeContents(target);
      var sel = window.getSelection();
      sel.removeAllRanges();
      sel.addRange(range);
      label(button, 'failed');
      setTimeout(function () { label(button, 'idle'); }, DONE_MS * 2);
      return;
    }

    navigator.clipboard.writeText(text).then(
      function () {
        label(button, 'done');
        setTimeout(function () { label(button, 'idle'); }, DONE_MS);
      },
      function () {
        label(button, 'failed');
        setTimeout(function () { label(button, 'idle'); }, DONE_MS * 2);
      }
    );
  }

  document.addEventListener('click', function (event) {
    var button = event.target.closest ? event.target.closest('[data-copy]') : null;
    if (button) copy(button);
  });
})();
