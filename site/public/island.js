/* ============================================================================
   The conversion island — the ONE script openconvert.dev ships on load.

   Three jobs, all on the homepage:

   1. THE CONVERTER. Identify a dropped file by its first bytes through
      `openconvert-core` compiled to wasm32, print the real plan, and finish
      the conversions a tab can genuinely finish: image to image, image to
      PDF, and CSV to JSON either way. Everything else is offered and refused
      by name, with the CLI line that does run.

   2. THE TOOLS. The eleven tools a tab can perform — invert, black and white,
      compress, colour picker, and seven PDF page edits through `pdf-lib` —
      actually run, on a file you choose, in this tab. The other nine are
      dimmed in the menu with the reason.

   3. THE AMBIENT LAYER. Pauses the hero pills when they scroll out of view.

   The honesty rules this file obeys:

     * WHAT IS OFFERED IS WHAT CAN BE DONE. Encoder support is MEASURED, not
       assumed: `canvas.toBlob` does not throw for a format it cannot write —
       it silently returns a PNG. So every encoder is probed by encoding and
       reading the blob's own `type` back, and a target that fails that test
       is disabled rather than producing a mislabelled file.
     * Decode support is not guessed either. The file you gave us is decoded
       first; the target list is built from what that decode actually allowed.
     * `data-capable` on each menu entry is rendered by AppDemo.astro from one
       list on the server. This file READS that attribute; it does not keep a
       second copy that could disagree.
     * The receipt names `browser canvas` or `pdf-lib` because that is the
       engine. The desktop binary runs the same route through sandboxed native
       engines, and the sample says so rather than impersonating it.
     * `pdf-lib` is 201 KB gzipped and is fetched ONLY when a PDF tool is
       opened. Nobody downloads it for reading the page.
     * No framework, no bundler output, CSP `connect-src 'self'` — every byte
       stays on this machine.
   ========================================================================== */
(function () {
  'use strict';

  /* ---- ambient layer: pause offscreen --------------------------------- */
  var ambient = document.querySelector('[data-ambient]');
  if (ambient && 'IntersectionObserver' in window) {
    new IntersectionObserver(function (entries) {
      ambient.toggleAttribute('data-paused', !entries[0].isIntersecting);
    }, { threshold: 0 }).observe(ambient);
  }

  /* ======================================================================
     CAPABILITY — measured once, lazily, and never assumed.
     ====================================================================== */

  /** Candidate raster targets and the MIME type each would need. */
  var MIME = {
    png: 'image/png',
    jpeg: 'image/jpeg',
    webp: 'image/webp',
    avif: 'image/avif',
    gif: 'image/gif',
    bmp: 'image/bmp',
    tiff: 'image/tiff',
  };

  var encoders = null; // { png: 'image/png', ... } once probed

  /**
   * Which image formats this browser can WRITE.
   *
   * The test is not "did toBlob return something" — it returns a PNG for every
   * type it does not support, with no error anywhere. So the returned blob's
   * own `type` is compared against what was asked for, and only an exact match
   * counts. Measured here rather than listed, because the list differs by
   * browser and by version and a stale list ships mislabelled files.
   */
  function probeEncoders() {
    if (encoders) return Promise.resolve(encoders);
    var c = document.createElement('canvas');
    c.width = 8;
    c.height = 8;
    var g = c.getContext('2d');
    g.fillStyle = '#888';
    g.fillRect(0, 0, 8, 8);

    var names = Object.keys(MIME);
    return Promise.all(
      names.map(function (name) {
        return new Promise(function (res) {
          try {
            c.toBlob(function (b) { res(b && b.type === MIME[name]); }, MIME[name], 0.9);
          } catch (e) {
            res(false);
          }
        });
      })
    ).then(function (ok) {
      encoders = {};
      names.forEach(function (n, i) { if (ok[i]) encoders[n] = MIME[n]; });
      return encoders;
    });
  }

  /** `pdf-lib`, fetched on first use and never before. */
  var pdfLib = null;
  function loadPdfLib() {
    if (window.PDFLib) return Promise.resolve(window.PDFLib);
    if (pdfLib) return pdfLib;
    pdfLib = new Promise(function (res, rej) {
      var s = document.createElement('script');
      s.src = '/vendor/pdf-lib.min.js';
      s.onload = function () {
        window.PDFLib ? res(window.PDFLib) : rej(new Error('pdf-lib did not define itself'));
      };
      s.onerror = function () { rej(new Error('pdf-lib could not be loaded')); };
      document.head.appendChild(s);
    });
    return pdfLib;
  }

  /* ---- small shared helpers ------------------------------------------- */
  function esc(s) {
    return String(s).replace(/[&<>"]/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c];
    });
  }
  var kb = function (n) { return n < 1024 ? n + ' B' : (n / 1024).toFixed(1) + ' KB'; };
  var stem = function (n) { var m = n.match(/^(.*)\.[^.]+$/); return m ? m[1] : n; };

  /** Hand a blob to the visitor. Nothing is uploaded; this is a local save. */
  function offer(blob, name) {
    var url = URL.createObjectURL(blob);
    var a = document.createElement('a');
    a.href = url;
    a.download = name;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(function () { URL.revokeObjectURL(url); }, 4000);
    return url;
  }

  function bitmap(f) { return createImageBitmap(f); }

  function draw(bm) {
    var c = document.createElement('canvas');
    c.width = bm.width;
    c.height = bm.height;
    c.getContext('2d', { willReadFrequently: true }).drawImage(bm, 0, 0);
    return c;
  }

  function encode(canvas, mime, q) {
    return new Promise(function (res, rej) {
      canvas.toBlob(function (b) {
        if (!b) return rej(new Error('encode failed'));
        if (b.type !== mime) return rej(new Error('this browser has no ' + mime + ' encoder'));
        res(b);
      }, mime, q);
    });
  }

  /** CSV and JSON, both directions. Pure string work, no dependency. */
  function splitCsvLine(line) {
    var out = [], cur = '', q = false;
    for (var i = 0; i < line.length; i++) {
      var ch = line[i];
      if (q) {
        if (ch === '"' && line[i + 1] === '"') { cur += '"'; i++; }
        else if (ch === '"') q = false;
        else cur += ch;
      } else if (ch === '"') q = true;
      else if (ch === ',') { out.push(cur); cur = ''; }
      else cur += ch;
    }
    out.push(cur);
    return out;
  }
  function csvToJson(text) {
    var lines = text.replace(/\r\n/g, '\n').split('\n').filter(function (l) { return l.length; });
    if (!lines.length) return '[]';
    var head = splitCsvLine(lines[0]);
    var rows = lines.slice(1).map(function (l) {
      var cells = splitCsvLine(l), o = {};
      head.forEach(function (h, i) { o[h] = cells[i] === undefined ? '' : cells[i]; });
      return o;
    });
    return JSON.stringify(rows, null, 2);
  }
  function jsonToCsv(text) {
    var data = JSON.parse(text);
    if (!Array.isArray(data)) data = [data];
    if (!data.length) return '';
    var cols = [];
    data.forEach(function (r) {
      Object.keys(r || {}).forEach(function (k) { if (cols.indexOf(k) < 0) cols.push(k); });
    });
    var cell = function (v) {
      if (v === null || v === undefined) return '';
      var s = typeof v === 'object' ? JSON.stringify(v) : String(v);
      return /[",\n]/.test(s) ? '"' + s.replace(/"/g, '""') + '"' : s;
    };
    return [cols.join(',')]
      .concat(data.map(function (r) { return cols.map(function (c) { return cell(r && r[c]); }).join(','); }))
      .join('\n');
  }

  /** An image, wrapped in a PDF page of its own size. */
  function imageToPdf(f) {
    return Promise.all([loadPdfLib(), bitmap(f)]).then(function (r) {
      var L = r[0], bm = r[1];
      var c = draw(bm);
      bm.close();
      // PNG because pdf-lib embeds PNG and JPEG only, and PNG is the lossless
      // one — re-encoding through a lossy format to reach a container would
      // lose pixels for no reason.
      return encode(c, 'image/png').then(function (png) {
        return png.arrayBuffer();
      }).then(function (buf) {
        return L.PDFDocument.create().then(function (doc) {
          return doc.embedPng(buf).then(function (img) {
            var page = doc.addPage([img.width, img.height]);
            page.drawImage(img, { x: 0, y: 0, width: img.width, height: img.height });
            return doc.save();
          });
        });
      }).then(function (bytes) {
        return new Blob([bytes], { type: 'application/pdf' });
      });
    });
  }

  /* ======================================================================
     THE CONVERTER
     ====================================================================== */

  var root = document.getElementById('convert-island');
  var W = null;
  var loading = null;
  var head = null;
  var current = null;
  var file = null;
  var detected = '';
  var fileName = '';
  var decoded = false; // did THIS file decode as an image in THIS browser

  var els = root
    ? {
        zone: root.querySelector('[data-zone]'),
        input: root.querySelector('[data-file]'),
        idle: root.querySelector('[data-idle]'),
        out: root.querySelector('[data-out]'),
        status: root.querySelector('[data-status]'),
        detected: root.querySelector('[data-detected]'),
        targets: root.querySelector('[data-targets]'),
        plan: root.querySelector('[data-plan]'),
        action: root.querySelector('[data-action]'),
        result: root.querySelector('[data-result]'),
      }
    : null;

  function say(msg) { if (els && els.status) els.status.textContent = msg; }

  function load() {
    if (W) return Promise.resolve(W);
    if (loading) return loading;
    say('loading the router…');
    loading = WebAssembly.instantiateStreaming(fetch('/openconvert.wasm'), {})
      .catch(function () {
        return fetch('/openconvert.wasm')
          .then(function (r) { return r.arrayBuffer(); })
          .then(function (b) { return WebAssembly.instantiate(b, {}); });
      })
      .then(function (m) { W = m.instance.exports; return W; });
    return loading;
  }

  function ask(bytes, target) {
    var ptr = W.alloc(bytes.length);
    new Uint8Array(W.memory.buffer).set(bytes, ptr);
    var out = W.plan(ptr, bytes.length, target === undefined ? 0xffffffff : target);
    var len = W.last_len();
    var json = new TextDecoder().decode(
      new Uint8Array(W.memory.buffer).subarray(out, out + len)
    );
    W.dealloc(ptr, bytes.length);
    return JSON.parse(json);
  }

  var GLYPH = { A: '=', B: '≈', C: '⌇', D: '✦' };
  var WORD = { A: 'lossless', B: 'lossy', C: 'inferred', D: 'generated' };

  function cliLine(to) { return 'openconvert convert "' + fileName + '" -t ' + to; }

  function refuse(title, why, cmd) {
    els.action.innerHTML =
      '<div class="il-refuse">' +
      '<span class="il-refuse-title">' + esc(title) + '</span>' +
      '<span class="caption">' + why + '</span>' +
      (cmd ? '<code class="il-cmd mono">' + esc(cmd) + '</code>' : '') +
      '</div>';
  }

  /**
   * Can this tab actually produce that target from the file in hand?
   *
   * Returns null when it can, or the reason when it cannot. The reason is
   * shown on the disabled button, because a control that is greyed out with no
   * explanation is indistinguishable from one that is broken.
   */
  function whyNot(target) {
    if (detected === 'csv' && target === 'json') return null;
    if (detected === 'json' && target === 'csv') return null;
    if (!decoded) {
      return 'reading ' + detected + ' needs a native engine in a sandboxed worker';
    }
    if (target === 'pdf') return null; // image -> pdf, via pdf-lib
    if (encoders && encoders[target]) return null;
    if (MIME[target]) return 'this browser has no ' + target.toUpperCase() + ' encoder';
    return 'writing ' + target + ' needs a native engine';
  }

  /**
   * Targets the BROWSER can reach that the wasm router does not list.
   *
   * The router is deliberately conservative: it answers for an environment
   * with no engines available, so a format whose decoder lives in `oc-images`
   * comes back with an empty target list. AVIF is the plain case — the route
   * table really does have `avif -> png`, and the CLI runs it, but the router
   * cannot promise it in-process so it offers nothing.
   *
   * This tab, meanwhile, decodes AVIF natively. Refusing a conversion the
   * browser can genuinely perform, because a router built for a different
   * environment declined to plan it, would understate the product on the one
   * page meant to demonstrate it.
   *
   * These are marked apart from the router's own answers and carry NO fidelity
   * class, because the class is the router's to state and this is not the
   * router talking.
   */
  function browserExtras(routerTargets) {
    if (!decoded || !encoders) return [];
    var have = {};
    routerTargets.forEach(function (t) { have[t.id] = 1; });
    var out = [];
    ['png', 'jpeg', 'webp'].forEach(function (n) {
      if (encoders[n] && !have[n] && n !== detected) out.push(n);
    });
    if (!have.pdf && detected !== 'pdf') out.push('pdf');
    return out;
  }

  /**
   * A plan this file wrote, clearly labelled as such.
   *
   * The class is the ENCODE's own nature, stated per target rather than in one
   * blanket: writing PNG from decoded pixels loses nothing, and calling it
   * lossy — as a single hardcoded label did — is exactly the kind of small
   * untruth this product exists to avoid. It does not claim the input was
   * lossless; it says what this step does to what it was handed.
   */
  var BROWSER_CLASS = { png: 'A', pdf: 'A', jpeg: 'B', webp: 'B' };

  function browserPlan(target) {
    var cls = BROWSER_CLASS[target] || 'B';
    els.plan.innerHTML =
      '<p class="il-verdict">planned by this browser · decode ' + esc(detected) +
      ' → encode ' + esc(target) + '</p>' +
      '<ol class="il-steps"><li class="il-step">' +
      '<span class="il-kind mono">' + (target === 'pdf' ? 'Wrap { in: Pdf }' : 'Transcode { to: ' + esc(target) + ' }') + '</span>' +
      '<span class="il-cls mono">' + GLYPH[cls] + ' ' + cls + ' ' + WORD[cls] + '</span>' +
      '<span class="il-iso">this tab</span>' +
      '<span class="caption il-lim mono">' + (target === 'pdf' ? 'pdf-lib' : 'browser codecs') + '</span>' +
      '</li></ol>' +
      '<p class="caption">The router plans for the desktop environment and does not offer this pair ' +
      'in-process. The binary runs it through a sandboxed engine; this tab uses its own codec.</p>';
    els.action.innerHTML =
      '<button type="button" class="btn btn-primary il-go" data-go>Convert here</button>' +
      '<span class="caption">in this tab, with the browser’s own codecs — then download</span>';
    current = { target: target, cls: cls };
    [].forEach.call(els.targets.querySelectorAll('.il-t'), function (b) {
      b.setAttribute('aria-pressed', String(b.getAttribute('data-b') === target));
    });
  }

  function show(r) {
    if (els.idle) els.idle.hidden = true;
    if (els.out) els.out.hidden = false;
    els.result.hidden = true;
    detected = String(r.detected);

    var det = esc(r.detected);
    els.detected.innerHTML =
      '<span class="il-file mono">' + esc(fileName) + '</span>' +
      '<span class="il-arrow" aria-hidden="true">→</span>' +
      '<span class="il-det mono">' + det + '</span>' +
      (r.polyglot
        ? '<span class="il-poly">polyglot — refused</span>'
        : '<span class="il-by">by content, from ' + r.head_bytes + ' bytes</span>');

    var extras = browserExtras(r.targets);

    if (!r.targets.length && !extras.length) {
      els.targets.innerHTML = '';
      els.plan.innerHTML = '';
      els.action.innerHTML =
        '<div class="il-refuse">' +
        '<span class="il-refuse-title">No route from ' + det + '.</span>' +
        '<span class="caption">Nothing pretends otherwise. The engines that read this format are ' +
        'C libraries running in sandboxed worker processes — <a href="/download">the binary has them</a>.</span>' +
        '</div>';
      say('');
      return;
    }

    /* EVERY ROUTE IS LISTED; the ones this tab cannot finish are DISABLED
       rather than hidden. Hiding them would misrepresent the product, which
       has all of them; leaving them live would produce a file that is not what
       its name says. */
    els.targets.innerHTML =
      r.targets
        .map(function (t) {
          var why = whyNot(t.id);
          return (
            '<button type="button" class="il-t mono" data-i="' + t.i + '"' +
            (why ? ' disabled title="In this preview: ' + esc(why) + '"' : '') + '>' +
            '<span class="glyph" aria-hidden="true">' + (GLYPH[t.class] || '') + '</span> ' +
            esc(t.id) + '</button>'
          );
        })
        .join('') +
      extras
        .map(function (id) {
          return (
            '<button type="button" class="il-t il-t-browser mono" data-b="' + esc(id) + '" ' +
            'title="This browser can do it; the router plans only what the desktop environment runs in-process.">' +
            '<span class="glyph" aria-hidden="true">·</span> ' + esc(id) + '</button>'
          );
        })
        .join('');

    // Open on the first target this tab can actually do, not blindly the first.
    var first = r.targets.filter(function (t) { return !whyNot(t.id); })[0];
    if (first) plan(first.i);
    else if (extras.length) browserPlan(extras[0]);
    else plan(r.targets[0].i);
    say('');
  }

  function plan(i) {
    var r = ask(head, i);
    var p = r.plan;
    [].forEach.call(els.targets.querySelectorAll('.il-t'), function (b) {
      b.setAttribute('aria-pressed', b.getAttribute('data-i') === String(i));
    });

    /* THE TARGET, TAKEN FROM THE PLAN — not by indexing the target list. `i`
       is the router's FORMAT index, not a position in `r.targets`. */
    var chosen = null;
    if (r.targets) {
      for (var k = 0; k < r.targets.length; k++) {
        if (r.targets[k].i === i) chosen = r.targets[k];
      }
    }
    var targetId = String((p && p.target) || (chosen && chosen.id) || '');
    var why = whyNot(targetId);

    if (!p || !p.executable) {
      els.plan.innerHTML = p
        ? '<p class="il-verdict">refused — no route to ' + esc(p.target) + '</p>'
        : '';
      refuse(
        'Runs in the desktop app.',
        'This pair needs engines a tab does not have. The route table above still applies to the binary.',
        cliLine(targetId)
      );
      current = null;
      return;
    }

    els.plan.innerHTML =
      '<p class="il-verdict">' +
      'class ' + p.class + ' · ' + WORD[p.class] + ' · ' +
      p.steps.length + ' step' + (p.steps.length === 1 ? '' : 's') +
      '</p><ol class="il-steps">' +
      p.steps.map(function (s) {
        return (
          '<li class="il-step">' +
          '<span class="il-kind mono">' + esc(s.kind) + '</span>' +
          '<span class="il-cls mono">' + GLYPH[s.class] + ' ' + s.class + ' ' + WORD[s.class] + '</span>' +
          '<span class="il-iso">' + esc(s.isolation) + '</span>' +
          '<span class="caption il-lim mono">' +
          Math.round(s.limits.memory_bytes / 1048576) + ' MiB · ' +
          s.limits.wall_time_secs + 's wall</span>' +
          '</li>'
        );
      }).join('') + '</ol>';

    if (!why) {
      els.action.innerHTML =
        '<button type="button" class="btn btn-primary il-go" data-go>Convert here</button>' +
        '<span class="caption">in this tab, with the browser’s own codecs — then download</span>';
      current = { target: targetId, cls: p.class };
    } else {
      refuse('Runs in the desktop app.', esc(why) + '. The route above is real; this tab is not where it runs.', cliLine(targetId));
      current = null;
    }
  }

  function outName() {
    return stem(fileName) + '.' + (current.target === 'jpeg' ? 'jpg' : current.target);
  }

  function receipt(outBlob, ms, engine) {
    return JSON.stringify(
      {
        input: { name: fileName, size: file.size },
        output: { name: outName(), size: outBlob.size },
        steps: [
          {
            engine: engine,
            op: 'transcode ' + String(detected) + ' → ' + current.target,
            class: current.cls,
          },
        ],
        sandbox: 'this tab',
        network_calls: 0,
        duration_ms: Math.round(ms),
        note: 'the desktop binary runs this route through sandboxed native engines and writes a fuller receipt',
      },
      null,
      2
    );
  }

  function convert() {
    if (!file || !current) return;
    var t0 = performance.now();
    var target = current.target;
    var engine = 'browser canvas';
    var job;

    if (target === 'pdf') {
      engine = 'pdf-lib, in this tab';
      say('fetching pdf-lib…');
      job = imageToPdf(file);
    } else if (target === 'json' || target === 'csv') {
      engine = 'this tab, no dependency';
      job = file.text().then(function (text) {
        var out = target === 'json' ? csvToJson(text) : jsonToCsv(text);
        return new Blob([out], { type: target === 'json' ? 'application/json' : 'text/csv' });
      });
    } else {
      job = bitmap(file).then(function (bm) {
        var c = draw(bm);
        bm.close();
        return encode(c, MIME[target], 0.85);
      });
    }

    job
      .then(function (blob) {
        var ms = performance.now() - t0;
        var name = outName();
        offer(blob, name);
        els.result.hidden = false;
        els.result.innerHTML =
          '<div class="il-done-head">' +
          '<span class="mono">' + esc(name) + '</span>' +
          '<span class="caption">' + kb(file.size) + ' → ' + kb(blob.size) + ' · ' +
          Math.round(ms) + ' ms · 0 network calls</span>' +
          '</div>' +
          '<button type="button" class="il-rbtn mono" data-receipt-toggle aria-expanded="false">receipt ▾</button>' +
          '<pre class="il-receipt mono" data-receipt hidden>' + esc(receipt(blob, ms, engine)) + '</pre>';
        say('');
      })
      .catch(function (e) { say('this tab could not finish it: ' + e.message); });
  }

  function take(f) {
    if (!f) return;
    file = f;
    fileName = f.name;
    decoded = false;
    Promise.all([load(), probeEncoders()])
      .then(function () {
        var n = Math.min(W.head_bytes(), f.size);
        return f.slice(0, n).arrayBuffer();
      })
      .then(function (buf) {
        head = new Uint8Array(buf);
        // DECODE FIRST, then build the target list from what that allowed.
        // Guessing from the format name is how a browser without an AVIF
        // decoder ends up offering AVIF conversions it cannot start.
        return bitmap(f).then(
          function (bm) { decoded = true; bm.close(); },
          function () { decoded = false; }
        );
      })
      .then(function () { show(ask(head)); })
      .catch(function (e) { say('the router did not load: ' + e.message); });
  }

  if (root && 'WebAssembly' in window && els.zone) {
    els.zone.addEventListener('dragover', function (e) {
      e.preventDefault();
      els.zone.setAttribute('data-over', '');
      load();
    });
    els.zone.addEventListener('dragleave', function () { els.zone.removeAttribute('data-over'); });
    els.zone.addEventListener('drop', function (e) {
      e.preventDefault();
      els.zone.removeAttribute('data-over');
      take(e.dataTransfer.files[0]);
    });
    if (els.input) {
      els.input.addEventListener('change', function () { take(els.input.files[0]); });
    }
    els.targets.addEventListener('click', function (e) {
      var b = e.target.closest('.il-t');
      if (!b || b.disabled) return;
      var only = b.getAttribute('data-b');
      if (only) browserPlan(only);
      else plan(Number(b.getAttribute('data-i')));
    });
    els.action.addEventListener('click', function (e) {
      if (e.target.closest('[data-go]')) convert();
    });
    els.result.addEventListener('click', function (e) {
      var t = e.target.closest('[data-receipt-toggle]');
      if (!t) return;
      var pre = els.result.querySelector('[data-receipt]');
      var open = t.getAttribute('aria-expanded') === 'true';
      t.setAttribute('aria-expanded', String(!open));
      t.textContent = open ? 'receipt ▾' : 'receipt ▴';
      pre.hidden = open;
    });
  }

  /* ======================================================================
     THE TOOLS
     ====================================================================== */

  var run = document.querySelector('[data-tool-run]');
  if (run) {
    var rEls = {
      why: run.querySelector('[data-tool-why]'),
      pick: run.querySelector('[data-tool-pick]'),
      fileIn: run.querySelector('[data-tool-file]'),
      pickLabel: run.querySelector('[data-tool-picklabel]'),
      params: run.querySelector('[data-tool-params]'),
      go: run.querySelector('[data-tool-go]'),
      status: run.querySelector('[data-tool-status]'),
      result: run.querySelector('[data-tool-result]'),
    };
    var hint = document.querySelector('[data-tool-hint]');
    var picked = [];

    /**
     * What each tool asks for. `accept` narrows the file picker, `multi` takes
     * more than one file, and `params` are the controls the desktop tool has —
     * the same names, so the preview teaches the real thing.
     */
    var SPEC = {
      'image-invert': { accept: 'image/*' },
      'image-greyscale': { accept: 'image/*' },
      'image-compress': {
        accept: 'image/*',
        /* A FORMAT CHOICE, because a browser cannot honour the desktop tool's
           rule on its own. There, compression keeps the format it was given.
           Here, `canvas.toBlob` ignores the quality argument for PNG entirely
           — there is no lossy PNG — so "compress this PNG" would either do
           nothing or quietly hand back a JPEG under a .png-shaped promise.
           Asking is the honest third option, and the default keeps the source
           format whenever that format has a quality knob at all. */
        params: [
          { id: 'quality', label: 'Quality', type: 'number', min: 1, max: 100, value: 75 },
          { id: 'format', label: 'Write as', type: 'select', options: [], value: '' },
        ],
      },
      'image-pick': { accept: 'image/*', verb: 'Read a colour' },
      'pdf-merge': { accept: 'application/pdf', multi: true, verb: 'Merge' },
      'pdf-split': {
        accept: 'application/pdf',
        params: [{ id: 'every', label: 'Pages per file', type: 'number', min: 1, max: 1000, value: 1 }],
      },
      'pdf-extract': {
        accept: 'application/pdf',
        params: [{ id: 'pages', label: 'Pages to keep', type: 'text', placeholder: '1-3,7', value: '' }],
      },
      'pdf-remove': {
        accept: 'application/pdf',
        params: [{ id: 'pages', label: 'Pages to remove', type: 'text', placeholder: '1-3,7', value: '' }],
      },
      'pdf-reorder': {
        accept: 'application/pdf',
        params: [{ id: 'order', label: 'New order', type: 'text', placeholder: '3,1,2', value: '' }],
      },
      'pdf-rotate': {
        accept: 'application/pdf',
        params: [
          { id: 'turn', label: 'Turn', type: 'select', options: [['90', '90° right'], ['180', '180°'], ['270', '90° left']], value: '90' },
        ],
      },
      'pdf-crop': {
        accept: 'application/pdf',
        params: [{ id: 'margin', label: 'Margin (pt)', type: 'number', min: 0, max: 300, value: 24 }],
      },
    };

    /** "1-3,7" against a page count, zero-based, in the order written. */
    function parsePages(spec, count) {
      var out = [];
      String(spec).split(',').forEach(function (part) {
        part = part.trim();
        if (!part) return;
        var m = part.match(/^(\d+)\s*-\s*(\d*)$/);
        if (m) {
          var a = Number(m[1]), b = m[2] ? Number(m[2]) : count;
          for (var i = a; i <= b; i++) if (i >= 1 && i <= count) out.push(i - 1);
        } else if (/^\d+$/.test(part)) {
          var n = Number(part);
          if (n >= 1 && n <= count) out.push(n - 1);
        }
      });
      return out;
    }

    function activeTool() {
      var r = document.querySelector('.dv-tool:checked');
      if (!r) return null;
      var id = r.id.replace(/^dv-t-/, '');
      var label = document.querySelector('.m-item[for="dv-t-' + id + '"]');
      return {
        id: id,
        capable: label ? label.getAttribute('data-capable') === 'yes' : false,
        why: label ? (label.getAttribute('title') || '') : '',
        name: label ? label.textContent.replace(/desktop\s*$/i, '').trim() : id,
      };
    }

    function paramValue(id) {
      var el = rEls.params.querySelector('[data-p="' + id + '"]');
      return el ? el.value : '';
    }

    /** Fill the compress tool's format list from what this browser can write. */
    function withEncoders(spec) {
      if (!spec.params) return spec;
      var copy = { accept: spec.accept, multi: spec.multi, verb: spec.verb, params: spec.params.map(function (p) {
        if (p.id !== 'format') return p;
        var opts = Object.keys(encoders || {}).map(function (n) { return [n, n.toUpperCase()]; });
        return { id: 'format', label: p.label, type: 'select', options: opts, value: (opts[0] || [''])[0] };
      }) };
      return copy;
    }

    function renderParams(spec) {
      rEls.params.innerHTML = (spec.params || [])
        .map(function (p) {
          if (p.type === 'select') {
            return '<label>' + esc(p.label) + ' <select data-p="' + p.id + '">' +
              p.options.map(function (o) {
                return '<option value="' + esc(o[0]) + '"' + (o[0] === p.value ? ' selected' : '') + '>' + esc(o[1]) + '</option>';
              }).join('') + '</select></label>';
          }
          return '<label>' + esc(p.label) + ' <input data-p="' + p.id + '" type="' + p.type + '"' +
            (p.min !== undefined ? ' min="' + p.min + '"' : '') +
            (p.max !== undefined ? ' max="' + p.max + '"' : '') +
            (p.placeholder ? ' placeholder="' + esc(p.placeholder) + '"' : '') +
            ' value="' + esc(p.value) + '" /></label>';
        })
        .join('');
    }

    /** Re-read the whole runner for whichever tool is now open. */
    function sync() {
      var t = activeTool();
      picked = [];
      rEls.result.hidden = true;
      rEls.result.innerHTML = '';
      rEls.status.textContent = '';
      if (!t) {
        run.hidden = true;
        if (hint) hint.hidden = false;
        return;
      }
      run.hidden = false;
      var spec = SPEC[t.id];
      if (!t.capable || !spec) {
        rEls.why.hidden = false;
        rEls.why.textContent =
          'This one runs in the desktop app — ' +
          (t.why.replace(/^In this preview:\s*/, '') || 'a tab cannot do it') + '.';
        rEls.pick.hidden = true;
        rEls.go.hidden = true;
        rEls.params.innerHTML = '';
        if (hint) hint.hidden = false;
        return;
      }
      rEls.why.hidden = true;
      rEls.pick.hidden = false;
      rEls.go.hidden = false;
      rEls.go.textContent = spec.verb ? spec.verb + ' here' : 'Run here';
      rEls.fileIn.accept = spec.accept || '';
      rEls.fileIn.multiple = !!spec.multi;
      rEls.fileIn.value = '';
      rEls.pickLabel.textContent = spec.multi ? 'Choose files' : 'Choose a file';
      renderParams(withEncoders(spec));
      if (hint) hint.hidden = true;
    }

    function done(msg, nodes) {
      rEls.status.textContent = '';
      rEls.result.hidden = false;
      rEls.result.innerHTML = msg;
      if (nodes) nodes.forEach(function (n) { rEls.result.appendChild(n); });
    }

    /* ---- the image tools: one canvas pass each ------------------------- */
    function pixels(f, fn) {
      return bitmap(f).then(function (bm) {
        var c = draw(bm);
        bm.close();
        var g = c.getContext('2d', { willReadFrequently: true });
        var d = g.getImageData(0, 0, c.width, c.height);
        fn(d.data);
        g.putImageData(d, 0, 0);
        return c;
      });
    }

    function runImage(id) {
      var f = picked[0];
      if (id === 'image-pick') {
        return bitmap(f).then(function (bm) {
          var c = draw(bm);
          bm.close();
          var g = c.getContext('2d', { willReadFrequently: true });
          // The centre pixel: a preview with no pointer has to pick somewhere,
          // and the middle is the one place that needs no explanation.
          var d = g.getImageData((c.width / 2) | 0, (c.height / 2) | 0, 1, 1).data;
          var hex = '#' + [d[0], d[1], d[2]].map(function (n) {
            return ('0' + n.toString(16)).slice(-2);
          }).join('').toUpperCase();
          var sw = document.createElement('span');
          sw.className = 'w-swatch';
          sw.style.background = hex;
          done('<span class="mono">' + hex + '</span><span class="caption">centre pixel, read on this machine</span>', [sw]);
          return null;
        });
      }

      var work;
      if (id === 'image-invert') {
        work = pixels(f, function (d) {
          for (var i = 0; i < d.length; i += 4) { d[i] = 255 - d[i]; d[i + 1] = 255 - d[i + 1]; d[i + 2] = 255 - d[i + 2]; }
        });
      } else if (id === 'image-greyscale') {
        work = pixels(f, function (d) {
          for (var i = 0; i < d.length; i += 4) {
            // Rec. 709 luma, not a flat mean: the eye is far more sensitive to
            // green, and averaging makes a photograph look muddy.
            var y = (0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2]) | 0;
            d[i] = d[i + 1] = d[i + 2] = y;
          }
        });
      } else {
        work = bitmap(f).then(function (bm) { var c = draw(bm); bm.close(); return c; });
      }

      var q = id === 'image-compress' ? Math.max(1, Math.min(100, Number(paramValue('quality')) || 75)) / 100 : 0.92;
      // Compress writes the format you chose; everything else is written as
      // PNG, so a lossy re-encode is never a side effect of a tool that is not
      // about compression.
      var chosen = id === 'image-compress' ? (paramValue('format') || 'png') : 'png';
      var mime = MIME[chosen] || 'image/png';
      var ext = chosen === 'jpeg' ? 'jpg' : chosen;
      return work.then(function (c) { return encode(c, mime, q); }).then(function (blob) {
        var name = stem(f.name) + '-' + id.replace('image-', '') + '.' + ext;
        offer(blob, name);
        var note = '';
        if (id === 'image-compress' && chosen === 'png') {
          // Said rather than hidden: the number in the box did nothing.
          note = ' · PNG has no quality setting in a browser, so this is a re-encode, not a compression';
        }
        done(
          '<span class="mono">' + esc(name) + '</span><span class="caption">' +
          kb(f.size) + ' → ' + kb(blob.size) + ' · 0 network calls' + note + '</span>'
        );
      });
    }

    /* ---- the PDF tools: page-tree edits through pdf-lib ---------------- */
    function runPdf(id) {
      rEls.status.textContent = 'fetching pdf-lib…';
      return loadPdfLib().then(function (L) {
        rEls.status.textContent = 'working…';
        var first = picked[0];

        if (id === 'pdf-merge') {
          if (picked.length < 2) throw new Error('merging takes two files or more');
          return L.PDFDocument.create().then(function (out) {
            var chain = Promise.resolve();
            picked.forEach(function (f) {
              chain = chain
                .then(function () { return f.arrayBuffer(); })
                .then(function (b) { return L.PDFDocument.load(b, { ignoreEncryption: true }); })
                .then(function (src) { return out.copyPages(src, src.getPageIndices()); })
                .then(function (pages) { pages.forEach(function (p) { out.addPage(p); }); });
            });
            return chain.then(function () { return out.save(); }).then(function (bytes) {
              var blob = new Blob([bytes], { type: 'application/pdf' });
              var name = stem(first.name) + '-merged.pdf';
              offer(blob, name);
              done('<span class="mono">' + esc(name) + '</span><span class="caption">' + picked.length + ' files · ' + kb(blob.size) + ' · 0 network calls</span>');
            });
          });
        }

        return first.arrayBuffer()
          .then(function (b) { return L.PDFDocument.load(b, { ignoreEncryption: true }); })
          .then(function (doc) {
            var count = doc.getPageCount();

            if (id === 'pdf-split') {
              var every = Math.max(1, Number(paramValue('every')) || 1);
              var parts = [];
              for (var s = 0; s < count; s += every) parts.push([s, Math.min(s + every, count)]);
              var chain = Promise.resolve();
              var made = 0;
              parts.forEach(function (range, idx) {
                chain = chain.then(function () {
                  return L.PDFDocument.create().then(function (out) {
                    var idxs = [];
                    for (var i = range[0]; i < range[1]; i++) idxs.push(i);
                    return out.copyPages(doc, idxs).then(function (pages) {
                      pages.forEach(function (p) { out.addPage(p); });
                      return out.save();
                    }).then(function (bytes) {
                      offer(new Blob([bytes], { type: 'application/pdf' }), stem(first.name) + '-' + (idx + 1) + '.pdf');
                      made++;
                    });
                  });
                });
              });
              return chain.then(function () {
                done('<span class="mono">' + made + ' files</span><span class="caption">' + count + ' pages, ' + every + ' per file · 0 network calls</span>');
              });
            }

            var keep;
            if (id === 'pdf-extract') {
              keep = parsePages(paramValue('pages'), count);
              if (!keep.length) throw new Error('name the pages to keep, like 1-3,7');
            } else if (id === 'pdf-remove') {
              var drop = parsePages(paramValue('pages'), count);
              if (!drop.length) throw new Error('name the pages to remove, like 1-3,7');
              keep = [];
              for (var i = 0; i < count; i++) if (drop.indexOf(i) < 0) keep.push(i);
              if (!keep.length) throw new Error('that would remove every page');
            } else if (id === 'pdf-reorder') {
              var named = parsePages(paramValue('order'), count);
              if (!named.length) throw new Error('give an order, like 3,1,2');
              // REORDERING MUST NOT DELETE. "3,1,2" on a six-page document
              // named three pages and returned a three-page file, quietly
              // dropping half of it — extract's behaviour under reorder's
              // name. Pages nobody mentioned keep their original order behind
              // the ones that were.
              keep = named.slice();
              for (var j = 0; j < count; j++) if (named.indexOf(j) < 0) keep.push(j);
            } else {
              keep = doc.getPageIndices();
            }

            return L.PDFDocument.create().then(function (out) {
              return out.copyPages(doc, keep).then(function (pages) {
                pages.forEach(function (p) {
                  if (id === 'pdf-rotate') {
                    var turn = Number(paramValue('turn')) || 90;
                    p.setRotation(L.degrees((p.getRotation().angle + turn) % 360));
                  }
                  if (id === 'pdf-crop') {
                    var m = Math.max(0, Number(paramValue('margin')) || 0);
                    var box = p.getMediaBox();
                    p.setCropBox(box.x + m, box.y + m, Math.max(1, box.width - 2 * m), Math.max(1, box.height - 2 * m));
                  }
                  out.addPage(p);
                });
                return out.save();
              });
            }).then(function (bytes) {
              var blob = new Blob([bytes], { type: 'application/pdf' });
              var name = stem(first.name) + '-' + id.replace('pdf-', '') + '.pdf';
              offer(blob, name);
              done('<span class="mono">' + esc(name) + '</span><span class="caption">' + keep.length + ' of ' + count + ' pages · ' + kb(blob.size) + ' · 0 network calls</span>');
            });
          });
      });
    }

    rEls.fileIn.addEventListener('change', function () {
      picked = [].slice.call(rEls.fileIn.files || []);
      rEls.pickLabel.textContent = picked.length
        ? (picked.length === 1 ? picked[0].name : picked.length + ' files')
        : 'Choose a file';
      rEls.status.textContent = '';
    });

    rEls.go.addEventListener('click', function () {
      var t = activeTool();
      if (!t || !picked.length) { rEls.status.textContent = 'choose a file first'; return; }
      rEls.go.disabled = true;
      rEls.status.textContent = 'working…';
      probeEncoders()
        .then(function () {
          return t.id.indexOf('pdf-') === 0 ? runPdf(t.id) : runImage(t.id);
        })
        .catch(function (e) { rEls.status.textContent = e.message || 'that did not run'; })
        .then(function () { rEls.go.disabled = false; });
    });

    // The tool is chosen by a radio the stylesheet also reads, so the runner
    // follows the same signal the window's own navigation does.
    [].forEach.call(document.querySelectorAll('.dv'), function (r) {
      r.addEventListener('change', function () {
        // Probe first: the compress tool's format list is built from what this
        // browser can actually write, and rendering it before the answer is in
        // gives an empty menu on the first open.
        probeEncoders().then(sync, sync);
      });
    });
    probeEncoders().then(sync, sync);
  }
})();
