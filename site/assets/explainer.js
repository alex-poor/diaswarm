// Animation engine for the explainers on index.html and developers.html.
//
// Plain JS + inline SVG, no build step, like the rest of site/. This file holds
// the motion, the controls and the shared drawing helpers. What is drawn and
// what moves is a *scene*, registered by name from another file:
//
//   scene-simple.js  — the brochure's seven steps     (index.html)
//   scene-dev.js     — the technical version          (developers.html)
//
// A figure opts in with <figure class="ex" data-scene="simple">. Its step text
// is the <ol class="ex-steps"> inside it, which is also what a visitor without
// JavaScript reads. Change the words there, not in the scenes.
//
// Wording follows docs/feasibility.md §11: "stop sharing", never "revoke", and
// what a reader already holds, they keep.
//
// Model: each step is a list of phases; each phase is a patch of target states
// for named elements ({x, y, s, o} plus k = key-tag visibility, t = text,
// cls = data-state). Showing step N statically applies every phase of steps
// 0..N instantly; playing step N renders N-1 statically and animates N's phases
// in order. So any step can be jumped to, and the picture is the same however
// you got there.
(function () {
  'use strict';
  var NS = 'http://www.w3.org/2000/svg';
  var EASE = 'cubic-bezier(.45,0,.2,1)';
  var POP = 'cubic-bezier(.34,1.56,.64,1)';
  var scenes = {};
  var nextUid = 0;

  window.diaswarmExplainer = {
    POP: POP,
    register: function (name, scene) { scenes[name] = scene; }
  };

  function mix(a, b) { var o = {}, k; for (k in a) o[k] = a[k]; for (k in b) o[k] = b[k]; return o; }

  // ---------------------------------------------------------------- drawing helpers
  function helpers(uid) {
    function el(tag, attrs, parent) {
      var n = document.createElementNS(NS, tag);
      for (var k in attrs) n.setAttribute(k, attrs[k]);
      if (parent) parent.appendChild(n);
      return n;
    }
    function txt(parent, str, attrs) { var t = el('text', attrs, parent); t.textContent = str; return t; }
    function textWidth(str, fs, mono) { return str.length * fs * (mono ? 0.6 : 0.55); }

    var H = {
      el: el, txt: txt, textWidth: textWidth, uid: uid,
      at: function (parent, x, y) { return el('g', { transform: 'translate(' + x + ' ' + y + ')' }, parent); },

      defs: function (svg) {
        var defs = el('defs', {}, svg);
        var f = el('filter', { id: uid + '-shadow', x: '-40%', y: '-40%', width: '180%', height: '200%' }, defs);
        el('feDropShadow', { dx: 0, dy: 5, stdDeviation: 5, 'flood-color': '#000', 'flood-opacity': 0.45 }, f);
        var gr = el('linearGradient', { id: uid + '-env', x1: 0, y1: 0, x2: 0, y2: 1 }, defs);
        el('stop', { offset: 0, 'stop-color': '#ffffff' }, gr);
        el('stop', { offset: 1, 'stop-color': '#e2e6df' }, gr);
        var sg = el('linearGradient', { id: uid + '-screen', x1: 0, y1: 0, x2: 0, y2: 1 }, defs);
        el('stop', { offset: 0, 'stop-color': '#121920' }, sg);
        el('stop', { offset: 1, 'stop-color': '#080b0e' }, sg);
      },

      pill: function (parent, x, y, str, fs, cls) {
        var g = el('g', { transform: 'translate(' + x + ' ' + y + ')', class: 'ex-pill ' + (cls || '') }, parent);
        var h = fs * 1.9, w = textWidth(str, fs) + fs * 1.6;
        el('rect', { x: -w / 2, y: -h / 2, width: w, height: h, rx: h / 2 }, g);
        txt(g, str, { x: 0, y: fs * 0.36, 'font-size': fs, 'text-anchor': 'middle' });
        return g;
      },

      // returns the pulse ring
      phone: function (parent, c, d, kind) {
        var g = el('g', { transform: 'translate(' + c[0] + ' ' + c[1] + ')' }, parent);
        var ring = el('rect', { x: -d.w / 2, y: -d.h / 2, width: d.w, height: d.h, rx: d.w * 0.2, class: 'ex-ring ' + kind }, g);
        el('rect', { x: -d.w / 2, y: -d.h / 2, width: d.w, height: d.h, rx: d.w * 0.2, class: 'ex-phone ' + kind }, g);
        el('rect', { x: -d.w / 2 + 6, y: -d.h / 2 + 6, width: d.w - 12, height: d.h - 12, rx: d.w * 0.15, class: 'ex-screen', fill: 'url(#' + uid + '-screen)' }, g);
        el('rect', { x: -d.w * 0.14, y: -d.h / 2 + 12, width: d.w * 0.28, height: 5, rx: 2.5, class: 'ex-notch' }, g);
        return ring;
      },

      // tag: 'key' (green key), 'nokey' (dashed, slashed), or a secret label like '#1'
      envelope: function (tag) {
        var g = el('g');
        var inn = el('g', { class: 'ex-in' }, g);
        el('rect', { x: -34, y: -23, width: 68, height: 46, rx: 8, class: 'ex-env', fill: 'url(#' + uid + '-env)', filter: 'url(#' + uid + '-shadow)' }, inn);
        el('path', { d: 'M-29 -18 L0 3 L29 -18', class: 'ex-flap' }, inn);
        el('path', { d: 'M-5.5 5 v-4 a5.5 5.5 0 0 1 11 0 v4', class: 'ex-shackle' }, inn);
        el('rect', { x: -9, y: 4, width: 18, height: 14, rx: 3.5, class: 'ex-lockbody' }, inn);
        var t = el('g', { 'data-k': '', class: 'ex-tag' }, inn);
        if (tag === 'key') {
          el('circle', { cx: 32, cy: -22, r: 14, class: 'ex-keytag' }, t);
          el('circle', { cx: 27, cy: -22, r: 4, class: 'ex-keyglyph' }, t);
          el('path', { d: 'M31 -22 H40 M37 -22 V-18', class: 'ex-keyglyph' }, t);
        } else if (tag === 'nokey') {
          el('circle', { cx: 32, cy: -22, r: 13, class: 'ex-noslot' }, t);
          el('path', { d: 'M27 -27 L37 -17', class: 'ex-noslash' }, t);
        } else {
          el('circle', { cx: 32, cy: -22, r: 14, class: 'ex-sec ex-sec' + tag.replace('#', '') }, t);
          txt(t, tag, { x: 32, y: -17.5, 'font-size': 13, 'text-anchor': 'middle', class: 'ex-sectext' });
        }
        return g;
      },

      chip: function (val, sub, mum) {
        var g = el('g', { class: 'ex-chip' + (mum ? ' mum' : '') });
        txt(g, val, { x: 0, y: 4, 'font-size': 40, 'text-anchor': 'middle', class: 'ex-val' });
        if (mum) {
          el('rect', { x: -43, y: 13, width: 86, height: 21, rx: 10.5, class: 'ex-agepill' }, g);
          txt(g, sub, { x: 0, y: 28, 'font-size': 12, 'text-anchor': 'middle', class: 'ex-sub', 'data-t': '' });
        } else {
          txt(g, sub, { x: 0, y: 25, 'font-size': 12, 'text-anchor': 'middle', class: 'ex-sub', 'data-t': '' });
        }
        return g;
      },

      lockIcon: function (parent, x, y, s) {
        var g = el('g', { transform: 'translate(' + x + ' ' + y + ') scale(' + s + ')', class: 'ex-minilock' }, parent);
        el('path', { d: 'M-4 0 v-3 a4 4 0 0 1 8 0 v3', fill: 'none' }, g);
        el('rect', { x: -6, y: -0.5, width: 12, height: 9, rx: 2 }, g);
        return g;
      },

      // a line of small text, optionally with a lock in front; centred unless anchor is 'start'
      label: function (str, fs, cls, opts) {
        opts = opts || {};
        var g = el('g', { class: cls || '' });
        var inner = el('g', {}, g);
        var off = opts.lock ? 16 : 0;
        if (opts.lock) H.lockIcon(inner, 6, -2, fs / 14);
        var t = txt(inner, str, { x: off, y: fs * 0.36, 'font-size': fs, 'data-t': '' });
        if (opts.anchor !== 'start') {
          if (opts.lock) inner.setAttribute('transform', 'translate(' + (-(off + textWidth(str, fs, opts.mono)) / 2) + ' 0)');
          else t.setAttribute('text-anchor', 'middle');
        }
        return g;
      },

      sleepOverlay: function (d) {
        var g = el('g', { class: 'ex-sleep' });
        el('rect', { x: -d.w / 2 + 6, y: -d.h / 2 + 6, width: d.w - 12, height: d.h - 12, rx: d.w * 0.15 }, g);
        el('path', { d: 'M4 -26 a14 14 0 1 0 12 20 a11 11 0 0 1 -12 -20 z', class: 'ex-moon' }, g);
        txt(g, 'asleep', { x: 0, y: 22, 'font-size': 14, 'text-anchor': 'middle' });
        return g;
      },

      // a log entry: a small rounded block with a label
      block: function (str, w, cls) {
        var g = el('g', { class: 'ex-blk ' + (cls || '') });
        el('rect', { x: -w / 2, y: -12, width: w, height: 24, rx: 6 }, g);
        txt(g, str, { x: 0, y: 4.5, 'font-size': 12, 'text-anchor': 'middle' });
        return g;
      }
    };
    return H;
  }

  // ---------------------------------------------------------------- one figure
  function mount(fig) {
    var scene = scenes[fig.getAttribute('data-scene')];
    if (!scene || !window.SVGElement) return;

    var reduce = window.matchMedia('(prefers-reduced-motion: reduce)');
    var stage = fig.querySelector('.ex-stage');
    var svg = stage.querySelector('svg');
    var cap = fig.querySelector('.ex-cap');
    var capCount = cap.querySelector('.ex-count');
    var capTitle = cap.querySelector('h3');
    var capBody = cap.querySelector('p');
    var specBox = fig.querySelector('.ex-specbox');
    var dotsEl = fig.querySelector('.ex-dots');
    var btnPrev = fig.querySelector('[data-ex=prev]');
    var btnNext = fig.querySelector('[data-ex=next]');
    var btnPlay = fig.querySelector('[data-ex=play]');
    var uid = 'ex' + (nextUid++);

    var TEXT = Array.prototype.map.call(fig.querySelectorAll('.ex-steps > li'), function (li) {
      var spec = li.querySelector('.ex-spec');
      return {
        title: li.querySelector('h3').textContent.trim(),
        body: li.querySelector('p').textContent.trim(),
        spec: spec ? spec.outerHTML : ''
      };
    });
    var N = TEXT.length;

    var L, mode, D, R, S, INIT, STEPS;

    function build(m) {
      mode = m;
      L = scene.layouts[m];
      while (svg.firstChild) svg.removeChild(svg.firstChild);
      svg.setAttribute('viewBox', '0 0 ' + L.w + ' ' + L.h);
      svg.setAttribute('class', m);
      var H = helpers(uid);
      H.defs(svg);
      D = {};
      R = {};
      scene.build({ svg: svg, L: L, H: H, mode: m, D: D, R: R,
        dyn: function (id, node) { node.classList.add('ex-dyn'); svg.appendChild(node); D[id] = node; return node; } });
      INIT = scene.init(L, m);
      STEPS = scene.steps(L, m);
      if (STEPS.length !== N) throw new Error('explainer: ' + STEPS.length + ' steps animated, ' + N + ' described');
    }

    // ------------------------------------------------ rendering
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

    function apply(set, dur) {
      var maxDl = 0;
      for (var id in set) {
        if (!D[id]) throw new Error('explainer: no element ' + id);
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
              kf.push({
                transform: T({
                  x: u * u * from.x + 2 * u * t * cx + t * t * st.x,
                  y: u * u * from.y + 2 * u * t * cy + t * t * st.y,
                  s: from.s + (st.s - from.s) * t + Math.sin(Math.PI * t) * 0.1
                }),
                opacity: from.o + (st.o - from.o) * Math.min(1, t / 0.2)
              });
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
          var target = n.querySelector('.ex-in') || n.firstChild;
          live.push(target.animate(
            [0, -8, 7, -5, 3, 0].map(function (v) { return { transform: 'translateX(' + v + 'px)' }; }),
            { duration: dur, delay: dl, easing: 'ease-out' }));
        }
      }
      return maxDl;
    }

    function pulse(ring, delay) {
      if (!R[ring] || !R[ring].animate) return null;
      return R[ring].animate(
        [{ opacity: 0.85, transform: 'scale(1)' }, { opacity: 0, transform: 'scale(1.16)' }],
        { duration: 900, delay: delay, easing: 'ease-out' });
    }

    // ------------------------------------------------ sequencing
    var cur = 0, token = 0, timer = 0, done = true, started = false, inView = false;
    var auto = !reduce.matches;

    function sleep(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
    function running() { return auto && inView && !document.hidden; }

    function caption(i) {
      capCount.textContent = 'Step ' + (i + 1) + ' of ' + N;
      capTitle.textContent = TEXT[i].title;
      capBody.textContent = TEXT[i].body;
      if (specBox) specBox.innerHTML = TEXT[i].spec;
      Array.prototype.forEach.call(dotsEl.children, function (li, j) {
        var b = li.firstChild;
        b.classList.toggle('past', j < i);
        if (j === i) b.setAttribute('aria-current', 'step'); else b.removeAttribute('aria-current');
      });
      if (!reduce.matches && cap.animate) {
        [cap, specBox].forEach(function (n) {
          if (n) n.animate([{ opacity: 0, transform: 'translateY(6px)' }, { opacity: 1, transform: 'none' }],
            { duration: 380, easing: 'ease-out' });
        });
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

    var perChar = scene.readingMsPerChar || 24;
    function hold(i) { return (i === N - 1 ? 6500 : 1800) + (TEXT[i].body.length + TEXT[i].spec.replace(/<[^>]+>/g, '').length * 0.4) * perChar; }

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

    // ------------------------------------------------ wiring
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

    var breakpoint = scene.breakpoint || 640;
    function pickMode() { return stage.clientWidth < breakpoint ? 'tall' : 'wide'; }

    build(pickMode());
    staticTo(0);
    caption(0);
    setAuto(auto);
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
      }, { threshold: 0.35 }).observe(stage);
    } else {
      inView = true;
      if (auto) play(0);
    }
    document.addEventListener('visibilitychange', schedule);
  }

  function start() { Array.prototype.forEach.call(document.querySelectorAll('.ex[data-scene]'), mount); }
  // scenes load after this file (all deferred), so wait for the document
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start);
  else setTimeout(start, 0);
})();
