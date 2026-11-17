// The brochure explainer: seven steps, sealed envelopes. Engine: explainer.js.
// Step text lives in index.html (<ol class="ex-steps">), not here.
(function () {
  'use strict';
  var X = window.diaswarmExplainer;
  if (!X) return;
  var POP = X.POP;

  function P(pt, dy) { return { x: pt[0], y: pt[1] + (dy || 0) }; }
  function mix(a, b) { var o = {}, k; for (k in a) o[k] = a[k]; for (k in b) o[k] = b[k]; return o; }

  X.register('simple', {
    // wide: you → strangers (a column) → mum, left to right.
    // tall: you and mum side by side on top, strangers in a row beneath, so the
    //       envelopes go down to the pool and back up and it stays short on a phone.
    layouts: {
      wide: {
        w: 1000, h: 480, fs: 20, sfs: 15,
        big: { w: 116, h: 196 }, small: { w: 80, h: 120 }, envSmall: 0.6,
        you: [130, 250], mum: [870, 250], s: [[500, 105], [500, 250], [500, 395]],
        youLabel: [130, 384], mumLabel: [870, 384], sLabel: [500, 22],
        keyLabel: [130, 124], noKey: [870, 424], mumKey: { x: 922, y: 158 },
        badge: function (i) { return { x: 554, y: this.s[i][1], anchor: 'start' }; }
      },
      tall: {
        w: 400, h: 524, fs: 16, sfs: 13,
        big: { w: 100, h: 168 }, small: { w: 72, h: 104 }, envSmall: 0.56,
        you: [100, 128], mum: [300, 128], s: [[72, 420], [200, 420], [328, 420]],
        youLabel: [100, 240], mumLabel: [300, 240], sLabel: [200, 330],
        keyLabel: [100, 22], noKey: [300, 276], mumKey: { x: 344, y: 50 },
        badge: function (i) { return { x: this.s[i][0], y: 500, anchor: 'middle' }; }
      }
    },

    build: function (c) {
      var svg = c.svg, L = c.L, H = c.H, dyn = c.dyn;

      var flows = H.el('g', { class: 'ex-flows' }, svg);
      L.s.forEach(function (p) {
        [[L.you, p], [p, L.mum]].forEach(function (pair) {
          var a = pair[0], b = pair[1];
          var mx = (a[0] + b[0]) / 2, my = (a[1] + b[1]) / 2 - (c.mode === 'wide' ? 26 : 0);
          H.el('path', { d: 'M' + a[0] + ' ' + a[1] + ' Q' + mx + ' ' + my + ' ' + b[0] + ' ' + b[1] }, flows);
        });
      });

      c.R.you = H.phone(svg, L.you, L.big, 'you');
      c.R.mum = H.phone(svg, L.mum, L.big, 'mum');
      L.s.forEach(function (p, i) { c.R['s' + i] = H.phone(svg, p, L.small, 'stranger'); });

      H.pill(svg, L.youLabel[0], L.youLabel[1], 'You', L.fs, 'you');
      H.pill(svg, L.mumLabel[0], L.mumLabel[1], 'Mum', L.fs, 'mum');
      H.txt(svg, "other people's phones", { x: L.sLabel[0], y: L.sLabel[1], 'font-size': L.sfs, 'text-anchor': 'middle', class: 'ex-grp' });

      dyn('chipA', H.chip('5.7', 'mmol/L'));
      dyn('envA', H.envelope('key'));
      dyn('chipB', H.chip('6.1', 'mmol/L'));
      dyn('envB', H.envelope('nokey'));
      dyn('sleep', H.sleepOverlay(L.big));
      for (var i = 0; i < 3; i++) {
        dyn('envA' + i, H.envelope('key'));
        dyn('envB' + i, H.envelope('nokey'));
        dyn('badge' + i, H.label("can't open", L.sfs, 'ex-badge', { lock: true, mono: true, anchor: L.badge(i).anchor }));
      }
      dyn('keyLabel', H.label('Mum is given the key', L.sfs, 'ex-keylbl'));
      dyn('noKey', H.label("can't open — new key", L.sfs, 'ex-nokey'));
      var mk = H.el('g', { class: 'ex-tag' });
      H.el('circle', { cx: 0, cy: 0, r: 14, class: 'ex-keytag' }, mk);
      H.el('circle', { cx: -5, cy: 0, r: 4, class: 'ex-keyglyph' }, mk);
      H.el('path', { d: 'M-1 0 H8 M5 0 V4', class: 'ex-keyglyph' }, mk);
      dyn('mumKey', mk);
      dyn('mumChip', H.chip('5.7', '2 min ago', true));
      dyn('envAm', H.envelope('key'));
      dyn('envBm', H.envelope('nokey'));
    },

    init: function (L) {
      var you = L.you, mum = L.mum, s = L.s, SS = L.envSmall;
      var hidden = { s: 0.6, o: 0 };
      var I = {
        chipA: mix(P(you, -4), hidden),
        envA: mix(P(you), { s: 0.5, o: 0, k: 0 }),
        chipB: mix(P(you, -4), hidden),
        envB: mix(P(you), { s: 0.5, o: 0 }),
        sleep: mix(P(you), { s: 1, o: 0 }),
        envAm: mix(P(s[1]), { s: SS, o: 0 }),
        envBm: mix(P(s[1]), { s: SS, o: 0 }),
        mumChip: mix(P(mum, -18), { s: 0.6, o: 0, t: '2 min ago', cls: '' }),
        keyLabel: mix(P(L.keyLabel), { s: 1, o: 0 }),
        noKey: mix(P(L.noKey), { s: 1, o: 0 }),
        mumKey: mix(P(you, -30), { s: 0.4, o: 0 })
      };
      for (var i = 0; i < 3; i++) {
        I['envA' + i] = mix(P(you), { s: 0.9, o: 0 });
        I['envB' + i] = mix(P(you), { s: 0.9, o: 0 });
        var bp = L.badge(i);
        I['badge' + i] = { x: bp.x, y: bp.y, s: 1, o: 0 };
      }
      return I;
    },

    steps: function (L, mode) {
      var you = L.you, mum = L.mum, s = L.s, SS = L.envSmall;
      function fan(prefix) {
        var o = {};
        s.forEach(function (p, i) { o[prefix + i] = mix(P(p), { s: SS, o: 1, arc: mode === 'wide' ? 46 : 30, dl: i * 140 }); });
        return o;
      }
      function arrivals(d) { return s.map(function (p, i) { return ['s' + i, d * 0.85 + i * 140]; }); }
      var lift = mode === 'wide' ? 60 : 90;

      return [
        [{ d: 700, set: { chipA: { o: 1, s: 1, ease: POP } } }],
        [
          { d: 420, set: { chipA: { o: 0, s: 0.35 } }, gap: -220 },
          { d: 650, set: { envA: { o: 1, s: 1, ease: POP } } }
        ],
        [
          { d: 650, set: { envA: { k: 1 }, keyLabel: { o: 1 } } },
          { d: 1200, set: { mumKey: mix(L.mumKey, { s: 1, o: 1, arc: mode === 'wide' ? 70 : 50 }) }, fx: [['mum', 1050]] }
        ],
        [
          { d: 300, set: { keyLabel: { o: 0 } }, gap: 0 },
          { d: 1100, set: fan('envA'), fx: arrivals(1100), gap: 200 },
          { d: 450, set: { badge0: { o: 1 }, badge1: { o: 1, dl: 120 }, badge2: { o: 1, dl: 240 } } }
        ],
        [
          { d: 700, set: { sleep: { o: 1 } } },
          { d: 1250, set: { envAm: mix(P(mum, -10), { s: 1, o: 1, arc: lift }) } }
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
          { d: 1250, set: { envBm: mix(P(mum, 46), { s: 0.7, o: 1, arc: lift }) } },
          { d: 550, set: { envBm: { shake: true, cls: 'denied' }, noKey: { o: 1 } } }
        ]
      ];
    }
  });
})();
