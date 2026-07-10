# ZeroCast GitHub Pages site

Static landing page for [ZeroCast](https://github.com/YanChao1999/ZeroCast).

## Local preview

```bash
# Python
python3 -m http.server 8080 --directory website

# Or with any static file server
npx serve website
```

Open http://localhost:8080

## Deployment

This site deploys automatically via GitHub Actions when changes are pushed to the `main` branch (`.github/workflows/pages.yml`).

**One-time setup in the GitHub repo:**

1. Go to **Settings → Pages**
2. Under **Build and deployment**, set **Source** to **GitHub Actions** (not “Deploy from a branch”)
3. Merge this PR into `main` — the deploy workflow runs automatically on push

The site will be available at:

**https://yanchao1999.github.io/ZeroCast/**

## Structure

```
website/
├── index.html      # Landing page
├── css/style.css   # Styles
├── js/main.js      # Copy buttons, mobile nav
└── README.md       # This file
```
