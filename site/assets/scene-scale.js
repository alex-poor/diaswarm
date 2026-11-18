// The scale-up explainer on developers.html ("The idea at scale"). Engine: explainer.js.
// A different camera move from scene-simple / scene-dev: those pan ACROSS one reading;
// this one ZOOMS OUT — one key gesture becomes a whole commons.
//
// Four beats, text in developers.html (<ol class="ex-steps">):
//   1 the atom       — one subject, one follower, one key
//   2 three scopes   — the same gesture to a follower, a clinic, a study
//   3 zoom out       — the trio shrinks into a varied field of clusters + carriers + mesh
//   4 no middle      — a server box snaps in, then dissolves; the mesh carries on, flowing
//
// The field is deliberately VARIEGATED: clusters differ in size, in how many readers
// they have, and in what kind (follower / clinic / study), so the picture reads as a
// real, complex network rather than one shape stamped across the screen. The mesh links
// carry a continuous dash-flow (CSS, disabled under prefers-reduced-motion) to show live
// traffic at scale.
(function () {
  'use strict';
  var X = window.diaswarmExplainer;
  if (!X) return;
  var POP = X.POP;

  function mix(a, b) { var o = {}, k; for (k in a) o[k] = a[k]; for (k in b) o[k] = b[k]; return o; }

  // cluster anchors as fractions of the viewBox; [0] is where the hero trio collapses to
  var A = [
    [0.18, 0.50], [0.40, 0.16], [0.63, 0.24], [0.84, 0.14], [0.92, 0.42],
    [0.81, 0.71], [0.57, 0.83], [0.34, 0.74], [0.20, 0.86], [0.10, 0.27],
    [0.73, 0.52], [0.28, 0.57], [0.50, 0.30], [0.46, 0.92]
  ];
  // per-field-cluster composition (index matches A[1..]); varied on purpose.
  // k: 'r' follower (green), 'c' clinic (blue), 's' study (amber). sr = subject radius.
  var SPECS = [
    null,
    { sr: 9,    readers: [{ k: 'r', dx: 21, dy: -9, rr: 7 }] },
    { sr: 10.5, readers: [{ k: 'r', dx: 23, dy: -6, rr: 7.5 }, { k: 'r', dx: 7, dy: 21, rr: 6 }] },
    { sr: 9,    readers: [{ k: 'c', dx: 22, dy: -11, rr: 7.5 }, { k: 'r', dx: -5, dy: 22, rr: 6 }] },
    { sr: 9.5,  readers: [{ k: 's', dx: 23, dy: -4, rr: 7.5 }, { k: 'r', dx: -14, dy: 17, rr: 6 }] },
    { sr: 7.5,  readers: [{ k: 'r', dx: 18, dy: -10, rr: 6.5 }] },
    { sr: 11.5, readers: [{ k: 'r', dx: 25, dy: -11, rr: 7 }, { k: 'c', dx: 22, dy: 12, rr: 7 }, { k: 's', dx: -9, dy: 23, rr: 6.5 }] },
    { sr: 10,   readers: [{ k: 'r', dx: 25, dy: -8, rr: 8 }, { k: 'r', dx: -16, dy: 16, rr: 7 }] },
    { sr: 8,    readers: [{ k: 's', dx: 20, dy: -10, rr: 7.5 }] },
    { sr: 9,    readers: [{ k: 'r', dx: 22, dy: -9, rr: 7 }, { k: 'c', dx: 4, dy: 22, rr: 6.5 }] },
    { sr: 7.5,  readers: [{ k: 'r', dx: 17, dy: -9, rr: 6.5 }] },
    { sr: 8.5,  readers: [{ k: 'r', dx: 19, dy: 9, rr: 6.5 }] },
    { sr: 9,    readers: [{ k: 'c', dx: 19, dy: -10, rr: 7 }, { k: 'r', dx: -12, dy: 15, rr: 6 }] },
    { sr: 7,    readers: [{ k: 'r', dx: 16, dy: -10, rr: 6 }] }
  ];
  var MESH = [
    [0,1],[1,2],[2,3],[3,4],[4,5],[5,6],[6,7],[7,8],[8,9],[9,0],[0,2],[4,7],[3,6],
    [12,1],[12,2],[10,4],[10,5],[11,7],[11,8],[13,6],[13,7],[0,11]
  ];
  var SPOKES = [1, 3, 4, 6, 8];
  var CARRIERS = [
    [0.30,0.40],[0.50,0.13],[0.72,0.37],[0.88,0.60],[0.67,0.56],[0.47,0.47],
    [0.26,0.63],[0.42,0.61],[0.62,0.69],[0.31,0.25],[0.53,0.66],[0.75,0.85],
    [0.14,0.67],[0.87,0.27],[0.60,0.40],[0.37,0.34],[0.68,0.20],[0.22,0.44]
  ];

  function P(L, f, dy, dx) { return { x: f[0] * L.w + (dx || 0), y: f[1] * L.h + (dy || 0) }; }

  // ---- custom glyph builders ----
  function nodeG(H, cls, glyph, r) {
    var g = H.el('g', { class: 'ex-node ' + cls });
    H.el('circle', { cx: 0, cy: 0, r: r, class: 'ex-ncirc' }, g);
    if (glyph === 'cross') H.el('path', { d: 'M0 -9 V9 M-9 0 H9', class: 'ex-gly' }, g);
    else if (glyph === 'bars') H.el('path', { d: 'M-8 7 V-1 M0 7 V-7 M8 7 V2', class: 'ex-gly' }, g);
    else if (glyph === 'person') {
      H.el('circle', { cx: 0, cy: -4, r: 3.4, class: 'ex-gly-f' }, g);
      H.el('path', { d: 'M-6 9 a6 7 0 0 1 12 0', class: 'ex-gly' }, g);
    }
    return g;
  }
  function keyTok(H) {
    var g = H.el('g', { class: 'ex-ktok' });
    H.el('circle', { cx: 0, cy: 0, r: 13, class: 'ex-keytag' }, g);
    H.el('circle', { cx: -4, cy: 0, r: 3.6, class: 'ex-keyglyph' }, g);
    H.el('path', { d: 'M0 0 H8 M5 0 V4', class: 'ex-keyglyph' }, g);
    return g;
  }
  function readerCls(k) { return k === 'c' ? 'ex-cnode-mini' : k === 's' ? 'ex-anode-mini' : 'ex-rnode'; }
  function clusterG(H, spec) {
    var g = H.el('g', { class: 'ex-mc' });
    (spec.readers || []).forEach(function (rd) {
      H.el('line', { x1: 0, y1: 0, x2: rd.dx, y2: rd.dy, class: 'ex-mlink' }, g);
    });
    H.el('circle', { cx: 0, cy: 0, r: spec.sr || 9, class: 'ex-snode' }, g);
    (spec.readers || []).forEach(function (rd) {
      H.el('circle', { cx: rd.dx, cy: rd.dy, r: rd.rr || 7, class: readerCls(rd.k) }, g);
    });
    return g;
  }

  var DASH = ['2 11', '3 9', '2 14', '4 10'];

  X.register('scale', {
    breakpoint: 720,
    readingMsPerChar: 24,
    layouts: {
      wide: { w: 1120, h: 600, heroR: 34, fs: 15 },
      tall: { w: 420, h: 640, heroR: 26, fs: 13 }
    },

    build: function (c) {
      var svg = c.svg, L = c.L, H = c.H, dyn = c.dyn;
      var wide = c.mode === 'wide';
      var hx = wide ? 0.26 : 0.30, rx = wide ? 0.74 : 0.72;
      L.hero = { subj: [hx, 0.50], full: [rx, 0.50], clin: [rx, wide ? 0.20 : 0.18], res: [rx, wide ? 0.80 : 0.82] };

      // mesh lines — varied width / opacity (static) + a per-line flow speed (motion only)
      MESH.forEach(function (pair, i) {
        var a = P(L, A[pair[0]]), b = P(L, A[pair[1]]);
        var ln = H.el('path', { d: 'M' + a.x + ' ' + a.y + ' L' + b.x + ' ' + b.y, class: 'ex-mesh' + (i % 2 ? ' rev' : '') });
        ln.style.strokeOpacity = (0.4 + (i % 5) * 0.12).toFixed(2);
        ln.style.strokeWidth = (1 + (i % 3) * 0.45).toFixed(2);
        ln.style.strokeDasharray = DASH[i % DASH.length];
        ln.style.animationDuration = (1.2 + (i % 4) * 0.45).toFixed(2) + 's';
        dyn('mesh' + i, ln);
      });
      // carriers — varied size + opacity
      CARRIERS.forEach(function (f, i) {
        var p = P(L, f);
        var d = H.el('circle', { cx: p.x, cy: p.y, r: ([3, 4, 5, 6][i % 4]) * (wide ? 1 : 0.8), class: 'ex-carrier' });
        d.style.fillOpacity = (0.3 + (i % 4) * 0.2).toFixed(2);
        dyn('car' + i, d);
      });
      // field clusters (cluster 0 is the collapsed hero)
      for (var k = 1; k < A.length; k++) dyn('mc' + k, clusterG(H, SPECS[k]));

      // server box + spokes (beat 4)
      SPOKES.forEach(function (ai, i) {
        var s = P(L, [0.52, 0.50]), a = P(L, A[ai]);
        dyn('spoke' + i, H.el('path', { d: 'M' + s.x + ' ' + s.y + ' L' + a.x + ' ' + a.y, class: 'ex-spoke' }));
      });
      var sv = H.el('g', { class: 'ex-server' });
      var sw = wide ? 150 : 128, sh = wide ? 60 : 52;
      H.el('rect', { x: -sw / 2, y: -sh / 2, width: sw, height: sh, rx: 10 }, sv);
      H.txt(sv, 'SERVER', { x: 0, y: 5, 'font-size': wide ? 17 : 15, 'text-anchor': 'middle', class: 'ex-servertext' });
      dyn('server', sv);

      // hero nodes (on top)
      dyn('subj', nodeG(H, 'ex-snode-hero', 'person', L.heroR));
      dyn('full', nodeG(H, 'ex-rnode-hero', null, L.heroR));
      dyn('clin', nodeG(H, 'ex-cnode-hero', 'cross', L.heroR));
      dyn('res', nodeG(H, 'ex-anode-hero', 'bars', L.heroR));

      dyn('key0', keyTok(H));
      dyn('key1', keyTok(H));
      dyn('key2', keyTok(H));

      dyn('lsubj', H.label('you', L.fs, 'ex-nlabel'));
      dyn('lfull', H.label('a follower · full history', L.fs, 'ex-nlabel'));
      dyn('lclin', H.label('a clinic · a 90-day window', L.fs, 'ex-nlabel clin'));
      dyn('lres', H.label('a study · research rows', L.fs, 'ex-nlabel res'));
      dyn('lcommons', H.label('one commons · no centre', Math.round(L.fs * 1.6), 'ex-bignote'));
      dyn('lnomiddle', H.label('nothing to run · nothing to switch off', Math.round(L.fs * 1.6), 'ex-bignote'));
    },

    init: function (L) {
      var hero = L.hero, I = {};
      function node(f, extra) { return mix(P(L, f), mix({ s: 1, o: 0 }, extra || {})); }
      I.subj = node(hero.subj, { o: 1 });
      I.full = node(hero.full, { s: 0.6 });
      I.clin = node(hero.clin, { s: 0.6 });
      I.res  = node(hero.res,  { s: 0.6 });
      I.key0 = mix(P(L, hero.subj), { s: 0.4, o: 0 });
      I.key1 = mix(P(L, hero.subj), { s: 0.4, o: 0 });
      I.key2 = mix(P(L, hero.subj), { s: 0.4, o: 0 });
      I.lsubj = mix(P(L, hero.subj, L.heroR + 24), { s: 1, o: 0 });
      I.lfull = mix(P(L, hero.full, L.heroR + 22), { s: 1, o: 0 });
      I.lclin = mix(P(L, hero.clin, -L.heroR - 16), { s: 1, o: 0 });
      I.lres  = mix(P(L, hero.res,  L.heroR + 22), { s: 1, o: 0 });
      I.lcommons  = mix(P(L, [0.5, 0.045]), { s: 1, o: 0 });
      I.lnomiddle = mix(P(L, [0.5, 0.045]), { s: 1, o: 0 });
      for (var i = 0; i < MESH.length; i++) I['mesh' + i] = { x: 0, y: 0, s: 1, o: 0 };
      for (var k = 0; k < CARRIERS.length; k++) I['car' + k] = { x: 0, y: 0, s: 1, o: 0 };
      for (var m = 1; m < A.length; m++) I['mc' + m] = mix(P(L, A[m]), { s: 0.4, o: 0 });
      for (var sp = 0; sp < SPOKES.length; sp++) I['spoke' + sp] = { x: 0, y: 0, s: 1, o: 0 };
      I.server = mix(P(L, [0.52, 0.50]), { s: 0.6, o: 0 });
      return I;
    },

    steps: function (L) {
      var hero = L.hero, HR = L.heroR;
      var a0 = P(L, A[0]);
      var collapsedS = 10 / HR;

      function fadeField(baseDelay, step) {
        var o = {};
        for (var m = 1; m < A.length; m++) o['mc' + m] = { o: 1, s: 1, dl: baseDelay + (m - 1) * (step || 55) };
        return o;
      }
      function fadeCarriers(v, baseDelay) {
        var o = {};
        for (var k = 0; k < CARRIERS.length; k++) o['car' + k] = { o: v, dl: baseDelay + k * 24 };
        return o;
      }
      function setMesh(v, baseDelay, step) {
        var o = {};
        for (var i = 0; i < MESH.length; i++) o['mesh' + i] = { o: v, dl: baseDelay + i * (step || 32) };
        return o;
      }
      function setSpokes(v, baseDelay) {
        var o = {};
        for (var s = 0; s < SPOKES.length; s++) o['spoke' + s] = { o: v, dl: baseDelay + s * 60 };
        return o;
      }

      return [
        // BEAT 1 — the atom
        [
          { d: 500, set: { subj: { o: 1, s: 1, ease: POP }, lsubj: { o: 1 } } },
          { d: 1150, set: { key0: mix(P(L, hero.full), { s: 1, o: 1, arc: 54 }) }, gap: 0 },
          { d: 420, set: { key0: { o: 0, s: 0.5 } }, gap: -160 },
          { d: 600, set: { full: { o: 1, s: 1, ease: POP }, lfull: { o: 1 } } }
        ],
        // BEAT 2 — the same gesture, three scopes
        [
          { d: 900, set: {
              clin: { o: 1, s: 1, ease: POP },
              res:  { o: 1, s: 1, ease: POP, dl: 140 },
              key1: mix(P(L, hero.clin), { s: 0.9, o: 1, arc: 40 }),
              key2: mix(P(L, hero.res),  { s: 0.9, o: 1, arc: 40, dl: 140 })
            } },
          { d: 420, set: { key1: { o: 0, s: 0.5 }, key2: { o: 0, s: 0.5 } }, gap: -140 },
          { d: 520, set: { lclin: { o: 1 }, lres: { o: 1 } } }
        ],
        // BEAT 3 — zoom out
        [
          { d: 650, set: { lfull: { o: 0 }, lclin: { o: 0 }, lres: { o: 0 }, lsubj: { o: 0 } }, gap: -200 },
          { d: 900, set: {
              subj: mix(a0, { s: collapsedS }),
              full: mix({ x: a0.x + 22 * (L.w / 1120), y: a0.y - 10 }, { s: collapsedS * 0.85 }),
              clin: { o: 0, s: 0.3 }, res: { o: 0, s: 0.3 }
            } },
          { d: 1050, set: mix(fadeCarriers(0.9, 0), fadeField(120, 55)), gap: 150 },
          { d: 900, set: mix(setMesh(0.55, 0, 32), { lcommons: { o: 1 } }) }
        ],
        // BEAT 4 — the middle that was never there
        [
          { d: 700, set: mix(setSpokes(0.8, 0), { server: { o: 1, s: 1, ease: POP } }) },
          { d: 500, set: {}, gap: 500 },
          { d: 700, set: mix(setSpokes(0, 0), { server: { o: 0, s: 1.3 } }), gap: -150 },
          { d: 900, set: mix(setMesh(1, 0, 34), { lcommons: { o: 0 }, lnomiddle: { o: 1 } }) }
        ]
      ];
    }
  });
})();
