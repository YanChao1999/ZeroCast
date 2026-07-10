# GitHub Pages site

Static landing page for [ZeroCast](https://github.com/YanChao1999/ZeroCast), served from this `docs/` folder.

## Local preview

```bash
python3 -m http.server 8080 --directory docs
```

Open http://localhost:8080

## Deployment

Deploys automatically via GitHub Actions when changes under `docs/` are pushed to `main` (`.github/workflows/pages.yml`).

**One-time setup:**

1. **Settings → Pages → Build and deployment → Source → GitHub Actions**
2. Merge into `main` — the workflow publishes the `docs/` folder

Site URL: **https://yanchao1999.github.io/ZeroCast/**

## Site files

```
docs/
├── index.html      # Landing page
├── css/style.css
├── js/main.js
└── PAGES.md        # This file
```

Phase and protocol docs in this folder are also published alongside the landing page.
