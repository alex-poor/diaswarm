// The "sealed envelopes" explainer in the how-it-works section of index.html.
//
// Plain JS + inline SVG, no build step, like the rest of site/. The step text is
// NOT in this file: it is the <ol class="ex-steps"> in index.html, which is what
// a visitor without JavaScript reads instead. Change the words there.
//
// The wording follows docs/feasibility.md §11 — "stop sharing", never "revoke";
// what a reader already has, they keep. Keep it that way if you edit the steps.
//
// Model: each step is a list of phases; each phase is a patch of target states
// for named elements. Showing step N statically means applying every phase of
// steps 0..N instantly; playing step N means rendering N-1 statically and then
// animating N's phases in order. So any step can be jumped to, and the picture
// is always the same however you got there.
(function () {
  'use strict';
  var fig = document.getElementById('explainer');
  if (!fig || !window.SVGElement) return;

  var NS = 'http://www.w3.org/2000/svg';
  var reduce = window.matchMedia('(prefers-reduced-motion: reduce)');
  var stage = fig.querySelector('.ex-stage');
  var svg = stage.querySelector('svg');
  var cap = fig.querySelector('.ex-cap');
  var capCount = cap.querySelector('.ex-count');
  var capTitle = cap.querySelector('h3');
  var capBody = cap.querySelector('p');
  var dotsEl = fig.querySelector('.ex-dots');
  var btnPrev = fig.querySelector('[data-ex=prev]');
  var btnNext = fig.querySelector('[data-ex=next]');
  var btnPlay = fig.querySelector('[data-ex=play]');

  var TEXT = Array.prototype.map.call(fig.querySelectorAll('.ex-steps > li'), function (li) {
    return { title: li.querySelector('h3').textContent.trim(), body: li.querySelector('p').textContent.trim() };
  });

  var EASE = 'cubic-bezier(.45,0,.2,1)';
  var POP = 'cubic-bezier(.34,1.56,.64,1)';

  // ---------------------------------------------------------------- layouts
  // wide: you → strangers (a column) → mum, left to right.
  // tall: you and mum side by side on top, strangers in a row beneath — the
  //       envelopes go down to the pool and back up, so it stays short on a phone.
  var LAYOUTS = {
    wide: {
      w: 1000, h: 480, fs: 20, sfs: 15,
      big: { w: 116, h: 196 }, small: { w: 80, h: 120 }, envSmall: 0.6,
      you: [130, 250], mum: [870, 250], s: [[500, 105], [500, 250], [500, 395]],
      youLabel: [130, 384], mumLabel: [870, 384], sLabel: [500, 22],
      keyLabel: [130, 124], noKey: [870, 424],
      badge: function (i) { return { x: 554, y: this.s[i][1], anchor: 'start' }; }
    },
    tall: {
      w: 400, h: 524, fs: 16, sfs: 13,
      big: { w: 100, h: 168 }, small: { w: 72, h: 104 }, envSmall: 0.56,
      you: [100, 128], mum: [300, 128], s: [[72, 420], [200, 420], [328, 420]],
      youLabel: [100, 240], mumLabel: [300, 240], sLabel: [200, 330],
      keyLabel: [100, 22], noKey: [300, 276],
      badge: function (i) { return { x: this.s[i][0], y: 500, anchor: 'middle' }; }
    }
  };

  // ---------------------------------------------------------------- svg helpers
  function el(tag, attrs, parent) {
    var n = document.createElementNS(NS, tag);
    for (var k in attrs) n.setAttribute(k, attrs[k]);
    if (parent) parent.appendChild(n);
    return n;
  }
  function txt(parent, str, attrs) { var t = el('text', attrs, parent); t.textContent = str; return t; }
  function textWidth(str, fs) { return str.length * fs * 0.55; }

  function pill(parent, x, y, str, fs, cls) {
    var g = el('g', { transform: 'translate(' + x + ' ' + y + ')', class: 'ex-pill ' + (cls || '') }, parent);
    var h = fs * 1.9, w = textWidth(str, fs) + fs * 1.6;
    el('rect', { x: -w / 2, y: -h / 2, width: w, height: h, rx: h / 2 }, g);
    txt(g, str, { x: 0, y: fs * 0.36, 'font-size': fs, 'text-anchor': 'middle' });
    return g;
  }

  function phone(parent, c, d, kind) {
    var g = el('g', { transform: 'translate(' + c[0] + ' ' + c[1] + ')' }, parent);
    var ring = el('rect', { x: -d.w / 2, y: -d.h / 2, width: d.w, height: d.h, rx: d.w * 0.2, class: 'ex-ring ' + kind }, g);
    el('rect', { x: -d.w / 2, y: -d.h / 2, width: d.w, height: d.h, rx: d.w * 0.2, class: 'ex-phone ' + kind }, g);
    el('rect', { x: -d.w / 2 + 6, y: -d.h / 2 + 6, width: d.w - 12, height: d.h - 12, rx: d.w * 0.15, class: 'ex-screen' }, g);
    el('rect', { x: -d.w * 0.14, y: -d.h / 2 + 12, width: d.w * 0.28, height: 5, rx: 2.5, class: 'ex-notch' }, g);
    return ring;
  }

  function envelope(withKey) {
    var g = el('g');
    var inn = el('g', { class: 'ex-in' }, g);
    el('rect', { x: -34, y: -23, width: 68, height: 46, rx: 8, class: 'ex-env', filter: 'url(#ex-shadow)' }, inn);
    el('path', { d: 'M-29 -18 L0 3 L29 -18', class: 'ex-flap' }, inn);
    el('path', { d: 'M-5.5 5 v-4 a5.5 5.5 0 0 1 11 0 v4', class: 'ex-shackle' }, inn);
    el('rect', { x: -9, y: 4, width: 18, height: 14, rx: 3.5, class: 'ex-lockbody' }, inn);
    var tag = el('g', { 'data-k': '', class: 'ex-tag' }, inn);
    if (withKey) {
      el('circle', { cx: 32, cy: -22, r: 14, class: 'ex-keytag' }, tag);
      el('circle', { cx: 27, cy: -22, r: 4, class: 'ex-keyglyph' }, tag);
      el('path', { d: 'M31 -22 H40 M37 -22 V-18', class: 'ex-keyglyph' }, tag);
    } else {
      el('circle', { cx: 32, cy: -22, r: 13, class: 'ex-noslot' }, tag);
      el('path', { d: 'M27 -27 L37 -17', class: 'ex-noslash' }, tag);
    }
    return g;
  }

  function chip(val, sub, mum) {
    var g = el('g', { class: 'ex-chip' + (mum ? ' mum' : '') });
    txt(g, val, { x: 0, y: 4, 'font-size': 40, 'text-anchor': 'middle', class: 'ex-val' });
    if (mum) {
      el('rect', { x: -43, y: 13, width: 86, height: 21, rx: 10.5, class: 'ex-agepill' }, g);
      txt(g, sub, { x: 0, y: 28, 'font-size': 12, 'text-anchor': 'middle', class: 'ex-sub', 'data-t': '' });
    } else {
      txt(g, sub, { x: 0, y: 25, 'font-size': 12, 'text-anchor': 'middle', class: 'ex-sub', 'data-t': '' });
    }
    return g;
  }

  function lockIcon(parent, x, y, s) {
    var g = el('g', { transform: 'translate(' + x + ' ' + y + ') scale(' + s + ')', class: 'ex-minilock' }, parent);
    el('path', { d: 'M-4 0 v-3 a4 4 0 0 1 8 0 v3', fill: 'none' }, g);
    el('rect', { x: -6, y: -0.5, width: 12, height: 9, rx: 2 }, g);
    return g;
  }

  function badge(fs) {
    var g = el('g', { class: 'ex-badge' });
    var str = "can't open";
    var w = 16 + textWidth(str, fs);
    var inner = el('g', { 'data-a': '' }, g);
    lockIcon(inner, 6, -2, fs / 14);
    txt(inner, str, { x: 16, y: fs * 0.36, 'font-size': fs });
    g._w = w;
    return g;
  }

  function sleepOverlay(d) {
    var g = el('g', { class: 'ex-sleep' });
    el('rect', { x: -d.w / 2 + 6, y: -d.h / 2 + 6, width: d.w - 12, height: d.h - 12, rx: d.w * 0.15 }, g);
    el('path', { d: 'M4 -26 a14 14 0 1 0 12 20 a11 11 0 0 1 -12 -20 z', class: 'ex-moon' }, g);
    txt(g, 'asleep', { x: 0, y: 22, 'font-size': 14, 'text-anchor': 'middle' });
    return g;
  }

  // ---------------------------------------------------------------- build
  var L, mode, D, R, S, INIT, STEPS;

  function build(m) {
    mode = m;
    L = LAYOUTS[m];
    while (svg.firstChild) svg.removeChild(svg.firstChild);
    svg.setAttribute('viewBox', '0 0 ' + L.w + ' ' + L.h);
    svg.setAttribute('class', m);

    var defs = el('defs', {}, svg);
    var f = el('filter', { id: 'ex-shadow', x: '-40%', y: '-40%', width: '180%', height: '200%' }, defs);
    el('feDropShadow', { dx: 0, dy: 5, stdDeviation: 5, 'flood-color': '#000', 'flood-opacity': 0.45 }, f);
    var gr = el('linearGradient', { id: 'ex-envgrad', x1: 0, y1: 0, x2: 0, y2: 1 }, defs);
    el('stop', { offset: 0, 'stop-color': '#ffffff' }, gr);
    el('stop', { offset: 1, 'stop-color': '#e2e6df' }, gr);
    var sg = el('linearGradient', { id: 'ex-screengrad', x1: 0, y1: 0, x2: 0, y2: 1 }, defs);
    el('stop', { offset: 0, 'stop-color': '#121920' }, sg);
    el('stop', { offset: 1, 'stop-color': '#080b0e' }, sg);

    // flow lines, faint, so the route is legible before anything moves
    var flows = el('g', { class: 'ex-flows' }, svg);
    L.s.forEach(function (p) {
      [[L.you, p], [p, L.mum]].forEach(function (pair) {
        var a = pair[0], b = pair[1];
        var mx = (a[0] + b[0]) / 2, my = (a[1] + b[1]) / 2 - (m === 'wide' ? 26 : 0);
        el('path', { d: 'M' + a[0] + ' ' + a[1] + ' Q' + mx + ' ' + my + ' ' + b[0] + ' ' + b[1] }, flows);
      });
    });

    R = {};
    R.you = phone(svg, L.you, L.big, 'you');
    R.mum = phone(svg, L.mum, L.big, 'mum');
    L.s.forEach(function (p, i) { R['s' + i] = phone(svg, p, L.small, 'stranger'); });

    pill(svg, L.youLabel[0], L.youLabel[1], 'You', L.fs, 'you');
    pill(svg, L.mumLabel[0], L.mumLabel[1], 'Mum', L.fs, 'mum');
    txt(svg, "other people's phones", { x: L.sLabel[0], y: L.sLabel[1], 'font-size': L.sfs, 'text-anchor': 'middle', class: 'ex-grp' });

    D = {};
    function dyn(id, node) { node.classList.add('ex-dyn'); svg.appendChild(node); D[id] = node; }

    dyn('chipA', chip('5.7', 'mmol/L'));
    dyn('envA', envelope(true));
    dyn('chipB', chip('6.1', 'mmol/L'));
    dyn('envB', envelope(false));
    dyn('sleep', sleepOverlay(L.big));
    for (var i = 0; i < 3; i++) {
      dyn('envA' + i, envelope(true));
      dyn('envB' + i, envelope(false));
      var b = badge(L.sfs);
      dyn('badge' + i, b);
      if (L.badge(i).anchor === 'middle') b.querySelector('[data-a]').setAttribute('transform', 'translate(' + (-b._w / 2) + ' 0)');
    }
    var kl = el('g', {});
    txt(kl, '+ a key only Mum can use', { x: 0, y: 0, 'font-size': L.sfs, 'text-anchor': 'middle', class: 'ex-keylbl' });
    dyn('keyLabel', kl);
    var nk = el('g', {});
    txt(nk, "can't open — no key", { x: 0, y: 0, 'font-size': L.sfs, 'text-anchor': 'middle', class: 'ex-nokey' });
    dyn('noKey', nk);
    dyn('mumChip', chip('5.7', '2 min ago', true));
    dyn('envAm', envelope(true));
    dyn('envBm', envelope(false));

    defineSteps();
  }

  function P(pt, dy) { return { x: pt[0], y: pt[1] + (dy || 0) }; }
  function mix(a, b) { var o = {}, k; for (k in a) o[k] = a[k]; for (k in b) o[k] = b[k]; return o; }

  function defineSteps() {
    var you = L.you, mum = L.mum, s = L.s, SS = L.envSmall;
    var hidden = { s: 0.6, o: 0 };

    INIT = {
      chipA: mix(P(you, -4), hidden),
      envA: mix(P(you), { s: 0.5, o: 0, k: 0 }),
      chipB: mix(P(you, -4), hidden),
      envB: mix(P(you), { s: 0.5, o: 0 }),
      sleep: mix(P(you), { s: 1, o: 0 }),
      envAm: mix(P(s[1]), { s: SS, o: 0 }),
      envBm: mix(P(s[1]), { s: SS, o: 0 }),
      mumChip: mix(P(mum, -18), { s: 0.6, o: 0, t: '2 min ago', cls: '' }),
      keyLabel: mix(P(L.keyLabel), { s: 1, o: 0 }),
      noKey: mix(P(L.noKey), { s: 1, o: 0 })
    };
    for (var i = 0; i < 3; i++) {
      INIT['envA' + i] = mix(P(you), { s: 0.9, o: 0 });
      INIT['envB' + i] = mix(P(you), { s: 0.9, o: 0 });
      var bp = L.badge(i);
      INIT['badge' + i] = { x: bp.x, y: bp.y, s: 1, o: 0 };
    }

    function fan(prefix, extra) {
      var o = {};
      s.forEach(function (p, i) { o[prefix + i] = mix(mix(P(p), { s: SS, o: 1, arc: mode === 'wide' ? 46 : 30, dl: i * 140 }), extra || {}); });
      return o;
    }
    function arrivals(d) { return s.map(function (p, i) { return ['s' + i, d * 0.85 + i * 140]; }); }

    STEPS = [
      [{ d: 700, set: { chipA: { o: 1, s: 1, ease: POP } } }],
      [
        { d: 420, set: { chipA: { o: 0, s: 0.35 } }, gap: -220 },
        { d: 650, set: { envA: { o: 1, s: 1, ease: POP } } }
      ],
      [{ d: 650, set: { envA: { k: 1 }, keyLabel: { o: 1 } } }],
      [
        { d: 300, set: { keyLabel: { o: 0 } }, gap: 0 },
        { d: 1100, set: fan('envA'), fx: arrivals(1100), gap: 200 },
        { d: 450, set: { badge0: { o: 1 }, badge1: { o: 1, dl: 120 }, badge2: { o: 1, dl: 240 } } }
      ],
      [
        { d: 700, set: { sleep: { o: 1 } } },
        { d: 1250, set: { envAm: mix(P(mum, -10), { s: 1, o: 1, arc: mode === 'wide' ? 60 : 90 }) } }
      ],
      [
        { d: 420, set: { envAm: { s: 1.3, o: 0 } }, gap: -160 },
        { d: 700, set: { mumChip: { o: 1, s: 1, ease: POP } }, fx: [['mum', 120]] }
      ],
      [
        { d: 650, set: (function () {
            var o = { sleep: { o: 0 }, envA: { o: 0, s: 0.6 }, mumChip: { t: 'yesterday', cls: 'stale' } };
            s.forEach(function (p, i) { o['envA' + i] = mix(P(p, -30), { s: 0.38, o: 0.45 }); });
            return o;
          })() },
        { d: 600, set: { chipB: { o: 1, s: 1, ease: POP } } },
        { d: 420, set: { chipB: { o: 0, s: 0.35 } }, gap: -220 },
        { d: 650, set: { envB: { o: 1, s: 1, ease: POP } } },
        { d: 1100, set: fan('envB'), fx: arrivals(1100), gap: 200 },
        { d: 1250, set: { envBm: mix(P(mum, 46), { s: 0.7, o: 1, arc: mode === 'wide' ? 60 : 90 }) } },
        { d: 550, set: { envBm: { shake: true, cls: 'denied' }, noKey: { o: 1 } } }
      ]
    ];
  }

  // ---------------------------------------------------------------- rendering
  var live = [];

  function T(st) { return 'translate(' + st.x + 'px,' + st.y + 'px) scale(' + st.s + ')'; }
  function tagT(k) { return 'scale(' + (k ? 1 : 0.2) + ')'; }

  function paint(id) {
    var n = D[id], st = S[id];
    n.style.transform = T(st);
    n.style.opacity = st.o;
    n.style.visibility = st.o ? 'visible' : 'hidden';
    if ('k' in st) { var tag = n.querySelector('[data-k]'); tag.style.opacity = st.k; tag.style.transform = tagT(st.k); }
    if ('t' in st) n.querySelector('[data-t]').textContent = st.t;
    if ('cls' in st) n.setAttribute('data-state', st.cls);
  }

  function reset() {
    live.forEach(function (a) { a.cancel(); });
    live = [];
    S = {};
    for (var id in INIT) { S[id] = mix(INIT[id], {}); paint(id); }
  }

  // Animate one phase. dur 0 = apply instantly. Returns the largest delay used.
  function apply(set, dur) {
    var maxDl = 0;
    for (var id in set) {
      var p = set[id], n = D[id], st = S[id], from = mix(st, {});
      ['x', 'y', 's', 'o', 'k', 't', 'cls'].forEach(function (k) { if (k in p) st[k] = p[k]; });
      var dl = p.dl || 0;
      maxDl = Math.max(maxDl, dl);
      paint(id);
      if (!dur || !n.animate) continue;
      if (from.o || st.o) n.style.visibility = 'visible';

      if (from.x !== st.x || from.y !== st.y || from.s !== st.s || from.o !== st.o) {
        var kf;
        if (p.arc) {
          // sample a quadratic curve so the flight bends instead of kinking
          var cx = (from.x + st.x) / 2, cy = (from.y + st.y) / 2 - p.arc * 2;
          kf = [];
          for (var j = 0; j <= 16; j++) {
            var t = j / 16, u = 1 - t;
            var x = u * u * from.x + 2 * u * t * cx + t * t * st.x;
            var y = u * u * from.y + 2 * u * t * cy + t * t * st.y;
            var sc = from.s + (st.s - from.s) * t + Math.sin(Math.PI * t) * 0.1;
            var op = from.o + (st.o - from.o) * Math.min(1, t / 0.2);
            kf.push({ transform: T({ x: x, y: y, s: sc }), opacity: op });
          }
        } else {
          kf = [{ transform: T(from), opacity: from.o }, { transform: T(st), opacity: st.o }];
        }
        live.push(n.animate(kf, { duration: dur, delay: dl, easing: p.ease || EASE, fill: 'backwards' }));
      }
      if ('k' in p && from.k !== st.k) {
        live.push(n.querySelector('[data-k]').animate(
          [{ opacity: from.k, transform: tagT(from.k) }, { opacity: st.k, transform: tagT(st.k) }],
          { duration: dur, delay: dl, easing: POP, fill: 'backwards' }));
      }
      if (p.shake) {
        live.push(n.querySelector('.ex-in').animate(
          [0, -8, 7, -5, 3, 0].map(function (v) { return { transform: 'translateX(' + v + 'px)' }; }),
          { duration: dur, delay: dl, easing: 'ease-out' }));
      }
    }
    return maxDl;
  }

  function pulse(ring, delay) {
    if (!R[ring].animate) return null;
    return R[ring].animate(
      [{ opacity: 0.85, transform: 'scale(1)' }, { opacity: 0, transform: 'scale(1.16)' }],
      { duration: 900, delay: delay, easing: 'ease-out' });
  }

  // ---------------------------------------------------------------- sequencing
  var N = TEXT.length;
  var cur = 0, token = 0, timer = 0, done = true, started = false, inView = false;
  var auto = !reduce.matches;

  function sleep(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
  function running() { return auto && inView && !document.hidden; }

  function caption(i) {
    capCount.textContent = 'Step ' + (i + 1) + ' of ' + N;
    capTitle.textContent = TEXT[i].title;
    capBody.textContent = TEXT[i].body;
    Array.prototype.forEach.call(dotsEl.children, function (b, j) {
      b.classList.toggle('past', j < i);
      if (j === i) b.setAttribute('aria-current', 'step'); else b.removeAttribute('aria-current');
    });
    if (!reduce.matches && cap.animate) {
      cap.animate([{ opacity: 0, transform: 'translateY(6px)' }, { opacity: 1, transform: 'none' }],
        { duration: 380, easing: 'ease-out' });
    }
  }

  function staticTo(i) {
    token++;
    clearTimeout(timer);
    reset();
    for (var j = 0; j <= i; j++) STEPS[j].forEach(function (ph) { apply(ph.set, 0); });
    cur = Math.max(i, 0);
    done = true;
  }

  function play(i) {
    started = true;
    staticTo(i - 1);
    var my = ++token;
    cur = i;
    done = false;
    caption(i);
    var phases = STEPS[i], k = 0;
    (function nextPhase() {
      if (my !== token) return;
      if (k >= phases.length) { done = true; schedule(); return; }
      var ph = phases[k++];
      var d = reduce.matches ? 0 : ph.d;
      var extra = apply(ph.set, d);
      if (d && ph.fx) ph.fx.forEach(function (f) { var a = pulse(f[0], f[1]); if (a) live.push(a); });
      var wait = d ? d + extra + (ph.gap == null ? 150 : ph.gap) : 0;
      if (wait > 0) sleep(wait).then(nextPhase); else nextPhase();
    })();
  }

  function hold(i) { return (i === N - 1 ? 6500 : 1800) + TEXT[i].body.length * 24; }

  function schedule() {
    clearTimeout(timer);
    if (done && running()) timer = setTimeout(function () { play(cur >= N - 1 ? 0 : cur + 1); }, hold(cur));
  }

  function setAuto(on, byUser) {
    auto = on;
    btnPlay.setAttribute('aria-label', on ? 'Pause' : 'Play');
    btnPlay.classList.toggle('paused', !on);
    if (!on) clearTimeout(timer);
    else if (!started) { if (byUser || running()) play(0); }
    else if (done && cur === N - 1) play(0);
    else schedule();
  }

  // ---------------------------------------------------------------- wiring
  TEXT.forEach(function (t, j) {
    var li = document.createElement('li');
    var b = document.createElement('button');
    b.type = 'button';
    b.setAttribute('aria-label', 'Step ' + (j + 1) + ': ' + t.title);
    b.addEventListener('click', function () { play(j); });
    li.appendChild(b);
    dotsEl.appendChild(li);
  });
  btnPrev.addEventListener('click', function () { play(Math.max(0, cur - 1)); });
  btnNext.addEventListener('click', function () { play(cur >= N - 1 ? 0 : cur + 1); });
  btnPlay.addEventListener('click', function () { setAuto(!auto, true); });

  function pickMode() { return stage.clientWidth < 640 ? 'tall' : 'wide'; }

  build(pickMode());
  staticTo(0);
  caption(0);
  setAuto(auto); // syncs the button; does not start anything before it's seen
  fig.classList.add('js');

  if ('ResizeObserver' in window) {
    new ResizeObserver(function () {
      var m = pickMode();
      if (m === mode) return;
      build(m);
      staticTo(cur);
      schedule();
    }).observe(stage);
  }

  if ('IntersectionObserver' in window) {
    new IntersectionObserver(function (es) {
      inView = es[0].isIntersecting;
      if (!inView) { clearTimeout(timer); return; }
      if (auto && !started) play(0); else schedule();
    }, { threshold: 0.45 }).observe(stage);
  } else {
    inView = true;
    if (auto) play(0);
  }
  document.addEventListener('visibilitychange', schedule);
})();
