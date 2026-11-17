// The technical explainer on developers.html. Engine: explainer.js.
// Step text and the spec panels live in developers.html (<ol class="ex-steps">).
//
// This draws the SHIPPED path — crates/diaswarm-keys over diaswarm-net — not the
// diaswarm-core reference vault. So there are no per-reader wrap files: a grant
// is membership of a p2panda-encryption group, segments name the group secret
// they were sealed under (#1, #2), and stopping a reader rotates to a secret
// they never receive.
(function () {
  'use strict';
  var X = window.diaswarmExplainer;
  if (!X) return;
  var POP = X.POP;

  function P(pt, dy, dx) { return { x: pt[0] + (dx || 0), y: pt[1] + (dy || 0) }; }
  function mix(a, b) { var o = {}, k; for (k in a) o[k] = a[k]; for (k in b) o[k] = b[k]; return o; }

  var PEER_IDS = ['1c9e', '7a02', 'e4b1', '3f6d', '90ac', 'b25e'];
  var HOLDERS = [0, 3, 4];   // the top three by rendezvous score, in rank order
  var OTHERS = [1, 2, 5];
  var FETCH_FROM = 3;        // rank 2: any holder will do

  var RECORD = ['{"k":"cgm","mgdl":102.6,', ' "t":1794877119000,"trend":"FLAT"}'];

  var LAYOUTS = {
    wide: {
      w: 1100, h: 560, fs: 17, sfs: 13.5,
      big: { w: 116, h: 196 }, small: { w: 70, h: 104 }, envSmall: 0.5,
      sub: [130, 215], rdr: [970, 215],
      subLabel: [130, 345], rdrLabel: [970, 345],
      rec: [160, 50], codeW: 296,
      relay: [550, 58], relayW: 200, relayNote: [550, 104],
      peers: [[470, 185], [630, 185], [470, 318], [630, 318], [470, 451], [630, 451]],
      peersNote: [550, 548],
      log0Label: [36, 392], log1Label: [36, 456],
      log0: function (i) { return [53 + i * 42, 418]; }, blk0: 34,
      log1: function (i) { return [66 + i * 66, 482]; }, blk1: 60,
      ringLabel: [970, 392], slots: [[935, 432], [1005, 432]], slotR: 19,
      noSecret: [970, 478],
      relayPaths: function (L) {
        var t = L.big.h / 2, rx = L.relayW / 2;
        return 'M' + L.sub[0] + ' ' + (L.sub[1] - t) + ' Q' + L.sub[0] + ' ' + L.relay[1] + ' ' + (L.relay[0] - rx) + ' ' + L.relay[1] +
          ' M' + (L.relay[0] + rx) + ' ' + L.relay[1] + ' Q' + L.rdr[0] + ' ' + L.relay[1] + ' ' + L.rdr[0] + ' ' + (L.rdr[1] - t);
      },
      welcomeArc: 150, flightArc: 40
    },
    tall: {
      w: 420, h: 806, fs: 15, sfs: 12.5,
      big: { w: 100, h: 168 }, small: { w: 62, h: 92 }, envSmall: 0.44,
      sub: [105, 160], rdr: [315, 160],
      subLabel: [105, 270], rdrLabel: [315, 270],
      rec: [210, 34], codeW: 296,
      relay: [210, 452], relayW: 172, relayNote: [210, 494],
      peers: [[80, 580], [210, 580], [340, 580], [80, 712], [210, 712], [340, 712]],
      peersNote: [210, 794],
      log0Label: [14, 306], log1Label: [14, 360],
      log0: function (i) { return [29 + i * 36, 330]; }, blk0: 30,
      log1: function (i) { return [40 + i * 56, 384]; }, blk1: 52,
      ringLabel: [315, 306], slots: [[285, 344], [345, 344]], slotR: 17,
      noSecret: [315, 386],
      relayPaths: function (L) {
        var top = L.relay[1] - 23, ex = L.big.w / 2;
        return 'M' + (L.sub[0] + ex) + ' ' + (L.sub[1] + 10) + ' Q' + (L.relay[0] - 6) + ' ' + (L.sub[1] + 100) + ' ' + (L.relay[0] - 30) + ' ' + top +
          ' M' + (L.rdr[0] - ex) + ' ' + (L.rdr[1] + 10) + ' Q' + (L.relay[0] + 6) + ' ' + (L.rdr[1] + 100) + ' ' + (L.relay[0] + 30) + ' ' + top;
      },
      welcomeArc: -120, flightArc: 26
    }
  };

  function codeChip(H, lines, w, fs) {
    var g = H.el('g', { class: 'ex-code' });
    var h = lines.length * fs * 1.5 + 18;
    H.el('rect', { x: -w / 2, y: -h / 2, width: w, height: h, rx: 10 }, g);
    lines.forEach(function (ln, i) {
      H.txt(g, ln, { x: -w / 2 + 12, y: -h / 2 + 9 + fs * 1.1 + i * fs * 1.5, 'font-size': fs, 'xml:space': 'preserve' });
    });
    return g;
  }

  function welcomeToken(H) {
    var g = H.el('g', { class: 'ex-welcome' });
    H.el('rect', { x: -46, y: -15, width: 92, height: 30, rx: 15 }, g);
    H.el('circle', { cx: -29, cy: 0, r: 4.5, class: 'ex-keyglyph' }, g);
    H.el('path', { d: 'M-24.5 0 H-15 M-18 0 V4', class: 'ex-keyglyph' }, g);
    H.txt(g, 'welcome', { x: 8, y: 4.5, 'font-size': 13, 'text-anchor': 'middle' });
    return g;
  }

  function secretBadge(H, n, r) {
    var g = H.el('g', { class: 'ex-secbadge' });
    H.el('circle', { cx: 0, cy: 0, r: r, class: 'ex-sec ex-sec' + n }, g);
    H.txt(g, '#' + n, { x: 0, y: 5, 'font-size': 14, 'text-anchor': 'middle', class: 'ex-sectext' });
    return g;
  }

  function rankBadge(H, n) {
    var g = H.el('g', { class: 'ex-rank' });
    H.el('circle', { cx: 0, cy: 0, r: 12 }, g);
    H.txt(g, String(n), { x: 0, y: 4.5, 'font-size': 13, 'text-anchor': 'middle' });
    return g;
  }

  X.register('dev', {
    breakpoint: 700,
    readingMsPerChar: 26,
    layouts: LAYOUTS,

    build: function (c) {
      var svg = c.svg, L = c.L, H = c.H, dyn = c.dyn, R = c.R;

      // faint route through the relay
      H.el('path', { d: L.relayPaths(L), class: 'ex-flowline' }, svg);

      // relay
      var rg = H.at(svg, L.relay[0], L.relay[1]);
      R.relay = H.el('rect', { x: -L.relayW / 2, y: -23, width: L.relayW, height: 46, rx: 23, class: 'ex-ring relay' }, rg);
      var rb = H.el('g', { class: 'ex-relay' }, rg);
      H.el('rect', { x: -L.relayW / 2, y: -23, width: L.relayW, height: 46, rx: 23 }, rb);
      H.el('path', { d: 'M-' + (L.relayW / 2 - 22) + ' 3 h12 l4 -8 l6 14 l4 -6 h8', class: 'ex-relayglyph' }, rb);
      H.txt(rb, 'iroh relay', { x: 18, y: 5, 'font-size': L.fs - 1, 'text-anchor': 'middle' });

      R.sub = H.phone(svg, L.sub, L.big, 'you');
      R.rdr = H.phone(svg, L.rdr, L.big, 'mum');
      L.peers.forEach(function (p, i) {
        R['p' + i] = H.phone(svg, p, L.small, 'stranger');
        H.txt(svg, PEER_IDS[i] + '…', { x: p[0], y: p[1] + L.small.h / 2 + 17, 'font-size': L.sfs - 1, 'text-anchor': 'middle', class: 'ex-grp' });
      });

      H.pill(svg, L.subLabel[0], L.subLabel[1], 'Subject · AAPS', L.fs, 'you');
      H.pill(svg, L.rdrLabel[0], L.rdrLabel[1], 'Reader · Ayni', L.fs, 'mum');

      // the subject's two logs
      H.txt(svg, 'log 0 · segments', { x: L.log0Label[0], y: L.log0Label[1], 'font-size': L.sfs - 1, class: 'ex-grp' });
      H.txt(svg, 'log 1 · control', { x: L.log1Label[0], y: L.log1Label[1], 'font-size': L.sfs - 1, class: 'ex-grp' });
      [0, 1].forEach(function (i) {
        var b = H.block('#1', L.blk0, 's1');
        b.setAttribute('transform', 'translate(' + L.log0(i).join(' ') + ')');
        svg.appendChild(b);
      });
      var cr = H.block('create', L.blk1, 'ctl');
      cr.setAttribute('transform', 'translate(' + L.log1(0).join(' ') + ')');
      svg.appendChild(cr);

      // the reader's secrets
      H.txt(svg, 'secrets held', { x: L.ringLabel[0], y: L.ringLabel[1], 'font-size': L.sfs - 1, 'text-anchor': 'middle', class: 'ex-grp' });
      L.slots.forEach(function (s, i) {
        var g = H.at(svg, s[0], s[1]);
        H.el('circle', { cx: 0, cy: 0, r: L.slotR, class: 'ex-slot' }, g);
        H.txt(g, '#' + (i + 1), { x: 0, y: 5, 'font-size': 13, 'text-anchor': 'middle', class: 'ex-slottext' });
      });

      // moving parts
      dyn('subjChip', H.chip('5.7', 'mmol/L'));
      dyn('rec', codeChip(H, RECORD, L.codeW, 12.5));
      dyn('seg', H.envelope('#1'));
      dyn('segGhost', H.envelope('#1'));
      dyn('lg0n', H.block('#1', L.blk0, 's1'));
      dyn('relayHi', H.el('path', { d: L.relayPaths(L), class: 'ex-hiline' }));
      for (var k = 0; k < 4; k++) dyn('pk' + k, H.el('circle', { r: 6, class: 'ex-dot' }));
      dyn('poolNote', H.label('8 on presence → depth 3', L.sfs, 'ex-note', { mono: true }));
      dyn('lg1add', H.block('add', L.blk1, 'add'));
      dyn('welcome', welcomeToken(H));
      dyn('slot1', secretBadge(H, 1, L.slotR));
      OTHERS.forEach(function (pi, j) {
        dyn('dim' + j, H.el('rect', { x: -L.small.w / 2 - 2, y: -L.small.h / 2 - 2, width: L.small.w + 4, height: L.small.h + 4, rx: L.small.w * 0.22, class: 'ex-dim' }));
      });
      HOLDERS.forEach(function (pi, j) { dyn('rank' + j, rankBadge(H, j + 1)); });
      dyn('bucketNote', H.label('bucket 011 → top 3 by rendezvous', L.sfs, 'ex-note', { mono: true }));
      HOLDERS.forEach(function (pi, j) { dyn('cp' + j, H.envelope('#1')); });
      dyn('sleep', H.sleepOverlay(L.big));
      dyn('fetch', H.envelope('#1'));
      dyn('rchip', H.chip('5.7', '2 min ago', true));
      dyn('lg1rm', H.block('remove', L.blk1, 'rm'));
      dyn('seg2', H.envelope('#2'));
      dyn('segGhost2', H.envelope('#2'));
      dyn('lg0n2', H.block('#2', L.blk0, 's2'));
      HOLDERS.forEach(function (pi, j) { dyn('cpb' + j, H.envelope('#2')); });
      dyn('fetch2', H.envelope('#2'));
      dyn('slot2hl', H.el('circle', { r: L.slotR + 4, class: 'ex-slotdeny' }));
      dyn('noSecret', H.label('no secret #2 — can\'t open', L.sfs, 'ex-nokey'));
      dyn('leakRelay', H.label('sees node ids, IPs, timing', L.sfs, 'ex-leak', { mono: true }));
      dyn('leakPeers', H.label('hold ciphertext · see grant count & timing', L.sfs, 'ex-leak', { mono: true }));
    },

    init: function (L) {
      var SS = L.envSmall, sub = L.sub, rdr = L.rdr;
      var off = { s: 1, o: 0 };
      var I = {
        subjChip: mix(P(sub, -4), { s: 0.6, o: 0 }),
        rec: mix(P(sub, -10), { s: 0.3, o: 0 }),
        seg: mix(P(sub), { s: 0.5, o: 0 }),
        segGhost: mix(P(sub), { s: 0.9, o: 0 }),
        lg0n: mix(P(L.log0(2)), { s: 0.5, o: 0 }),
        relayHi: { x: 0, y: 0, s: 1, o: 0 },
        poolNote: mix(P(L.relayNote), off),
        lg1add: mix(P(L.log1(1)), { s: 0.5, o: 0 }),
        welcome: mix(P(sub, -20), { s: 0.8, o: 0 }),
        slot1: mix(P(L.slots[0]), { s: 0.4, o: 0 }),
        bucketNote: mix(P(L.peersNote), off),
        sleep: mix(P(sub), off),
        fetch: mix(P(L.peers[FETCH_FROM]), { s: SS, o: 0 }),
        rchip: mix(P(rdr, -30), { s: 0.6, o: 0 }),
        lg1rm: mix(P(L.log1(2)), { s: 0.5, o: 0 }),
        seg2: mix(P(sub), { s: 0.5, o: 0 }),
        segGhost2: mix(P(sub), { s: 0.9, o: 0 }),
        lg0n2: mix(P(L.log0(3)), { s: 0.5, o: 0 }),
        fetch2: mix(P(L.peers[FETCH_FROM]), { s: SS, o: 0 }),
        slot2hl: mix(P(L.slots[1]), off),
        noSecret: mix(P(L.noSecret), off),
        leakRelay: mix(P(L.relayNote), off),
        leakPeers: mix(P(L.peersNote), off)
      };
      // presence hops: subject→p0, p1→p3, p4→p5, reader→p1
      var src = [sub, L.peers[1], L.peers[4], rdr];
      for (var k = 0; k < 4; k++) I['pk' + k] = mix(P(src[k]), { s: 1, o: 0 });
      OTHERS.forEach(function (pi, j) { I['dim' + j] = mix(P(L.peers[pi]), off); });
      HOLDERS.forEach(function (pi, j) {
        var p = L.peers[pi];
        I['rank' + j] = mix(P(p, -L.small.h / 2 + 2, L.small.w / 2 - 2), { s: 0.4, o: 0 });
        I['cp' + j] = mix(P(sub), { s: 0.9, o: 0 });
        I['cpb' + j] = mix(P(sub), { s: 0.9, o: 0 });
      });
      return I;
    },

    steps: function (L) {
      var SS = L.envSmall, sub = L.sub, rdr = L.rdr, arc = L.flightArc;
      var dst = [L.peers[0], L.peers[3], L.peers[5], L.peers[1]];

      function toHolders(prefix, extra) {
        var o = {};
        HOLDERS.forEach(function (pi, j) { o[prefix + j] = mix(mix(P(L.peers[pi]), { s: SS, o: 1, arc: arc, dl: j * 140 }), extra || {}); });
        return o;
      }
      function holderPulses(d) { return HOLDERS.map(function (pi, j) { return ['p' + pi, d * 0.85 + j * 140]; }); }
      function each(prefix, n, v) { var o = {}; for (var i = 0; i < n; i++) o[prefix + i] = mix(v, {}); return o; }

      return [
        // 1 record
        [
          { d: 600, set: { subjChip: { o: 1, s: 1, ease: POP } } },
          { d: 900, set: { rec: mix(P(L.rec), { s: 1, o: 1, arc: 20 }) } }
        ],
        // 2 seal
        [
          { d: 700, set: { rec: mix(P(sub, -10), { s: 0.3, o: 0, arc: 20 }) }, gap: -250 },
          { d: 400, set: { subjChip: { o: 0, s: 0.4 } }, gap: -100 },
          { d: 650, set: { seg: { o: 1, s: 1, ease: POP } } }
        ],
        // 3 append to log 0
        [
          { d: 850, set: { segGhost: mix(P(L.log0(2)), { s: 0.35, o: 1, arc: 20 }) }, gap: 0 },
          { d: 380, set: { segGhost: { o: 0 }, lg0n: { o: 1, s: 1, ease: POP } } }
        ],
        // 4 discovery
        [
          { d: 600, set: { relayHi: { o: 1 } }, fx: [['relay', 0]] },
          { d: 900, set: (function () {
              var o = {};
              for (var k = 0; k < 4; k++) o['pk' + k] = mix(P(dst[k]), { s: 1, o: 1, arc: 18, dl: k * 160 });
              return o;
            })(), fx: dst.map(function (p, k) { return ['p' + [0, 3, 5, 1][k], 780 + k * 160]; }), gap: 100 },
          { d: 400, set: mix(each('pk', 4, { o: 0 }), { poolNote: { o: 1 } }) }
        ],
        // 5 grant
        [
          { d: 550, set: { lg1add: { o: 1, s: 1, ease: POP } } },
          { d: 1400, set: { welcome: mix(P(rdr, -20), { s: 1, o: 1, arc: L.welcomeArc }) } },
          { d: 350, set: { welcome: { o: 0, s: 0.6 } }, gap: -150 },
          { d: 600, set: { slot1: { o: 1, s: 1, ease: POP } }, fx: [['rdr', 0]] }
        ],
        // 6 buckets
        [
          { d: 500, set: each('dim', 3, { o: 1 }) },
          { d: 600, set: { rank0: { o: 1, s: 1, ease: POP }, rank1: { o: 1, s: 1, ease: POP, dl: 140 }, rank2: { o: 1, s: 1, ease: POP, dl: 280 } },
            fx: HOLDERS.map(function (pi, j) { return ['p' + pi, j * 140]; }) },
          { d: 400, set: { bucketNote: { o: 1 } } }
        ],
        // 7 replicate
        [{ d: 1150, set: toHolders('cp'), fx: holderPulses(1150) }],
        // 8 subject offline, reader fetches
        [
          { d: 600, set: { sleep: { o: 1 } } },
          { d: 1300, set: { fetch: mix(P(rdr, -10), { s: 1, o: 1, arc: arc }) } }
        ],
        // 9 open
        [
          { d: 400, set: { fetch: { s: 1.3, o: 0 } }, gap: -150 },
          { d: 650, set: { rchip: { o: 1, s: 1, ease: POP } }, fx: [['rdr', 100]] }
        ],
        // 10 stop sharing → rotate
        [
          { d: 600, set: { sleep: { o: 0 }, lg1rm: { o: 1, s: 1, ease: POP } } },
          { d: 450, set: { seg: { o: 0, s: 0.5 } }, gap: -200 },
          { d: 650, set: { seg2: { o: 1, s: 1, ease: POP } } },
          { d: 800, set: { segGhost2: mix(P(L.log0(3)), { s: 0.35, o: 1, arc: 20 }) }, gap: 0 },
          { d: 350, set: { segGhost2: { o: 0 }, lg0n2: { o: 1, s: 1, ease: POP } } },
          { d: 600, set: (function () {
              var o = {};
              HOLDERS.forEach(function (pi, j) { o['cp' + j] = mix(P(L.peers[pi], -L.small.h * 0.26), { s: SS * 0.6, o: 0.45 }); });
              return o;
            })() },
          { d: 1100, set: toHolders('cpb'), fx: holderPulses(1100), gap: 200 },
          { d: 1300, set: { fetch2: mix(P(rdr, 46), { s: 0.7, o: 1, arc: arc }) } },
          { d: 550, set: { fetch2: { shake: true, cls: 'denied' }, noSecret: { o: 1 }, slot2hl: { o: 1 } } }
        ],
        // 11 what is still visible
        [
          { d: 450, set: { poolNote: { o: 0 }, bucketNote: { o: 0 } } },
          { d: 600, set: { leakRelay: { o: 1 } }, fx: [['relay', 0]] },
          { d: 600, set: { leakPeers: { o: 1 } }, fx: HOLDERS.map(function (pi, j) { return ['p' + pi, j * 120]; }) }
        ]
      ];
    }
  });
})();
