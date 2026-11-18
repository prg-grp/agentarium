# Agentarium product site

A static, single-page website about Agentarium, deployed to GitHub Pages at `https://prg-grp.github.io/agentarium/` by `.github/workflows/pages.yml` on every push to `main` that touches `pages/`.

There is no build step. Everything in this folder is published as is.

```
pages/
  index.html          the site
  404.html            served by GitHub Pages for missing paths (absolute /agentarium/ asset URLs)
  assets/
    site.css          all styles
    site.js           note placement beside the app picture, GSAP animations
    vendor/           GSAP 3.13.0 and ScrollTrigger, unmodified (standard GSAP license)
    fonts/            Rubik and VT323, latin subsets (SIL Open Font License)
    app-icon.svg/png  copied from app/ui/
    shots/            app screenshots (WebP), full windows and crops, and the
                      create-and-start recording (GIF plus a WebP poster frame)
```

## Preview locally

```bash
cd pages && python3 -m http.server 8000
```

Then open `http://localhost:8000`. The 404 page only looks right when served under `/agentarium/`.

## Design rules

The style is "Memphis desktop": 90s Memphis print with 90s desktop windows. The app itself always appears in its own dark design.

- **Pastel is the room, neon is the signal.** Section grounds are pastels (lilac hero, mint features, sky steps, peach extras, butter records) and carry no meaning. Neon marks state or the forward action only: cyan = go and focus, green = allowed or running, pink = blocked or failed, orange = maintenance or writable. Each neon is the loud twin of an app colour (acc, ok, bad, warn).
- **Neon is a fill, never text.** Text on neon is black.
- **Everything has a black outline.** 2px outlines, hard 6px/4px offset shadows, no blur, no gradients, no rounded corners on the site.
- **Windows hold content.** Feature, step and info boxes are 90s windows with a title bar and decorative controls. Title bars name what's inside in plain words.
- **Frame the app, never restyle it.** Screenshots sit on a white (hero) or lilac (features) mat inside a black outline. No tint, overlay or extra title bar; the app brings its own window. Each picture links to its full-size file and has alt text describing what it shows.
- **Screenshots:** capture with ⌘⇧4, Space, Option-click (no macOS shadow), window about 1280 × 800 points, demo data only. Store as WebP in `assets/shots/`; crops come from the full window and are shown at actual size where possible, so 1× text stays readable.
- **Recordings:** convert screen recordings to GIF cropped to the app window (10 fps, 128-colour palette from `palettegen`, `paletteuse` with `diff_mode=rectangle`) and keep a WebP poster frame. The page shows the poster; the GIF plays only while on screen and only without reduced motion, and a Play/Pause button always controls it.
- **Notes go beside the app, not on it.** Memphis notes sit on the mat next to the hero screenshot. Each `.anno` has `data-y`, the spot it explains as a fraction of the image height; `site.js` stacks the notes and draws elbow leaders to those spots. Update the fractions when the screenshot changes.
- **Checkerboard dither means "not observed"** and nothing else.
- **Type:** Rubik for words (900 hero with cyan and pink offsets, 800 headings), VT323 only for data such as commands and BibTeX.
- **Motion:** GSAP, only inside `prefers-reduced-motion: no-preference`. Without JavaScript or with reduced motion everything is visible.

## Copy

Claims on the site must match what the app does today (check `ARCHITECTURE.md`, `design/` and the code). Use the app's own words: instance, maintenance, Read only, Read and write, Blocked. Don't show monitoring-gap rows, export, a live activity rail or Intel support until the app has them. Notes and captions must describe what the screenshot actually shows.
