// Agentarium product site: note placement beside the app picture, and GSAP animations.
// Without JavaScript, or with reduced motion, everything is visible and static.
(function () {
  const frame = document.getElementById('frame');

  // Place each note level with the spot of the screenshot it explains (data-y is a fraction of
  // the image height), never overlapping, and draw an elbow leader from the image's right edge to
  // the note. Below 1100px the notes stack under the screenshot without leaders.
  function placeNotes() {
    if (!frame) return;
    const svg = frame.querySelector('.leaders');
    const app = frame.querySelector('.shot img');
    const box = frame.querySelector('.notes');
    const notes = Array.from(box.querySelectorAll('.anno[data-y]'));
    svg.innerHTML = '';
    if (!window.matchMedia('(min-width: 1101px)').matches) {
      notes.forEach(function (n) { n.style.top = ''; });
      box.style.height = '';
      return;
    }
    const f = frame.getBoundingClientRect(), nb = box.getBoundingClientRect(), a = app.getBoundingClientRect();
    const items = notes.map(function (n) {
      return { n: n, rowY: a.top + a.height * Number(n.dataset.y) - nb.top };
    }).sort(function (p, q) { return p.rowY - q.rowY; });
    let floor = 0, paths = '';
    items.forEach(function (it) {
      const h = it.n.offsetHeight;
      const top = Math.max(floor, Math.round(it.rowY - h / 2));
      it.n.style.top = top + 'px';
      floor = top + h + 14;
      const x0 = a.right - f.left, x2 = nb.left - f.left, xm = (x0 + x2) / 2;
      const y0 = it.rowY + nb.top - f.top, y1 = top + h / 2 + nb.top - f.top;
      paths += '<path pathLength="1" d="M' + x0 + ' ' + y0 + ' H' + xm + ' V' + y1 + ' H' + x2 + '"/>' +
               '<circle cx="' + x0 + '" cy="' + y0 + '" r="4"/>';
    });
    box.style.height = Math.max(0, floor - 14) + 'px';
    svg.innerHTML = paths;
  }
  window.addEventListener('resize', placeNotes);
  window.addEventListener('load', placeNotes);
  if (document.fonts) document.fonts.ready.then(placeNotes);
  placeNotes();

  // The recording shows a still poster until played. Play and Pause swap the GIF in and out,
  // which also restarts it from the beginning.
  const rec = document.getElementById('recording');
  let recPlay = null;
  if (rec) {
    const img = rec.querySelector('img'), btn = rec.querySelector('.rec-btn');
    recPlay = function (on) {
      img.src = on ? img.dataset.gif : img.dataset.poster;
      btn.textContent = on ? 'Pause recording' : 'Play recording';
      btn.setAttribute('aria-pressed', String(on));
    };
    btn.hidden = false;
    btn.addEventListener('click', function () { recPlay(btn.getAttribute('aria-pressed') !== 'true'); });
    rec.querySelector('.rec').addEventListener('click', function (e) { e.preventDefault(); btn.click(); });
  }

  if (!window.gsap || !window.ScrollTrigger) return;
  gsap.registerPlugin(ScrollTrigger);
  const mm = gsap.matchMedia();

  mm.add('(prefers-reduced-motion: no-preference)', function () {
    // Hero: the headline drops in, then its cyan and pink offsets slide into register.
    const h1 = document.querySelector('.hero h1');
    const shadow = getComputedStyle(h1).textShadow;
    const flat = shadow.replace(/(-?[\d.]+)px (-?[\d.]+)px/g, '0px 0px');
    gsap.timeline()
      .from('.hero h1 span', { yPercent: 35, opacity: 0, duration: 0.55, stagger: 0.12, ease: 'back.out(1.6)' })
      .fromTo(h1, { textShadow: flat }, { textShadow: shadow, duration: 0.5, ease: 'back.out(3)', clearProps: 'textShadow' }, '-=0.15')
      .from('.hero .lead, .hero .actions, .hero .note, .hero .presets', { y: 14, opacity: 0, duration: 0.35, stagger: 0.07 }, '-=0.3');

    // Confetti drifts, and the hero's confetti layer lags behind on scroll.
    gsap.utils.toArray('.confetti svg').forEach(function (el, i) {
      gsap.to(el, { y: i % 2 ? 14 : -14, rotation: i % 2 ? 9 : -9, duration: 2.6 + i * 0.45, ease: 'sine.inOut', yoyo: true, repeat: -1 });
    });
    gsap.to('.hero .confetti', { yPercent: 30, ease: 'none', scrollTrigger: { trigger: '.hero', start: 'top top', end: 'bottom top', scrub: true } });

    // Screenshot: drawn open top to bottom in hard steps, then leaders draw and notes pop in.
    if (frame) {
      const notes = frame.querySelectorAll('.anno');
      gsap.set(notes, { opacity: 0, scale: 0.7 });
      const tl = gsap.timeline({ scrollTrigger: { trigger: frame, start: 'top 80%', once: true } });
      tl.fromTo(frame.querySelector('.shot'), { clipPath: 'inset(0% 0% 100% 0%)' }, { clipPath: 'inset(0% 0% 0% 0%)', duration: 0.6, ease: 'steps(8)', clearProps: 'clipPath' });
      tl.call(function () {
        placeNotes();
        gsap.fromTo(frame.querySelectorAll('.leaders path'), { strokeDasharray: 1, strokeDashoffset: 1 }, { strokeDashoffset: 0, duration: 0.45, stagger: 0.15, ease: 'power1.inOut' });
      });
      tl.to(notes, { opacity: 1, scale: 1, duration: 0.35, stagger: 0.15, ease: 'back.out(2.2)' }, '+=0.1');
    }

    // Windows open like a 90s window: drawn top to bottom in a few hard steps.
    gsap.utils.toArray('.feats, .grid3, .grid2, .steps, .start-grid').forEach(function (group) {
      gsap.fromTo(group.querySelectorAll(':scope > .win'),
        { clipPath: 'inset(0% -12px 100% -2px)' },
        { clipPath: 'inset(0% -12px -12px -2px)', duration: 0.45, ease: 'steps(7)', stagger: 0.12, clearProps: 'clipPath',
          scrollTrigger: { trigger: group, start: 'top 82%', once: true } });
    });

    // The recording plays while it's on screen and goes back to its poster when it leaves.
    if (recPlay) {
      ScrollTrigger.create({ trigger: rec, start: 'top 70%', end: 'bottom 20%',
        onEnter: function () { recPlay(true); }, onEnterBack: function () { recPlay(true); },
        onLeave: function () { recPlay(false); }, onLeaveBack: function () { recPlay(false); } });
    }
  });
})();
