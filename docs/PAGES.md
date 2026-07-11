# GitHub Pages site

Static landing page for [ZeroCast](https://github.com/YanChao1999/ZeroCast), served from this `docs/` folder.

**Live site:** https://yanchao1999.github.io/ZeroCast/

## Local preview

```bash
python3 -m http.server 8080 --directory docs
```

Open http://localhost:8080

## Deployment

Deploys automatically via GitHub Actions when changes under `docs/` are pushed to `main` (`.github/workflows/pages.yml`).

Repo setup (done):

- **Settings → Pages → Source → GitHub Actions**
- Workflow publishes the `docs/` folder on each qualifying push to `main`

## Site files

```
docs/
├── index.html      # Landing page
├── css/style.css
├── js/main.js
└── PAGES.md        # This file
```

Phase and protocol markdown in this folder is also published alongside the landing page.

## Editing the site

1. Change files under `docs/` (`index.html`, `css/`, `js/`)
2. Push to `main`
3. Check the **Deploy GitHub Pages** workflow in Actions
